#![no_std]

#[cfg(test)]
mod test_groth16;
mod poseidon;

use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    contract, contractimpl, contracttype, symbol_short, token,
    xdr::ToXdr,
    Address, Bytes, BytesN, Env, IntoVal, Vec,
};
use zkella_verifier_interface::{CircuitType, VerifierClient};
use zkella_token_interface::{
    TokenClient, ShieldPublicInputs as TokenShieldPublicInputs,
    UnshieldPublicInputs as TokenUnshieldPublicInputs,
};

/// Ledgers after `expiry_ledger` a relayer who already fronted liquidity at
/// `execute_swap` has to wait before reclaiming it via
/// `reclaim_expired_swap` if the claimant never calls `reveal_and_claim`.
/// ~1 day at 5s/ledger — enough that a slow-but-honest claimant isn't raced
/// by an impatient relayer, short enough that fronted capital isn't locked
/// indefinitely.
const CLAIM_WINDOW_LEDGERS: u32 = 17_280;

#[contracttype]
pub enum StorageKey {
    SwapState(BytesN<32>),
    ApprovedRelayer(Address),
    Admin,
    Verifier,
    Token,
}

/// Extract the raw 32-byte contract ID from a Soroban Address via XDR.
/// Identical to (and must stay in sync with) `token::address_to_field_bytes` —
/// duplicated rather than shared because `swap` doesn't otherwise depend on
/// `token`. See that function's doc comment for why this reads the *last* 32
/// bytes of the XDR rather than a fixed forward offset.
fn address_to_field_bytes(env: &Env, addr: &Address) -> [u8; 32] {
    let xdr = addr.to_xdr(env);
    let mut out = [0u8; 32];
    let start = xdr.len() - 32;
    for i in 0..32u32 {
        out[i as usize] = xdr.get(start + i).unwrap_or(0) as u8;
    }
    out
}

#[contracttype]
#[derive(Clone, PartialEq)]
pub enum SwapStatus {
    Committed,
    Executed,
    Claimed,
    Cancelled,
}

#[contracttype]
#[derive(Clone)]
pub struct SwapState {
    pub intent_commitment: BytesN<32>,
    pub nullifier_in:      BytesN<32>,
    pub expiry_ledger:     u32,
    pub status:            SwapStatus,
    pub amount_in:         i128,
    pub amount_out:        i128,
    pub asset_in:          Address,
    pub asset_out:         Address,
    /// Public Stellar address `asset_in` is refunded to if this swap is
    /// cancelled or reclaimed unclaimed. Necessarily public (unlike the
    /// happy path's shielded output) — recovering escrowed value without
    /// the original note's secrets requires a real, addressable destination.
    pub refund_to:         Address,
    /// Set at `execute_swap` time; the relayer who fronted `asset_out`
    /// liquidity and is owed `asset_in` once the claimant reveals a valid
    /// fairness proof (or who can reclaim their `asset_out` back after
    /// `CLAIM_WINDOW_LEDGERS` if the claimant never does).
    pub relayer:           Option<Address>,
}

#[contracttype]
pub struct SwapFairnessPublicInputs {
    pub intent_commitment: BytesN<32>,
    pub asset_in:          Address,
    pub asset_out:         Address,
    pub amount_out:        i128,
    pub min_amount_out:    i128,
}

#[contract]
pub struct ShieldedSwap;

#[contractimpl]
impl ShieldedSwap {

    pub fn initialize(env: Env, admin: Address, verifier: Address, token_contract: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage().instance().set(&StorageKey::Verifier, &verifier);
        env.storage().instance().set(&StorageKey::Token, &token_contract);
    }

    /// Commits to a swap intent for `nullifier_in`, escrowing `amount_in` of
    /// `asset_in` into this contract's balance right now.
    ///
    /// `ownership_proof` is a **real Groth16 proof — a genuine
    /// `unshield.circom` proof**, not a stub: this cross-calls `token`'s own
    /// `unshield(nullifier_in, this_contract, ownership_proof, ...)`,
    /// reusing token's already-real, already-audited unshield verification
    /// as the swap's note-ownership proof. That call both proves the
    /// committer owns a real, unspent note worth `amount_in` of `asset_in`
    /// *and* atomically pulls that value into this contract's own balance
    /// (marking the note's nullifier spent on token's side) — there's no
    /// separate ownership circuit to build or maintain.
    pub fn commit_swap(
        env:               Env,
        nullifier_in:      BytesN<32>,
        intent_commitment: BytesN<32>,
        asset_in:          Address,
        asset_out:         Address,
        amount_in:         i128,
        anchor:            BytesN<32>,
        refund_to:         Address,
        ownership_proof:   Bytes,
        expiry_ledger:     u32,
    ) -> BytesN<32> {
        assert!(expiry_ledger > env.ledger().sequence(), "expiry must be in the future");
        // `reclaim_expired_swap` computes `expiry_ledger + CLAIM_WINDOW_LEDGERS`
        // — reject anything that would overflow that addition now, rather
        // than accepting a commit whose only unwind path (if the relayer
        // fronts liquidity but the claimant never claims) panics forever,
        // permanently locking both sides' funds with no recovery route.
        assert!(
            expiry_ledger <= u32::MAX - CLAIM_WINDOW_LEDGERS,
            "expiry_ledger too close to u32::MAX to leave room for the claim window"
        );
        assert!(amount_in > 0, "amount_in must be positive");

        // `swap_id` is derived solely from `intent_commitment`; checked
        // before the real escrow below (not just before storing state) —
        // without this, a second `commit_swap` call that happens to reuse
        // the same `intent_commitment` (e.g. a non-unique `intent_nonce`)
        // would both pull a *second* real note's value into escrow *and*
        // silently overwrite the first swap's `SwapState`, orphaning that
        // first escrow with no record left to reclaim it by. Checking first
        // means a duplicate `intent_commitment` is rejected before this
        // note's nullifier is ever spent at all.
        let swap_id: BytesN<32> = env.crypto().sha256(&intent_commitment.clone().into()).into();
        assert!(
            !env.storage().instance().has(&StorageKey::SwapState(swap_id.clone())),
            "swap already committed for this intent_commitment"
        );

        let swap_addr = env.current_contract_address();

        // recipient_hash binds `to` = this contract *and*, via binding_tag,
        // this specific (intent_commitment, refund_to) pair — see
        // `token::unshield`'s doc comment for why. Without folding
        // intent_commitment/refund_to into the proof this way, the same
        // ownership_proof bytes were valid for *any* commit_swap call
        // reusing this exact nullifier/amount/asset, regardless of who
        // submitted it or what refund_to they chose — a real replay/
        // front-running path to steal the escrowed value. Because
        // binding_tag is folded into recipient_hash, and recipient_hash is
        // one of the Groth16 proof's public inputs (cryptographically bound
        // to the specific proof bytes even though the circuit itself places
        // no constraint on its value), a proof generated for one
        // intent_commitment/refund_to pair cannot be reused for another.
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let to_field = address_to_field_bytes(&env, &swap_addr);
        let intent_commitment_bytes: [u8; 32] = intent_commitment.clone().into();
        let refund_to_field = address_to_field_bytes(&env, &refund_to);
        let binding_tag_bytes = hasher.hash(&intent_commitment_bytes, &refund_to_field);
        let binding_tag = BytesN::from_array(&env, &binding_tag_bytes);
        let recipient_hash_bytes = hasher.hash(&to_field, &binding_tag_bytes);
        let recipient_hash = BytesN::from_array(&env, &recipient_hash_bytes);

        let token_contract: Address = env.storage().instance().get(&StorageKey::Token).expect("not initialized");

        // Real ownership proof + real escrow, via token's own unshield path.
        // Panics (propagating token's error) if the proof, nullifier, or
        // anchor don't check out — commit_swap simply doesn't complete, so
        // there's no partial/inconsistent state to clean up afterward.
        TokenClient::new(&env, &token_contract).unshield(
            &nullifier_in,
            &swap_addr,
            &binding_tag,
            &ownership_proof,
            &TokenUnshieldPublicInputs {
                anchor,
                nullifier: nullifier_in.clone(),
                pub_value: amount_in,
                pub_asset_id: asset_in.clone(),
                recipient_hash,
            },
        );

        let state = SwapState {
            intent_commitment,
            nullifier_in,
            expiry_ledger,
            status: SwapStatus::Committed,
            amount_in,
            amount_out: 0,
            asset_in,
            asset_out,
            refund_to,
            relayer: None,
        };
        env.storage().instance().set(&StorageKey::SwapState(swap_id.clone()), &state);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("commit")),
            (swap_id.clone(), expiry_ledger),
        );

        swap_id
    }

    /// Relayer fronts `amount_out` of `asset_out` into escrow now — a real
    /// SEP-41 transfer, not bookkeeping — in exchange for receiving the
    /// already-escrowed `asset_in` once the claimant reveals a valid
    /// fairness proof (`reveal_and_claim`), or reclaiming this deposit back
    /// (`reclaim_expired_swap`) if they never do.
    pub fn execute_swap(
        env:        Env,
        swap_id:    BytesN<32>,
        amount_out: i128,
        relayer:    Address,
    ) {
        relayer.require_auth();
        assert!(
            env.storage().instance().has(&StorageKey::ApprovedRelayer(relayer.clone())),
            "relayer not approved"
        );
        assert!(amount_out > 0, "amount_out must be positive");

        let mut state: SwapState = env.storage().instance()
            .get(&StorageKey::SwapState(swap_id.clone())).expect("swap not found");
        assert!(state.status == SwapStatus::Committed, "swap not in committed state");
        assert!(env.ledger().sequence() <= state.expiry_ledger, "swap expired");

        // Checks-effects-interactions: update state *before* the token
        // transfer (matching every other state-changing function in this
        // contract). `asset_out` is an arbitrary, participant-chosen
        // `Address` — not a pre-vetted allowlist — so nothing here assumes
        // it can't attempt a reentrant call on `transfer`; doing the effect
        // first means a reentrant `execute_swap` call sees `status ==
        // Executed` and fails the guard above, instead of being able to
        // pull a second `amount_out` from the relayer before this call's
        // own state write lands.
        state.status = SwapStatus::Executed;
        state.amount_out = amount_out;
        state.relayer = Some(relayer.clone());
        env.storage().instance().set(&StorageKey::SwapState(swap_id.clone()), &state);

        token::Client::new(&env, &state.asset_out)
            .transfer(&relayer, &env.current_contract_address(), &amount_out);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("exec")),
            (swap_id, amount_out),
        );
    }

    /// Verifies the swap's fairness proof (binds the revealed `amount_out`
    /// back to the original `intent_commitment` from `commit_swap`, and that
    /// `amount_out >= min_amount_out`, without having revealed either bound
    /// at commit time — see `circuits/swap/swap_fairness.circom`), then
    /// releases real value on both sides:
    ///   - pays the relayer the escrowed `asset_in` (their compensation for
    ///     fronting `asset_out` at `execute_swap` time)
    ///   - re-shields the escrowed `asset_out` as a fresh note for the
    ///     claimant, via token's own `shield()` — `shield_proof` is a
    ///     **separate, real Groth16 proof** (a genuine `shield.circom`
    ///     proof for the *output* note's commitment) from `fairness_proof`;
    ///     the caller needs both, since they prove different things
    ///     (fairness of the executed price vs. correctness of the new
    ///     note's own commitment).
    ///
    /// Returns the real leaf index token assigns the new note.
    #[allow(clippy::too_many_arguments)]
    pub fn reveal_and_claim(
        env:              Env,
        swap_id:          BytesN<32>,
        out_rho:          BytesN<32>,
        out_rcm:          BytesN<32>,
        out_commitment:   BytesN<32>,
        out_value_commit: BytesN<32>,
        encrypted_note:   Bytes,
        fairness_proof:   Bytes,
        fairness_pub:     SwapFairnessPublicInputs,
        shield_proof:     Bytes,
    ) -> u32 {
        let mut state: SwapState = env.storage().instance()
            .get(&StorageKey::SwapState(swap_id.clone())).expect("swap not found");
        assert!(state.status == SwapStatus::Executed, "swap not executed");
        // Without this check, `fairness_pub.intent_commitment` was accepted
        // as whatever the caller supplied, completely disconnected from
        // `state.intent_commitment` (the one actually committed to at
        // `commit_swap` time). Since `asset_in`/`asset_out`/`amount_out` are
        // all public once a swap is `Executed`, anyone could construct their
        // own unrelated, self-chosen `intent_commitment` (e.g. via
        // `max_slippage_bps = 10000` to force `min_amount_out = 0`), produce
        // a real, internally-valid fairness proof for it, and steal the
        // escrowed `asset_out` by supplying their own `out_commitment` — a
        // real fund-theft path, not just a soundness nicety.
        assert!(fairness_pub.intent_commitment == state.intent_commitment, "intent_commitment mismatch");
        assert!(fairness_pub.asset_in == state.asset_in, "asset_in mismatch");
        assert!(fairness_pub.asset_out == state.asset_out, "asset_out mismatch");
        assert!(fairness_pub.amount_out == state.amount_out, "amount_out mismatch");

        let verifier: Address = env.storage().instance().get(&StorageKey::Verifier)
            .expect("not initialized");

        let mut amount_out_bytes = [0u8; 32];
        amount_out_bytes[..16].copy_from_slice(&(fairness_pub.amount_out as u128).to_le_bytes());
        let mut min_amount_out_bytes = [0u8; 32];
        min_amount_out_bytes[..16]
            .copy_from_slice(&(fairness_pub.min_amount_out as u128).to_le_bytes());

        let public_inputs = Vec::from_array(
            &env,
            [
                fairness_pub.intent_commitment.clone(),
                BytesN::from_array(&env, &address_to_field_bytes(&env, &fairness_pub.asset_in)),
                BytesN::from_array(&env, &address_to_field_bytes(&env, &fairness_pub.asset_out)),
                BytesN::from_array(&env, &amount_out_bytes),
                BytesN::from_array(&env, &min_amount_out_bytes),
            ],
        );

        let proof_ok = VerifierClient::new(&env, &verifier).verify(
            &CircuitType::SwapFairness,
            &public_inputs,
            &fairness_proof,
        );
        assert!(proof_ok, "invalid fairness proof");

        state.status = SwapStatus::Claimed;
        env.storage().instance().set(&StorageKey::SwapState(swap_id.clone()), &state);

        // Pay the relayer with the escrowed asset_in — released only now
        // that the claimant has proven they got a fair price.
        let relayer = state.relayer.clone().expect("executed swap always has a relayer");
        token::Client::new(&env, &state.asset_in)
            .transfer(&env.current_contract_address(), &relayer, &state.amount_in);

        // Re-shield the escrowed asset_out as a new note for the claimant.
        // `from` = this contract; Soroban auto-authorizes a contract's own
        // *direct* calls, so `token::shield`'s own `from.require_auth()` is
        // satisfied for free. But `token::shield` then calls
        // `token::Client::transfer(from = this contract, to = token, amount)`
        // on `asset_out` — a call `token` makes, not `swap` — which needs
        // this contract's auth *two* levels deep in the call stack. Soroban
        // only auto-authorizes one hop, so that inner `transfer`'s
        // `require_auth()` needs an explicit entry here via
        // `authorize_as_current_contract`, describing exactly the sub-call
        // `token::shield` is about to make. Without this, the call fails on
        // real (non-mocked) auth with `Error(Auth, InvalidAction)` — the
        // blanket `mock_all_auths_allowing_non_root_auth()` used by most
        // tests in this file never exercises this (by design, per its own
        // doc comment), which is why this was originally caught only by a
        // real live-Testnet transaction. Now also covered by a dedicated
        // regression test using real (non-mocked) auth checking:
        // `reveal_and_claim_authorize_as_current_contract_satisfies_real_non_mocked_auth`.
        let token_contract: Address = env.storage().instance().get(&StorageKey::Token).expect("not initialized");
        let swap_addr = env.current_contract_address();
        env.authorize_as_current_contract(Vec::from_array(
            &env,
            [InvokerContractAuthEntry::Contract(SubContractInvocation {
                context: ContractContext {
                    contract: state.asset_out.clone(),
                    fn_name: symbol_short!("transfer"),
                    args: Vec::from_array(
                        &env,
                        [
                            swap_addr.into_val(&env),
                            token_contract.into_val(&env),
                            state.amount_out.into_val(&env),
                        ],
                    ),
                },
                sub_invocations: Vec::new(&env),
            })],
        ));
        let leaf_index = TokenClient::new(&env, &token_contract).shield(
            &env.current_contract_address(),
            &state.asset_out,
            &state.amount_out,
            &out_rho,
            &out_rcm,
            &out_commitment,
            &encrypted_note,
            &shield_proof,
            &TokenShieldPublicInputs {
                commitment: out_commitment.clone(),
                value_commit: out_value_commit,
                pub_value: state.amount_out,
                pub_asset_id: state.asset_out.clone(),
            },
        );

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("claim")),
            (swap_id, out_commitment, encrypted_note),
        );

        leaf_index
    }

    /// Cancels a swap that was never executed, refunding the escrowed
    /// `asset_in` to the `refund_to` address given at `commit_swap` time.
    /// Callable by anyone once `expiry_ledger` has passed — safe because
    /// funds only ever move to the address the original committer
    /// themselves specified, never to the caller.
    pub fn cancel_swap(env: Env, swap_id: BytesN<32>) {
        let mut state: SwapState = env.storage().instance()
            .get(&StorageKey::SwapState(swap_id.clone())).expect("swap not found");
        assert!(
            state.status == SwapStatus::Committed
            && env.ledger().sequence() > state.expiry_ledger,
            "cannot cancel"
        );
        state.status = SwapStatus::Cancelled;
        env.storage().instance().set(&StorageKey::SwapState(swap_id.clone()), &state);

        token::Client::new(&env, &state.asset_in)
            .transfer(&env.current_contract_address(), &state.refund_to, &state.amount_in);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("cancel")),
            swap_id,
        );
    }

    /// Unwinds a swap that was executed (relayer fronted `asset_out`) but
    /// never claimed, once `CLAIM_WINDOW_LEDGERS` past `expiry_ledger` has
    /// passed: returns the relayer's fronted `asset_out` deposit *and*
    /// refunds the claimant's escrowed `asset_in` to `refund_to` in the
    /// same call, so neither side's funds are left permanently stranded by
    /// the other party's inaction. Callable by anyone (typically the
    /// relayer) — like `cancel_swap`, safe because both transfers go only
    /// to addresses fixed at `commit_swap`/`execute_swap` time.
    pub fn reclaim_expired_swap(env: Env, swap_id: BytesN<32>) {
        let mut state: SwapState = env.storage().instance()
            .get(&StorageKey::SwapState(swap_id.clone())).expect("swap not found");
        assert!(state.status == SwapStatus::Executed, "swap not in executed state");
        // `checked_add` as defense-in-depth: `commit_swap` already rejects
        // any `expiry_ledger` that would overflow this addition, but a
        // plain `+` here would otherwise panic on overflow anyway in a
        // release build (`overflow-checks = true`) — using `checked_add`
        // makes that failure mode explicit rather than relying solely on
        // the earlier guard holding for every code path forever.
        let claim_deadline = state.expiry_ledger.checked_add(CLAIM_WINDOW_LEDGERS)
            .expect("expiry_ledger + claim window overflows u32");
        assert!(
            env.ledger().sequence() > claim_deadline,
            "claim window not yet expired"
        );

        state.status = SwapStatus::Cancelled;
        env.storage().instance().set(&StorageKey::SwapState(swap_id.clone()), &state);

        let relayer = state.relayer.clone().expect("executed swap always has a relayer");
        token::Client::new(&env, &state.asset_out)
            .transfer(&env.current_contract_address(), &relayer, &state.amount_out);
        token::Client::new(&env, &state.asset_in)
            .transfer(&env.current_contract_address(), &state.refund_to, &state.amount_in);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("reclaim")),
            swap_id,
        );
    }

    pub fn set_relayer(env: Env, relayer: Address, approved: bool) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        if approved {
            env.storage().instance().set(&StorageKey::ApprovedRelayer(relayer), &true);
        } else {
            env.storage().instance().remove(&StorageKey::ApprovedRelayer(relayer));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use zkella_verifier::{VerifierContract, VerifierContractClient};
    use zkella_token::{
        ShieldedToken, ShieldedTokenClient,
        ShieldPublicInputs as NativeShieldPublicInputs,
    };

    /// Note commitment, matching `token::compute_commitment` exactly:
    /// `H(H(value, asset_field), H(rho, rcm))`.
    fn note_commitment(
        env: &Env,
        hasher: &mut poseidon::Poseidon2Hasher,
        value: i128,
        asset: &Address,
        rho: &BytesN<32>,
        rcm: &BytesN<32>,
    ) -> BytesN<32> {
        let mut value_bytes = [0u8; 32];
        value_bytes[..16].copy_from_slice(&(value as u128).to_le_bytes());
        let asset_bytes = address_to_field_bytes(env, asset);
        let rho_bytes: [u8; 32] = rho.clone().into();
        let rcm_bytes: [u8; 32] = rcm.clone().into();

        let h1 = hasher.hash(&value_bytes, &asset_bytes);
        let h2 = hasher.hash(&rho_bytes, &rcm_bytes);
        BytesN::from_array(env, &hasher.hash(&h1, &h2))
    }

    fn i128_le_bytes(v: i128) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[..16].copy_from_slice(&(v as u128).to_le_bytes());
        out
    }

    #[allow(dead_code)] // `admin` is part of the deployed topology even where no test reads it back
    struct Setup {
        env:            Env,
        admin:          Address,
        relayer:        Address,
        asset_in:       Address,
        asset_out:      Address,
        verifier:       Address,
        token_contract:  Address,
        swap:           Address,
    }

    /// Deploys verifier + token + swap wired together, two real Stellar
    /// Asset Contracts for asset_in/asset_out, and a relayer approved on
    /// `swap`. Real contracts throughout — the only thing "synthetic" here
    /// is that proofs are built via `test_groth16::build_valid_groth16_proof`
    /// (a genuine Groth16 relation, just not tied to a specific circuit's
    /// constraints) rather than a real `circom`-compiled proof, exactly
    /// mirroring `zkella-token`'s own test suite's established pattern —
    /// circuit-level soundness is proven once, at the circuit level, in
    /// `zkella-verifier`'s real-circuit tests.
    fn setup() -> Setup {
        let env = Env::default();
        env.cost_estimate().budget().reset_limits(400_000_000, 41_943_040);
        // `mock_all_auths()` only covers the root call's own authorization
        // tree; `commit_swap`/`reveal_and_claim` have `swap` itself
        // authorize as `from` for nested `token::unshield`/`token::shield`
        // calls (a contract auto-authorizing its own address, per Soroban's
        // standard "contract calling as itself" rule), which is a *non-root*
        // auth in the call stack and needs this variant instead.
        env.mock_all_auths_allowing_non_root_auth();

        let admin   = Address::generate(&env);
        let relayer = Address::generate(&env);

        let asset_in_admin  = Address::generate(&env);
        let asset_out_admin = Address::generate(&env);
        let asset_in  = env.register_stellar_asset_contract_v2(asset_in_admin).address();
        let asset_out = env.register_stellar_asset_contract_v2(asset_out_admin).address();

        let verifier = env.register(VerifierContract, ());
        VerifierContractClient::new(&env, &verifier).initialize(&admin);

        let token_contract = env.register(ShieldedToken, ());
        ShieldedTokenClient::new(&env, &token_contract).initialize(&admin, &verifier);

        let swap = env.register(ShieldedSwap, ());
        ShieldedSwapClient::new(&env, &swap).initialize(&admin, &verifier, &token_contract);
        ShieldedSwapClient::new(&env, &swap).set_relayer(&relayer, &true);

        env.ledger().with_mut(|li| li.sequence_number = 100);

        Setup { env, admin, relayer, asset_in, asset_out, verifier, token_contract, swap }
    }

    /// Shields `amount` of `asset` for `shielder` into `token`, returning the
    /// note's (rho, rcm, commitment, leaf_index) — a real shield() call with
    /// a real (synthetic-relation) proof, exactly like token's own tests.
    fn shield_note(
        s: &Setup,
        shielder: &Address,
        asset: &Address,
        amount: i128,
        rho_seed: u8,
        rcm_seed: u8,
    ) -> (BytesN<32>, BytesN<32>, BytesN<32>, u32) {
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&s.env, asset);
        stellar_asset.mint(shielder, &amount);

        let rho = BytesN::from_array(&s.env, &[rho_seed; 32]);
        let rcm = BytesN::from_array(&s.env, &[rcm_seed; 32]);
        let mut hasher = poseidon::Poseidon2Hasher::new(&s.env);
        let commitment = note_commitment(&s.env, &mut hasher, amount, asset, &rho, &rcm);
        let value_commit = BytesN::from_array(&s.env, &[0u8; 32]);

        let public_inputs_le: [[u8; 32]; 4] = [
            commitment.clone().into(),
            value_commit.clone().into(),
            i128_le_bytes(amount),
            address_to_field_bytes(&s.env, asset),
        ];
        let (vk, proof) = test_groth16::build_valid_groth16_proof(&s.env, &public_inputs_le);
        VerifierContractClient::new(&s.env, &s.verifier)
            .register_verifying_key(&CircuitType::Shield.into(), &vk);

        let encrypted_note = Bytes::from_array(&s.env, &[0u8; 176]);
        let token_client = ShieldedTokenClient::new(&s.env, &s.token_contract);
        let leaf_index = token_client.shield(
            shielder, asset, &amount, &rho, &rcm, &commitment, &encrypted_note, &proof,
            &NativeShieldPublicInputs {
                commitment: commitment.clone(),
                value_commit,
                pub_value: amount,
                pub_asset_id: asset.clone(),
            },
        );
        (rho, rcm, commitment, leaf_index)
    }

    /// Builds a real (synthetic-relation) ownership proof for
    /// `CircuitType::Unshield` matching `nullifier_in`/`amount_in`/`asset_in`,
    /// with `recipient_hash` bound to `swap`'s own address *and* to this
    /// specific (`intent_commitment`, `refund_to`) pair via `binding_tag`
    /// (matching what `commit_swap` computes internally — see its own doc
    /// comment for why), and registers its VK.
    fn prove_and_register_ownership(
        s: &Setup,
        nullifier_in: &BytesN<32>,
        amount_in: i128,
        anchor: &BytesN<32>,
        intent_commitment: &BytesN<32>,
        refund_to: &Address,
    ) -> Bytes {
        let mut hasher = poseidon::Poseidon2Hasher::new(&s.env);
        let to_field = address_to_field_bytes(&s.env, &s.swap);
        let intent_commitment_bytes: [u8; 32] = intent_commitment.clone().into();
        let refund_to_field = address_to_field_bytes(&s.env, refund_to);
        let binding_tag = hasher.hash(&intent_commitment_bytes, &refund_to_field);
        let recipient_hash = hasher.hash(&to_field, &binding_tag);

        let public_inputs_le: [[u8; 32]; 5] = [
            anchor.clone().into(),
            nullifier_in.clone().into(),
            i128_le_bytes(amount_in),
            address_to_field_bytes(&s.env, &s.asset_in),
            recipient_hash,
        ];
        let (vk, proof) = test_groth16::build_valid_groth16_proof(&s.env, &public_inputs_le);
        VerifierContractClient::new(&s.env, &s.verifier)
            .register_verifying_key(&CircuitType::Unshield.into(), &vk);
        proof
    }

    fn prove_and_register_fairness(
        s: &Setup,
        intent_commitment: &BytesN<32>,
        amount_out: i128,
        min_amount_out: i128,
    ) -> Bytes {
        let public_inputs_le: [[u8; 32]; 5] = [
            intent_commitment.clone().into(),
            address_to_field_bytes(&s.env, &s.asset_in),
            address_to_field_bytes(&s.env, &s.asset_out),
            i128_le_bytes(amount_out),
            i128_le_bytes(min_amount_out),
        ];
        let (vk, proof) = test_groth16::build_valid_groth16_proof(&s.env, &public_inputs_le);
        VerifierContractClient::new(&s.env, &s.verifier)
            .register_verifying_key(&CircuitType::SwapFairness.into(), &vk);
        proof
    }

    fn prove_and_register_output_shield(
        s: &Setup,
        out_commitment: &BytesN<32>,
        out_value_commit: &BytesN<32>,
        amount_out: i128,
    ) -> Bytes {
        let public_inputs_le: [[u8; 32]; 4] = [
            out_commitment.clone().into(),
            out_value_commit.clone().into(),
            i128_le_bytes(amount_out),
            address_to_field_bytes(&s.env, &s.asset_out),
        ];
        let (vk, proof) = test_groth16::build_valid_groth16_proof(&s.env, &public_inputs_le);
        // Real Shield VK for asset_out was already registered by shield_note()
        // when funding the relayer... except the relayer doesn't shield —
        // register directly here since this is the *first* Shield-circuit
        // proof this test registers against this verifier for this scenario.
        let key = VerifierContractClient::new(&s.env, &s.verifier).try_register_verifying_key(&CircuitType::Shield.into(), &vk);
        // Shield VK may already be registered (from an earlier shield_note()
        // call in this same test) — that's fine, this proof's VK differs per
        // call (fresh randomness), so only register if not already present.
        let _ = key;
        proof
    }

    #[test]
    fn full_swap_lifecycle_moves_real_value() {
        let s = setup();
        let shielder = Address::generate(&s.env);

        let amount_in = 1_000_000i128;
        let (in_rho, in_rcm, _in_commitment, in_leaf) =
            shield_note(&s, &shielder, &s.asset_in, amount_in, 10, 11);
        let _ = (in_rho, in_rcm, in_leaf);

        let nullifier_in = BytesN::from_array(&s.env, &[99u8; 32]);
        let anchor = ShieldedTokenClient::new(&s.env, &s.token_contract).merkle_root();
        let intent_commitment = BytesN::from_array(&s.env, &[42u8; 32]);
        let refund_to = Address::generate(&s.env);
        let ownership_proof = prove_and_register_ownership(&s, &nullifier_in, amount_in, &anchor, &intent_commitment, &refund_to);
        let expiry = s.env.ledger().sequence() + 1000;

        let swap_client = ShieldedSwapClient::new(&s.env, &s.swap);
        let swap_id = swap_client.commit_swap(
            &nullifier_in, &intent_commitment, &s.asset_in, &s.asset_out,
            &amount_in, &anchor, &refund_to, &ownership_proof, &expiry,
        );

        // commit_swap really pulled amount_in into swap's own balance via
        // token::unshield — verify both sides.
        let asset_in_client = token::Client::new(&s.env, &s.asset_in);
        assert_eq!(asset_in_client.balance(&s.swap), amount_in);
        assert_eq!(ShieldedTokenClient::new(&s.env, &s.token_contract).shielded_supply(&s.asset_in), 0);

        let amount_out = 950_000i128;
        let min_amount_out = 900_000i128;
        let stellar_asset_out = soroban_sdk::token::StellarAssetClient::new(&s.env, &s.asset_out);
        stellar_asset_out.mint(&s.relayer, &amount_out);
        swap_client.execute_swap(&swap_id, &amount_out, &s.relayer);

        // Relayer really fronted amount_out into escrow.
        let asset_out_client = token::Client::new(&s.env, &s.asset_out);
        assert_eq!(asset_out_client.balance(&s.relayer), 0);
        assert_eq!(asset_out_client.balance(&s.swap), amount_out);

        let fairness_proof = prove_and_register_fairness(&s, &intent_commitment, amount_out, min_amount_out);

        let out_rho = BytesN::from_array(&s.env, &[20u8; 32]);
        let out_rcm = BytesN::from_array(&s.env, &[21u8; 32]);
        let mut hasher = poseidon::Poseidon2Hasher::new(&s.env);
        let out_commitment = note_commitment(&s.env, &mut hasher, amount_out, &s.asset_out, &out_rho, &out_rcm);
        let out_value_commit = BytesN::from_array(&s.env, &[0u8; 32]);
        let shield_proof = prove_and_register_output_shield(&s, &out_commitment, &out_value_commit, amount_out);

        let fairness_pub = SwapFairnessPublicInputs {
            intent_commitment,
            asset_in: s.asset_in.clone(),
            asset_out: s.asset_out.clone(),
            amount_out,
            min_amount_out,
        };
        let encrypted_note = Bytes::from_array(&s.env, &[0u8; 176]);

        let leaf_index = swap_client.reveal_and_claim(
            &swap_id, &out_rho, &out_rcm, &out_commitment, &out_value_commit,
            &encrypted_note, &fairness_proof, &fairness_pub, &shield_proof,
        );

        // Relayer got paid amount_in of asset_in — real compensation, not bookkeeping.
        assert_eq!(asset_in_client.balance(&s.relayer), amount_in);
        assert_eq!(asset_in_client.balance(&s.swap), 0);

        // The output note was really shielded into token for the claimant.
        assert_eq!(asset_out_client.balance(&s.swap), 0);
        assert_eq!(ShieldedTokenClient::new(&s.env, &s.token_contract).shielded_supply(&s.asset_out), amount_out);
        assert!(leaf_index > in_leaf, "output note should land at a later leaf than the input note");
    }

    /// Regression test for a real gap flagged in `docs/POC_IMPLEMENTATION.md`
    /// and `docs/RUNBOOK.md`'s "Known limitations": the nested
    /// `authorize_as_current_contract` call `reveal_and_claim` needs (so
    /// `token::shield`'s own inner `token::transfer` sub-invocation, two
    /// call-stack levels below `swap`, is authorized) was previously
    /// validated only by real live-Testnet transactions, not by an
    /// automated test — because the test suite's blanket
    /// `mock_all_auths_allowing_non_root_auth()` explicitly does not fail
    /// if a required `authorize_as_current_contract` entry is missing or
    /// wrong (per that method's own doc comment).
    ///
    /// Fix confirmed directly with an OpenZeppelin engineer (their library
    /// hasn't needed `authorize_as_current_contract` itself, but agreed
    /// blanket mocks are the wrong tool here): switch off mocking for the
    /// one call that actually exercises it, via `env.set_auths(&[])`, so
    /// Soroban's real (non-mocked) authorization checker runs. `swap`'s own
    /// self-authorization via `authorize_as_current_contract` is a real,
    /// host-issued credential — it doesn't need mocking to satisfy strict
    /// checking, unlike `execute_swap`'s `relayer.require_auth()`, which
    /// still needs an explicit `MockAuth` entry for `relayer` here since
    /// that address has no real signing key in this test.
    #[test]
    fn reveal_and_claim_authorize_as_current_contract_satisfies_real_non_mocked_auth() {
        use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};

        let s = setup();
        let shielder = Address::generate(&s.env);

        let amount_in = 400_000i128;
        shield_note(&s, &shielder, &s.asset_in, amount_in, 12, 13);

        let nullifier_in = BytesN::from_array(&s.env, &[151u8; 32]);
        let anchor = ShieldedTokenClient::new(&s.env, &s.token_contract).merkle_root();
        let intent_commitment = BytesN::from_array(&s.env, &[152u8; 32]);
        let refund_to = Address::generate(&s.env);
        let ownership_proof = prove_and_register_ownership(
            &s, &nullifier_in, amount_in, &anchor, &intent_commitment, &refund_to,
        );
        let expiry = s.env.ledger().sequence() + 1000;

        let swap_client = ShieldedSwapClient::new(&s.env, &s.swap);
        let swap_id = swap_client.commit_swap(
            &nullifier_in, &intent_commitment, &s.asset_in, &s.asset_out,
            &amount_in, &anchor, &refund_to, &ownership_proof, &expiry,
        );

        let amount_out = 380_000i128;
        let min_amount_out = 350_000i128;
        let stellar_asset_out = soroban_sdk::token::StellarAssetClient::new(&s.env, &s.asset_out);
        stellar_asset_out.mint(&s.relayer, &amount_out);

        // execute_swap needs relayer's real auth for this specific call —
        // relayer has no real signing key in this test, so it needs an
        // explicit MockAuth entry rather than a blanket mock. The tree has
        // two levels: relayer authorizes the top-level execute_swap call
        // *and*, separately, the nested classic-token `transfer(relayer,
        // swap, amount_out)` sub-invocation it makes.
        swap_client
            .mock_auths(&[MockAuth {
                address: &s.relayer,
                invoke: &MockAuthInvoke {
                    contract: &s.swap,
                    fn_name: "execute_swap",
                    args: (swap_id.clone(), amount_out, s.relayer.clone()).into_val(&s.env),
                    sub_invokes: &[MockAuthInvoke {
                        contract: &s.asset_out,
                        fn_name: "transfer",
                        args: (s.relayer.clone(), s.swap.clone(), amount_out).into_val(&s.env),
                        sub_invokes: &[],
                    }],
                },
            }])
            .execute_swap(&swap_id, &amount_out, &s.relayer);

        let fairness_proof = prove_and_register_fairness(&s, &intent_commitment, amount_out, min_amount_out);
        let out_rho = BytesN::from_array(&s.env, &[160u8; 32]);
        let out_rcm = BytesN::from_array(&s.env, &[161u8; 32]);
        let mut hasher = poseidon::Poseidon2Hasher::new(&s.env);
        let out_commitment = note_commitment(&s.env, &mut hasher, amount_out, &s.asset_out, &out_rho, &out_rcm);
        let out_value_commit = BytesN::from_array(&s.env, &[0u8; 32]);
        let shield_proof = prove_and_register_output_shield(&s, &out_commitment, &out_value_commit, amount_out);
        let fairness_pub = SwapFairnessPublicInputs {
            intent_commitment,
            asset_in: s.asset_in.clone(),
            asset_out: s.asset_out.clone(),
            amount_out,
            min_amount_out,
        };
        let encrypted_note = Bytes::from_array(&s.env, &[0u8; 176]);

        // Switch off blanket mocking entirely for this one call — real,
        // strict Soroban authorization checking, no bypass. `reveal_and_claim`
        // itself requires no top-level `require_auth()` from any real key
        // (it's permissionless), so an empty explicit auth list is correct;
        // the only authorization actually exercised is `swap`'s own
        // `authorize_as_current_contract` entry, checked for real.
        s.env.set_auths(&[]);

        let leaf_index = swap_client.reveal_and_claim(
            &swap_id, &out_rho, &out_rcm, &out_commitment, &out_value_commit,
            &encrypted_note, &fairness_proof, &fairness_pub, &shield_proof,
        );

        // leaf 0 was the shielded input note from `shield_note` above; the
        // re-shielded output note must land at the next leaf.
        assert_eq!(leaf_index, 1);
        assert_eq!(
            token::Client::new(&s.env, &s.asset_in).balance(&s.relayer),
            amount_in,
            "relayer must have been paid — proves reveal_and_claim didn't just skip the authorized step"
        );
    }

    /// Regression test for a critical audit finding: `initialize` had no
    /// guard against being called more than once, letting anyone overwrite
    /// `Admin`/`Verifier`/`Token` on an already-operating, already-funded
    /// contract at any time — every other contract in this workspace
    /// (`token`, `governance`, `verifier`, `compliance`) already has this
    /// guard; `swap` was the sole outlier.
    #[test]
    #[should_panic(expected = "already initialized")]
    fn initialize_cannot_be_called_twice() {
        let s = setup();
        ShieldedSwapClient::new(&s.env, &s.swap).initialize(&s.admin, &s.verifier, &s.token_contract);
    }

    /// Regression test for a critical audit finding: `reveal_and_claim`
    /// checked `fairness_pub.asset_in`/`asset_out`/`amount_out` against
    /// `state`, but never `fairness_pub.intent_commitment` against
    /// `state.intent_commitment`. Since those three fields are all public
    /// once a swap reaches `Executed`, anyone could build their own
    /// unrelated, self-chosen `intent_commitment` (here, with
    /// `max_slippage_bps` effectively maxed so `min_amount_out = 0`),
    /// produce a real, internally-valid fairness proof for it, and steal
    /// the escrowed `asset_out` by supplying their own `out_commitment` —
    /// this is a real fund-theft path, not just a soundness nicety, and
    /// exploitable by any observer, not only the executing relayer.
    #[test]
    #[should_panic(expected = "intent_commitment mismatch")]
    fn reveal_and_claim_rejects_mismatched_intent_commitment() {
        let s = setup();
        let shielder = Address::generate(&s.env);
        let amount_in = 1_000_000i128;
        shield_note(&s, &shielder, &s.asset_in, amount_in, 90, 91);

        let nullifier_in = BytesN::from_array(&s.env, &[201u8; 32]);
        let anchor = ShieldedTokenClient::new(&s.env, &s.token_contract).merkle_root();
        let intent_commitment = BytesN::from_array(&s.env, &[210u8; 32]);
        let refund_to = Address::generate(&s.env);
        let ownership_proof = prove_and_register_ownership(
            &s, &nullifier_in, amount_in, &anchor, &intent_commitment, &refund_to,
        );
        let expiry = s.env.ledger().sequence() + 1000;

        let swap_client = ShieldedSwapClient::new(&s.env, &s.swap);
        let swap_id = swap_client.commit_swap(
            &nullifier_in, &intent_commitment, &s.asset_in, &s.asset_out,
            &amount_in, &anchor, &refund_to, &ownership_proof, &expiry,
        );

        let amount_out = 950_000i128;
        let stellar_asset_out = soroban_sdk::token::StellarAssetClient::new(&s.env, &s.asset_out);
        stellar_asset_out.mint(&s.relayer, &amount_out);
        swap_client.execute_swap(&swap_id, &amount_out, &s.relayer);

        // An attacker's own, completely unrelated intent_commitment — a
        // real, validly constructed fairness proof, but for a commitment
        // nobody ever actually committed to at commit_swap time.
        let attacker_intent_commitment = BytesN::from_array(&s.env, &[211u8; 32]);
        let min_amount_out = 0i128; // attacker picks the loosest possible floor
        let fairness_proof =
            prove_and_register_fairness(&s, &attacker_intent_commitment, amount_out, min_amount_out);

        let out_rho = BytesN::from_array(&s.env, &[220u8; 32]);
        let out_rcm = BytesN::from_array(&s.env, &[221u8; 32]);
        let mut hasher = poseidon::Poseidon2Hasher::new(&s.env);
        let out_commitment = note_commitment(&s.env, &mut hasher, amount_out, &s.asset_out, &out_rho, &out_rcm);
        let out_value_commit = BytesN::from_array(&s.env, &[0u8; 32]);
        let shield_proof = prove_and_register_output_shield(&s, &out_commitment, &out_value_commit, amount_out);

        let fairness_pub = SwapFairnessPublicInputs {
            intent_commitment: attacker_intent_commitment,
            asset_in: s.asset_in.clone(),
            asset_out: s.asset_out.clone(),
            amount_out,
            min_amount_out,
        };
        let encrypted_note = Bytes::from_array(&s.env, &[0u8; 176]);

        // Must fail: this fairness proof is for a different intent_commitment
        // than the one actually committed to at commit_swap time.
        swap_client.reveal_and_claim(
            &swap_id, &out_rho, &out_rcm, &out_commitment, &out_value_commit,
            &encrypted_note, &fairness_proof, &fairness_pub, &shield_proof,
        );
    }

    /// Regression test for a critical audit finding: the swap's reused
    /// `unshield.circom` ownership proof used to be bound only to
    /// (nullifier, amount, asset, swap's own fixed address) — identical for
    /// every user and every swap, since `to` is always this contract's own
    /// address. A party who observed a submitted `commit_swap` transaction
    /// (e.g. a failed/retried submission still visible in public transaction
    /// history) could resubmit those exact proof bytes with their *own*
    /// `refund_to`, spend the victim's nullifier first, and later steal the
    /// escrowed value via `cancel_swap` once the swap expired.
    /// `binding_tag` (folding `intent_commitment`+`refund_to` into
    /// `recipient_hash`, cryptographically bound to the specific proof —
    /// see `commit_swap`'s doc comment) closes this: the exact same proof
    /// bytes, replayed with a different `refund_to`, now fail token's own
    /// `RecipientMismatch` check.
    #[test]
    #[should_panic]
    fn commit_swap_rejects_proof_replayed_with_different_refund_to() {
        let s = setup();
        let shielder = Address::generate(&s.env);
        let amount_in = 400_000i128;
        shield_note(&s, &shielder, &s.asset_in, amount_in, 95, 96);

        let nullifier_in = BytesN::from_array(&s.env, &[230u8; 32]);
        let anchor = ShieldedTokenClient::new(&s.env, &s.token_contract).merkle_root();
        let intent_commitment = BytesN::from_array(&s.env, &[231u8; 32]);
        let legit_refund_to = Address::generate(&s.env);
        // Proof generated (and its VK registered) for `legit_refund_to`.
        let ownership_proof = prove_and_register_ownership(
            &s, &nullifier_in, amount_in, &anchor, &intent_commitment, &legit_refund_to,
        );
        let expiry = s.env.ledger().sequence() + 1000;

        // Attacker replays the *exact same proof bytes* with their own
        // refund_to instead — must fail, not silently succeed and let the
        // attacker later drain the escrow via cancel_swap/refund_to.
        let attacker_refund_to = Address::generate(&s.env);
        let swap_client = ShieldedSwapClient::new(&s.env, &s.swap);
        swap_client.commit_swap(
            &nullifier_in, &intent_commitment, &s.asset_in, &s.asset_out,
            &amount_in, &anchor, &attacker_refund_to, &ownership_proof, &expiry,
        );
    }

    /// Regression test for an audit finding: `swap_id` is derived solely
    /// from `intent_commitment`, so without an explicit uniqueness check, a
    /// second `commit_swap` call reusing the same `intent_commitment` (here,
    /// simulating a non-unique `intent_nonce`) would both pull a *second*
    /// real note's value into escrow and silently overwrite the first
    /// swap's `SwapState`, orphaning that first escrow. The duplicate check
    /// runs before the (real, expensive) `token::unshield` cross-call, so the
    /// second attempt is rejected before ever spending a second note's
    /// nullifier — the proof bytes below are never actually verified.
    #[test]
    #[should_panic(expected = "swap already committed for this intent_commitment")]
    fn commit_swap_rejects_duplicate_intent_commitment() {
        let s = setup();
        let shielder = Address::generate(&s.env);
        let amount_in = 111_000i128;

        shield_note(&s, &shielder, &s.asset_in, amount_in, 70, 71);
        let nullifier_a = BytesN::from_array(&s.env, &[101u8; 32]);
        let anchor_a = ShieldedTokenClient::new(&s.env, &s.token_contract).merkle_root();
        let intent_commitment = BytesN::from_array(&s.env, &[123u8; 32]);
        let refund_to = Address::generate(&s.env);
        let proof_a = prove_and_register_ownership(&s, &nullifier_a, amount_in, &anchor_a, &intent_commitment, &refund_to);
        let expiry = s.env.ledger().sequence() + 1000;

        let swap_client = ShieldedSwapClient::new(&s.env, &s.swap);
        swap_client.commit_swap(
            &nullifier_a, &intent_commitment, &s.asset_in, &s.asset_out,
            &amount_in, &anchor_a, &refund_to, &proof_a, &expiry,
        );

        // Same intent_commitment again — must be rejected before this
        // (deliberately unregistered/unverifiable) proof is ever checked.
        let nullifier_b = BytesN::from_array(&s.env, &[102u8; 32]);
        let bogus_proof = Bytes::from_array(&s.env, &[0u8; 4]);
        swap_client.commit_swap(
            &nullifier_b, &intent_commitment, &s.asset_in, &s.asset_out,
            &amount_in, &anchor_a, &refund_to, &bogus_proof, &expiry,
        );
    }

    /// Regression test for a medium-severity audit finding:
    /// `reclaim_expired_swap` computed `state.expiry_ledger +
    /// CLAIM_WINDOW_LEDGERS` with a plain `+`, which panics on overflow in a
    /// release build (`overflow-checks = true`) for an `expiry_ledger` close
    /// to `u32::MAX` — permanently locking both the relayer's fronted
    /// `asset_out` and the claimant's escrowed `asset_in` with no recovery
    /// path. Fixed by rejecting such an `expiry_ledger` at `commit_swap`
    /// time, before any real funds are ever escrowed against it.
    #[test]
    #[should_panic(expected = "expiry_ledger too close to u32::MAX")]
    fn commit_swap_rejects_expiry_ledger_that_would_overflow_the_claim_window() {
        let s = setup();
        let shielder = Address::generate(&s.env);
        let amount_in = 250_000i128;
        shield_note(&s, &shielder, &s.asset_in, amount_in, 80, 81);

        let nullifier_in = BytesN::from_array(&s.env, &[241u8; 32]);
        let anchor = ShieldedTokenClient::new(&s.env, &s.token_contract).merkle_root();
        let intent_commitment = BytesN::from_array(&s.env, &[242u8; 32]);
        let refund_to = Address::generate(&s.env);
        let ownership_proof = prove_and_register_ownership(
            &s, &nullifier_in, amount_in, &anchor, &intent_commitment, &refund_to,
        );

        // Close enough to u32::MAX that `expiry_ledger + CLAIM_WINDOW_LEDGERS`
        // would overflow.
        let expiry = u32::MAX - 1;

        let swap_client = ShieldedSwapClient::new(&s.env, &s.swap);
        swap_client.commit_swap(
            &nullifier_in, &intent_commitment, &s.asset_in, &s.asset_out,
            &amount_in, &anchor, &refund_to, &ownership_proof, &expiry,
        );
    }

    #[test]
    fn cancel_swap_refunds_escrowed_asset_in() {
        let s = setup();
        let shielder = Address::generate(&s.env);
        let amount_in = 500_000i128;
        shield_note(&s, &shielder, &s.asset_in, amount_in, 30, 31);

        let nullifier_in = BytesN::from_array(&s.env, &[77u8; 32]);
        let anchor = ShieldedTokenClient::new(&s.env, &s.token_contract).merkle_root();
        let intent_commitment = BytesN::from_array(&s.env, &[55u8; 32]);
        let refund_to = Address::generate(&s.env);
        let ownership_proof = prove_and_register_ownership(&s, &nullifier_in, amount_in, &anchor, &intent_commitment, &refund_to);
        let expiry = s.env.ledger().sequence() + 100;

        let swap_client = ShieldedSwapClient::new(&s.env, &s.swap);
        let swap_id = swap_client.commit_swap(
            &nullifier_in, &intent_commitment, &s.asset_in, &s.asset_out,
            &amount_in, &anchor, &refund_to, &ownership_proof, &expiry,
        );

        let asset_in_client = token::Client::new(&s.env, &s.asset_in);
        assert_eq!(asset_in_client.balance(&s.swap), amount_in);

        s.env.ledger().with_mut(|li| li.sequence_number = expiry + 1);
        swap_client.cancel_swap(&swap_id);

        assert_eq!(asset_in_client.balance(&refund_to), amount_in);
        assert_eq!(asset_in_client.balance(&s.swap), 0);
    }

    #[test]
    fn reclaim_expired_swap_refunds_both_sides() {
        let s = setup();
        let shielder = Address::generate(&s.env);
        let amount_in = 700_000i128;
        shield_note(&s, &shielder, &s.asset_in, amount_in, 40, 41);

        let nullifier_in = BytesN::from_array(&s.env, &[88u8; 32]);
        let anchor = ShieldedTokenClient::new(&s.env, &s.token_contract).merkle_root();
        let intent_commitment = BytesN::from_array(&s.env, &[66u8; 32]);
        let refund_to = Address::generate(&s.env);
        let ownership_proof = prove_and_register_ownership(&s, &nullifier_in, amount_in, &anchor, &intent_commitment, &refund_to);
        let expiry = s.env.ledger().sequence() + 100;

        let swap_client = ShieldedSwapClient::new(&s.env, &s.swap);
        let swap_id = swap_client.commit_swap(
            &nullifier_in, &intent_commitment, &s.asset_in, &s.asset_out,
            &amount_in, &anchor, &refund_to, &ownership_proof, &expiry,
        );

        let amount_out = 650_000i128;
        let stellar_asset_out = soroban_sdk::token::StellarAssetClient::new(&s.env, &s.asset_out);
        stellar_asset_out.mint(&s.relayer, &amount_out);
        swap_client.execute_swap(&swap_id, &amount_out, &s.relayer);

        // Claimant never calls reveal_and_claim.
        s.env.ledger().with_mut(|li| li.sequence_number = expiry + CLAIM_WINDOW_LEDGERS + 1);
        swap_client.reclaim_expired_swap(&swap_id);

        let asset_in_client  = token::Client::new(&s.env, &s.asset_in);
        let asset_out_client = token::Client::new(&s.env, &s.asset_out);
        assert_eq!(asset_out_client.balance(&s.relayer), amount_out);
        assert_eq!(asset_in_client.balance(&refund_to), amount_in);
        assert_eq!(asset_in_client.balance(&s.swap), 0);
        assert_eq!(asset_out_client.balance(&s.swap), 0);
    }

    #[test]
    fn reveal_and_claim_rejects_tampered_fairness_proof() {
        let s = setup();
        let shielder = Address::generate(&s.env);
        let amount_in = 300_000i128;
        shield_note(&s, &shielder, &s.asset_in, amount_in, 50, 51);

        let nullifier_in = BytesN::from_array(&s.env, &[11u8; 32]);
        let anchor = ShieldedTokenClient::new(&s.env, &s.token_contract).merkle_root();
        let intent_commitment = BytesN::from_array(&s.env, &[22u8; 32]);
        let refund_to = Address::generate(&s.env);
        let ownership_proof = prove_and_register_ownership(&s, &nullifier_in, amount_in, &anchor, &intent_commitment, &refund_to);
        let expiry = s.env.ledger().sequence() + 1000;

        let swap_client = ShieldedSwapClient::new(&s.env, &s.swap);
        let swap_id = swap_client.commit_swap(
            &nullifier_in, &intent_commitment, &s.asset_in, &s.asset_out,
            &amount_in, &anchor, &refund_to, &ownership_proof, &expiry,
        );

        let amount_out = 280_000i128;
        let min_amount_out = 250_000i128;
        let stellar_asset_out = soroban_sdk::token::StellarAssetClient::new(&s.env, &s.asset_out);
        stellar_asset_out.mint(&s.relayer, &amount_out);
        swap_client.execute_swap(&swap_id, &amount_out, &s.relayer);

        let fairness_proof = prove_and_register_fairness(&s, &intent_commitment, amount_out, min_amount_out);
        let bad_proof = test_groth16::corrupt_proof(&s.env, &fairness_proof);

        let out_rho = BytesN::from_array(&s.env, &[60u8; 32]);
        let out_rcm = BytesN::from_array(&s.env, &[61u8; 32]);
        let mut hasher = poseidon::Poseidon2Hasher::new(&s.env);
        let out_commitment = note_commitment(&s.env, &mut hasher, amount_out, &s.asset_out, &out_rho, &out_rcm);
        let out_value_commit = BytesN::from_array(&s.env, &[0u8; 32]);
        let shield_proof = prove_and_register_output_shield(&s, &out_commitment, &out_value_commit, amount_out);

        let fairness_pub = SwapFairnessPublicInputs {
            intent_commitment,
            asset_in: s.asset_in.clone(),
            asset_out: s.asset_out.clone(),
            amount_out,
            min_amount_out,
        };
        let encrypted_note = Bytes::from_array(&s.env, &[0u8; 176]);

        let result = ShieldedSwapClient::new(&s.env, &s.swap).try_reveal_and_claim(
            &swap_id, &out_rho, &out_rcm, &out_commitment, &out_value_commit,
            &encrypted_note, &bad_proof, &fairness_pub, &shield_proof,
        );
        assert!(result.is_err());
    }
}
