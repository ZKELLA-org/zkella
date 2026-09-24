#![no_std]

mod merkle;
mod poseidon;
mod types;

#[cfg(test)]
mod test_groth16;

use soroban_sdk::{
    contract, contractimpl, symbol_short,
    token, Address, Bytes, BytesN, Env, Vec,
    xdr::ToXdr,
};
use zkella_verifier_interface::{CircuitType, VerifierClient};

use types::{
    NoteCommitmentEvent, NullifierEvent, ShieldBatchItem, ShieldEvent,
    StorageKey, UnshieldEvent,
};
// Re-exported for downstream crates that deploy a real `ShieldedToken` in
// their own tests (e.g. `contracts/swap`'s test suite, which shields a real
// note via a direct `ShieldedTokenClient` call before exercising
// `swap::commit_swap`'s cross-call into `token::unshield`).
pub use types::{Error, ShieldPublicInputs, TransferPublicInputs, UnshieldPublicInputs};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Default minimum shield amount in base units, set at `initialize` time.
/// Prevents spam note insertion at near-zero cost. Governance-settable
/// thereafter via `set_min_shield_amount`, not a fixed constant — see that
/// function's doc comment.
const DEFAULT_MIN_SHIELD_AMOUNT: i128 = 1_000;

/// Expected byte length of an encrypted note bundle (ephemeral_pk || chacha-poly ciphertext).
/// 32 (ephemeral pk) + 128 (plaintext) + 16 (Poly1305 MAC) = 176.
const ENCRYPTED_NOTE_LEN: u32 = 176;

/// Instance storage TTL parameters (Stellar ledger ≈ 5 s).
const INSTANCE_TTL_THRESHOLD: u32 = 17_280 * 30;  // 30 days: only bump if below this
const INSTANCE_TTL_EXTEND_TO: u32 = 17_280 * 365; // extend to 1 year from now

// ── Note commitment ───────────────────────────────────────────────────────────

/// Compute note commitment: Poseidon2(Poseidon2(value, asset_field), Poseidon2(rho, rcm))
///
/// Field encoding:
///   value       — little-endian u128, zero-padded to 32 bytes (safe for u64 amounts)
///   asset_field — raw 32-byte contract ID extracted from the Address XDR
///                 (matches SDK's addressToField = StrKey binary decode → 32 bytes)
///   rho / rcm   — passed as-is (caller ensures they are valid field elements)
///
/// This encoding is cross-validated with the TypeScript SDK via test vectors in
/// circuits/shield/shield_test_vectors.json.
fn compute_commitment(
    env:    &Env,
    value:  i128,
    asset:  &Address,
    rho:    &BytesN<32>,
    rcm:    &BytesN<32>,
    hasher: &mut poseidon::Poseidon2Hasher,
) -> [u8; 32] {
    let mut value_bytes = [0u8; 32];
    value_bytes[..16].copy_from_slice(&(value as u128).to_le_bytes());

    let asset_bytes = address_to_field_bytes(env, asset);

    let rho_bytes: [u8; 32] = rho.clone().into();
    let rcm_bytes: [u8; 32] = rcm.clone().into();

    let h1 = hasher.hash(&value_bytes, &asset_bytes);
    let h2 = hasher.hash(&rho_bytes, &rcm_bytes);
    hasher.hash(&h1, &h2)
}

/// BN254 scalar-field modulus r, big-endian.
const FR_MODULUS_BE: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

/// Extract the raw 32-byte contract ID from a Soroban Address via XDR.
///
/// `addr.to_xdr(env)` serializes the full `ScVal::Address(ScAddress::Contract(Hash))`,
/// not a bare `ScAddress` — so there are *two* 4-byte discriminants ahead of the
/// hash (the `ScVal` tag, then the `ScAddress` tag), not one. An earlier version
/// of this function assumed only the latter and read from a fixed offset of 4,
/// which actually landed on the `ScAddress` discriminant itself and truncated the
/// last 4 bytes of the real hash — caught by cross-checking a real testnet
/// contract address's derived value against an independent StrKey decode (see
/// `diagnostic_print_commitment_for_real_testnet_shield` below). Reading the
/// *last* 32 bytes instead of a fixed forward offset is robust to that kind of
/// wrapping regardless of how many discriminants precede the hash.
///
/// This produces the same bytes as the TypeScript SDK's addressToField():
///   StrKey base32-decode → skip 1-byte version + 2-byte checksum → 32-byte payload
/// Both paths yield the same underlying 32-byte contract ID.
fn address_to_field_bytes(env: &Env, addr: &Address) -> [u8; 32] {
    let xdr = addr.to_xdr(env);
    let mut out = [0u8; 32];
    let start = xdr.len() - 32;
    for i in 0..32u32 {
        out[i as usize] = xdr.get(start + i).unwrap_or(0) as u8;
    }
    // Reduce mod r: public inputs must be canonical field elements (the
    // verifier rejects >= r), and Poseidon reduces the same way internally, so
    // every hash over this value is unchanged.
    let mut be = out;
    be.reverse();
    let r = soroban_sdk::U256::from_be_bytes(env, &soroban_sdk::Bytes::from_array(env, &FR_MODULUS_BE));
    let v = soroban_sdk::U256::from_be_bytes(env, &soroban_sdk::Bytes::from_array(env, &be)).rem_euclid(&r);
    let reduced: [u8; 32] = v.to_be_bytes().try_into().expect("u256 is 32 bytes");
    let mut le = reduced;
    le.reverse();
    le
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct ShieldedToken;

#[contractimpl]
impl ShieldedToken {

    /// Initialize the contract. Can only be called once.
    /// `verifier` is the address of a deployed `zkella-verifier` registry
    /// contract with a verifying key already registered for
    /// `CircuitType::Shield` (and, once implemented, Transfer/Unshield).
    /// The verifying key itself lives in that contract, not here — see
    /// `contracts/verifier` for why it's kept separate.
    pub fn initialize(
        env:      Env,
        admin:    Address,
        verifier: Address,
    ) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }

        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage().instance().set(&StorageKey::Verifier, &verifier);
        env.storage().instance().set(&StorageKey::Paused, &false);
        env.storage().instance().set(&StorageKey::NextLeafIndex, &0u32);
        env.storage().instance().set(&StorageKey::MinShieldAmount, &DEFAULT_MIN_SHIELD_AMOUNT);
        // Seed TTL for the freshly created instance storage entries.
        env.storage().instance().extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
    }

    // ── Governance-settable parameters ──────────────────────────────────────

    /// Current minimum shield amount. Starts at `DEFAULT_MIN_SHIELD_AMOUNT`
    /// (set at `initialize`), governance-settable thereafter via
    /// `set_min_shield_amount` rather than requiring a redeploy.
    pub fn min_shield_amount(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&StorageKey::MinShieldAmount)
            .unwrap_or(DEFAULT_MIN_SHIELD_AMOUNT)
    }

    /// Sets the minimum shield amount. Admin-gated (in practice, the
    /// `governance` contract's own address, the same pattern as every other
    /// admin-only entrypoint here) so this is a real governance action, not
    /// a bare-key call.
    pub fn set_min_shield_amount(env: Env, new_amount: i128) -> Result<(), Error> {
        Self::require_admin(&env)?;
        if new_amount <= 0 {
            return Err(Error::AmountMismatch);
        }
        env.storage().instance().set(&StorageKey::MinShieldAmount, &new_amount);
        Ok(())
    }

    /// Whether `asset` may currently be shielded. Nothing is approved by
    /// default at `initialize`, including native XLM — approval is always an
    /// explicit governance action, never an implicit default.
    pub fn is_asset_approved(env: Env, asset: Address) -> bool {
        env.storage()
            .instance()
            .get(&StorageKey::AssetApproved(asset))
            .unwrap_or(false)
    }

    /// Approves or revokes `asset` for shielding. Admin-gated.
    ///
    /// This is the real, in-code policy for the clawback risk a non-native
    /// SEP-41 asset's issuer holds: Stellar Asset Contracts let an issuer
    /// invoke clawback against *any* balance holder, including this
    /// contract's own custodied balance after a deposit, not just a user's
    /// trustline before one. There is no way for this contract to detect or
    /// reverse a clawback after the fact — the real funds backing a shielded
    /// note would simply be gone, with no on-chain signal distinguishing
    /// that from correct operation until someone tries to unshield more than
    /// the contract's real balance can cover. Rather than accept that risk
    /// implicitly for every asset, shielding requires an explicit,
    /// per-asset governance decision first. Native XLM has no issuer and
    /// therefore no clawback right, so approving it is a safe, informed
    /// default a deployment can make immediately; approving any other asset
    /// is a deliberate acceptance of that issuer's clawback risk, made
    /// on-chain and auditable rather than assumed.
    pub fn set_asset_approved(env: Env, asset: Address, approved: bool) -> Result<(), Error> {
        Self::require_admin(&env)?;
        env.storage().instance().set(&StorageKey::AssetApproved(asset), &approved);
        Ok(())
    }

    // ── Shield ────────────────────────────────────────────────────────────────

    /// Move public SEP-41 tokens into the shielded pool.
    ///
    /// Security properties enforced on-chain:
    ///   • `asset` must be governance-approved via `set_asset_approved`
    ///   • `amount > 0` and >= the current `min_shield_amount()`
    ///   • `encrypted_note` must be exactly ENCRYPTED_NOTE_LEN bytes
    ///   • `shield_pub.pub_value` == `amount` and `pub_asset_id` == `asset`
    ///   • commitment == Poseidon2(Poseidon2(value_bytes, asset_bytes), Poseidon2(rho, rcm))
    ///   • commitment has not been seen before (prevents replay / double-spend)
    ///
    /// `shield_proof` is a Groth16 proof (see `contracts/verifier` for wire
    /// format) verified on-chain against the `CircuitType::Shield` verifying
    /// key registered in this contract's configured verifier registry.
    ///
    /// Returns the leaf index assigned in the Merkle tree.
    pub fn shield(
        env:            Env,
        from:           Address,
        asset:          Address,
        amount:         i128,
        rho:            BytesN<32>,
        rcm:            BytesN<32>,
        commitment:     BytesN<32>,
        encrypted_note: Bytes,
        shield_proof:   Bytes,
        shield_pub:     ShieldPublicInputs,
    ) -> Result<u32, Error> {
        // ── 1. Auth & pause check ───────────────────────────────────────────
        from.require_auth();
        Self::assert_not_paused(&env)?;

        // ── 1.5. Asset must be governance-approved for shielding ────────────
        // Real, non-native SEP-41 assets have an issuer who can invoke
        // clawback against any balance holder, including this contract's own
        // custodied balance after deposit — see `set_asset_approved`'s doc
        // comment for the full policy. Native XLM has no issuer and is
        // approved once at `initialize` time; any other asset requires an
        // explicit governance decision before it can be shielded at all.
        if !Self::is_asset_approved(env.clone(), asset.clone()) {
            return Err(Error::AssetNotApproved);
        }

        // ── 2. Validate amount ──────────────────────────────────────────────
        if amount <= 0 {
            return Err(Error::AmountMismatch);
        }
        if amount < Self::min_shield_amount(env.clone()) {
            return Err(Error::AmountMismatch);
        }

        // ── 3. Validate encrypted note length ───────────────────────────────
        if encrypted_note.len() != ENCRYPTED_NOTE_LEN {
            return Err(Error::InvalidNote);
        }

        // ── 4. Validate public inputs match tx params ───────────────────────
        if shield_pub.pub_value != amount {
            return Err(Error::AmountMismatch);
        }
        if shield_pub.pub_asset_id != asset {
            return Err(Error::AssetMismatch);
        }

        // ── 5. Verify commitment matches Poseidon2 re-computation ───────────
        // One hasher reused across the commitment check and the Merkle insert
        // below (~35 hashes total) — see Poseidon2Hasher's doc comment for why
        // a fresh sponge per call blew the instruction budget.
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let computed  = compute_commitment(&env, amount, &asset, &rho, &rcm, &mut hasher);
        let provided: [u8; 32] = commitment.clone().into();
        if computed != provided {
            return Err(Error::CommitmentMismatch);
        }

        // ── 6. Duplicate commitment check (prevents replay / Merkle pollution) ──
        let seen_key = StorageKey::CommitmentSeen(commitment.clone());
        if env.storage().persistent().has(&seen_key) {
            return Err(Error::DuplicateCommitment);
        }

        // ── 7. Groth16 proof verification ────────────────────────────────────
        // Public input order matches circuits/shield/shield.circom's
        // `component main {public [commitment, value_commit, pub_value, pub_asset_id]}`
        // and zkella-verifier's real-shield-circuit test.
        let verifier: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Verifier)
            .ok_or(Error::NotInitialized)?;

        let mut value_bytes = [0u8; 32];
        value_bytes[..16].copy_from_slice(&(amount as u128).to_le_bytes());
        let asset_bytes = address_to_field_bytes(&env, &asset);

        let public_inputs = Vec::from_array(
            &env,
            [
                commitment.clone(),
                shield_pub.value_commit.clone(),
                BytesN::from_array(&env, &value_bytes),
                BytesN::from_array(&env, &asset_bytes),
            ],
        );

        let proof_ok = VerifierClient::new(&env, &verifier).verify(
            &CircuitType::Shield,
            &public_inputs,
            &shield_proof,
        );
        if !proof_ok {
            return Err(Error::InvalidProof);
        }

        // ── 7.5. Merkle tree capacity check ──────────────────────────────────
        // Checked here, before any state mutation, so a full tree fails
        // gracefully via `Error::MerkleTreeFull` rather than reaching
        // `merkle::insert`'s own defensive panic.
        if merkle::is_full(&env) {
            return Err(Error::MerkleTreeFull);
        }

        // ── 8. Effects: record commitment, update supply, insert into tree ──
        // Mark commitment as seen before external token call (reentrancy safety).
        env.storage().persistent().set(&seen_key, &true);
        env.storage().persistent().extend_ttl(&seen_key, 17_280 * 30, 17_280 * 365);

        let prev_supply: i128 = env
            .storage()
            .instance()
            .get(&StorageKey::ShieldedSupply(asset.clone()))
            .unwrap_or(0);
        let new_supply = prev_supply
            .checked_add(amount)
            .ok_or(Error::AmountMismatch)?;
        env.storage()
            .instance()
            .set(&StorageKey::ShieldedSupply(asset.clone()), &new_supply);

        let leaf_index = merkle::insert(&env, commitment.clone(), &mut hasher);

        // Bump instance storage TTL on every shield (keeps root + counter alive).
        env.storage().instance().extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);

        // ── 9. Emit events (before external call so observers see them atomically) ──
        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("shield")),
            ShieldEvent {
                leaf_index,
                asset:      asset.clone(),
                commitment: commitment.clone(),
            },
        );
        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("note")),
            NoteCommitmentEvent {
                leaf_index,
                commitment,
                encrypted_note,
            },
        );

        // ── 10. Interaction: pull tokens from caller (last, after all state changes) ──
        // Doing the transfer last follows checks-effects-interactions and ensures that
        // a reentrant call on a malicious token cannot exploit partially-committed state.
        let token_client = token::Client::new(&env, &asset);
        token_client.transfer(&from, &env.current_contract_address(), &amount);

        Ok(leaf_index)
    }

    /// Shield several deposits of the same `asset` in one call, avoiding the
    /// per-transaction overhead (a separate auth check, a separate SEP-41
    /// transfer, a separate transaction envelope) of calling `shield()` once
    /// per deposit. Fixed to one asset per batch, not because a mixed-asset
    /// batch is unsafe, but because it would need one token transfer per
    /// distinct asset anyway, losing most of the amortization this exists
    /// for; a same-asset batch (the common real case: several payments of
    /// one asset arriving over time, shielded together) still gets a single
    /// aggregated transfer for the whole batch's total.
    ///
    /// Each item still needs and gets its own independent Groth16 proof —
    /// batching does not weaken per-note verification, it only shares the
    /// auth check, the asset-approval check, and the final token transfer
    /// across every item. Returns the leaf index assigned to each item, in
    /// the same order as `items`.
    pub fn shield_batch(
        env:   Env,
        from:  Address,
        asset: Address,
        items: Vec<ShieldBatchItem>,
    ) -> Result<Vec<u32>, Error> {
        // ── 1. Auth & pause check ───────────────────────────────────────────
        from.require_auth();
        Self::assert_not_paused(&env)?;

        // ── 1.5. Asset must be governance-approved for shielding ────────────
        if !Self::is_asset_approved(env.clone(), asset.clone()) {
            return Err(Error::AssetNotApproved);
        }

        if items.is_empty() {
            return Err(Error::EmptyBatch);
        }

        let min_amount = Self::min_shield_amount(env.clone());
        let verifier: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Verifier)
            .ok_or(Error::NotInitialized)?;

        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let mut leaf_indices = Vec::new(&env);
        let mut total_amount: i128 = 0;

        for item in items.iter() {
            // ── 2. Validate amount ───────────────────────────────────────────
            if item.amount <= 0 || item.amount < min_amount {
                return Err(Error::AmountMismatch);
            }

            // ── 3. Validate encrypted note length ────────────────────────────
            if item.encrypted_note.len() != ENCRYPTED_NOTE_LEN {
                return Err(Error::InvalidNote);
            }

            // ── 4. Validate public inputs match call params ──────────────────
            if item.shield_pub.pub_value != item.amount {
                return Err(Error::AmountMismatch);
            }
            if item.shield_pub.pub_asset_id != asset {
                return Err(Error::AssetMismatch);
            }

            // ── 5. Verify commitment matches Poseidon2 re-computation ────────
            let computed = compute_commitment(&env, item.amount, &asset, &item.rho, &item.rcm, &mut hasher);
            let provided: [u8; 32] = item.commitment.clone().into();
            if computed != provided {
                return Err(Error::CommitmentMismatch);
            }

            // ── 6. Duplicate commitment check ────────────────────────────────
            let seen_key = StorageKey::CommitmentSeen(item.commitment.clone());
            if env.storage().persistent().has(&seen_key) {
                return Err(Error::DuplicateCommitment);
            }

            // ── 7. Groth16 proof verification ────────────────────────────────
            let mut value_bytes = [0u8; 32];
            value_bytes[..16].copy_from_slice(&(item.amount as u128).to_le_bytes());
            let asset_bytes = address_to_field_bytes(&env, &asset);
            let public_inputs = Vec::from_array(
                &env,
                [
                    item.commitment.clone(),
                    item.shield_pub.value_commit.clone(),
                    BytesN::from_array(&env, &value_bytes),
                    BytesN::from_array(&env, &asset_bytes),
                ],
            );
            let proof_ok = VerifierClient::new(&env, &verifier).verify(
                &CircuitType::Shield,
                &public_inputs,
                &item.shield_proof,
            );
            if !proof_ok {
                return Err(Error::InvalidProof);
            }

            // ── 7.5. Merkle tree capacity check ──────────────────────────────
            if merkle::is_full(&env) {
                return Err(Error::MerkleTreeFull);
            }

            // ── 8. Effects: record commitment, insert into tree ──────────────
            env.storage().persistent().set(&seen_key, &true);
            env.storage().persistent().extend_ttl(&seen_key, 17_280 * 30, 17_280 * 365);

            let leaf_index = merkle::insert(&env, item.commitment.clone(), &mut hasher);
            leaf_indices.push_back(leaf_index);

            env.events().publish(
                (symbol_short!("zkella"), symbol_short!("shield")),
                ShieldEvent {
                    leaf_index,
                    asset:      asset.clone(),
                    commitment: item.commitment.clone(),
                },
            );
            env.events().publish(
                (symbol_short!("zkella"), symbol_short!("note")),
                NoteCommitmentEvent {
                    leaf_index,
                    commitment:     item.commitment.clone(),
                    encrypted_note: item.encrypted_note.clone(),
                },
            );

            total_amount = total_amount.checked_add(item.amount).ok_or(Error::AmountMismatch)?;
        }

        let prev_supply: i128 = env
            .storage()
            .instance()
            .get(&StorageKey::ShieldedSupply(asset.clone()))
            .unwrap_or(0);
        let new_supply = prev_supply
            .checked_add(total_amount)
            .ok_or(Error::AmountMismatch)?;
        env.storage()
            .instance()
            .set(&StorageKey::ShieldedSupply(asset.clone()), &new_supply);

        env.storage().instance().extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);

        // ── 9. Interaction: one aggregated transfer for the whole batch ─────
        let token_client = token::Client::new(&env, &asset);
        token_client.transfer(&from, &env.current_contract_address(), &total_amount);

        Ok(leaf_indices)
    }

    // ── Transfer ──────────────────────────────────────────────────────────────

    /// Private note-to-note transfer: spends `nullifiers` (proven owned via a
    /// zero-knowledge proof) and creates `commitments` as new notes. Fixed
    /// 2-in-2-out arity, matching `circuits/transfer_2in2out/transfer.circom`
    /// — see `transfer4()` for the 4-in-4-out circuit.
    ///
    /// No `require_auth()` on any note owner: authorization is the proof
    /// itself — only someone who knows a note's spending key can derive its
    /// nullifier and construct a valid proof against it, which is the whole
    /// point of a note-based (not account-based) shielded pool. Any account
    /// can submit the underlying transaction (e.g. a relayer).
    ///
    /// Public input order matches the circuit's `component main {public
    /// [anchor, nullifiers, out_commitments, in_value_commits,
    /// out_value_commits, fee, asset_id]}`.
    pub fn transfer(
        env:             Env,
        nullifiers:      Vec<BytesN<32>>,
        commitments:     Vec<BytesN<32>>,
        encrypted_notes: Vec<Bytes>,
        proof:           Bytes,
        pub_inputs:      TransferPublicInputs,
    ) -> Result<Vec<u32>, Error> {
        Self::transfer_internal(env, 2, CircuitType::Transfer, nullifiers, commitments, encrypted_notes, proof, pub_inputs)
    }

    /// Same as `transfer()`, against `circuits/transfer_4in4out/transfer.circom`
    /// (4-in-4-out) instead of the 2-in-2-out circuit. Shares the same
    /// `TransferPublicInputs` shape (its `Vec` fields aren't fixed-size) and
    /// the same security properties — see `transfer()`'s doc comment and
    /// `transfer_internal`'s implementation.
    pub fn transfer4(
        env:             Env,
        nullifiers:      Vec<BytesN<32>>,
        commitments:     Vec<BytesN<32>>,
        encrypted_notes: Vec<Bytes>,
        proof:           Bytes,
        pub_inputs:      TransferPublicInputs,
    ) -> Result<Vec<u32>, Error> {
        Self::transfer_internal(env, 4, CircuitType::Transfer4x4, nullifiers, commitments, encrypted_notes, proof, pub_inputs)
    }

    fn transfer_internal(
        env:             Env,
        n:               u32,
        circuit:         CircuitType,
        nullifiers:      Vec<BytesN<32>>,
        commitments:     Vec<BytesN<32>>,
        encrypted_notes: Vec<Bytes>,
        proof:           Bytes,
        pub_inputs:      TransferPublicInputs,
    ) -> Result<Vec<u32>, Error> {
        Self::assert_not_paused(&env)?;

        if pub_inputs.fee < 0 {
            return Err(Error::AmountMismatch);
        }

        // ── 1. Arity checks ───────────────────────────────────────────────────
        if nullifiers.len() != n || commitments.len() != n || encrypted_notes.len() != n {
            return Err(Error::InvalidInputCount);
        }
        if pub_inputs.nullifiers.len() != n
            || pub_inputs.out_commitments.len() != n
            || pub_inputs.in_value_commits.len() != n
            || pub_inputs.out_value_commits.len() != n
        {
            return Err(Error::InvalidInputCount);
        }

        // ── 2. Public inputs must match the call's actual parameters ─────────
        for i in 0..n {
            if nullifiers.get(i).unwrap() != pub_inputs.nullifiers.get(i).unwrap() {
                return Err(Error::CommitmentMismatch);
            }
            if commitments.get(i).unwrap() != pub_inputs.out_commitments.get(i).unwrap() {
                return Err(Error::CommitmentMismatch);
            }
        }

        // ── 3. Anchor must be a recent Merkle root ────────────────────────────
        // Accepts the current root or any of the last `ROOT_HISTORY_SIZE - 1`
        // roots before it, not only an exact match against the current root
        // — see `merkle::is_known_root`'s doc comment for why strict equality
        // made legitimate proofs fail whenever unrelated activity (even on a
        // different asset) landed first.
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        if !merkle::is_known_root(&env, &pub_inputs.anchor, &mut hasher) {
            return Err(Error::InvalidAnchor);
        }

        // ── 4. Nullifiers (and output commitments) must be pairwise distinct
        // within this call. Without this, the same real note could be
        // supplied as both input slots (same rho => same nullifier in both
        // positions): the circuit constrains each slot independently with no
        // cross-slot distinctness check, so sum_in would double-count a
        // single note's value, letting a holder of value V mint 2V-fee in
        // fresh output notes from one real note. The loop below closes that
        // at the contract boundary — the actual enforcement point, since the
        // "unspent" check after this only looks at already-persisted state
        // and can't see duplicates within the same call.
        for i in 0..n {
            for j in (i + 1)..n {
                if nullifiers.get(i).unwrap() == nullifiers.get(j).unwrap() {
                    return Err(Error::DuplicateInputInCall);
                }
                if commitments.get(i).unwrap() == commitments.get(j).unwrap() {
                    return Err(Error::DuplicateInputInCall);
                }
            }
        }

        // ── 5. Nullifiers must be unspent ─────────────────────────────────────
        for i in 0..n {
            let nf = nullifiers.get(i).unwrap();
            if env.storage().persistent().has(&StorageKey::Nullifier(nf)) {
                return Err(Error::NullifierSpent);
            }
        }

        // ── 6. Output commitments must not already exist (replay / pollution) ──
        for i in 0..n {
            let cm = commitments.get(i).unwrap();
            if env.storage().persistent().has(&StorageKey::CommitmentSeen(cm)) {
                return Err(Error::DuplicateCommitment);
            }
        }

        // ── 7. Groth16 proof verification ─────────────────────────────────────
        let verifier: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Verifier)
            .ok_or(Error::NotInitialized)?;

        let mut public_inputs = Vec::new(&env);
        public_inputs.push_back(pub_inputs.anchor.clone());
        for i in 0..n { public_inputs.push_back(pub_inputs.nullifiers.get(i).unwrap()); }
        for i in 0..n { public_inputs.push_back(pub_inputs.out_commitments.get(i).unwrap()); }
        for i in 0..n { public_inputs.push_back(pub_inputs.in_value_commits.get(i).unwrap()); }
        for i in 0..n { public_inputs.push_back(pub_inputs.out_value_commits.get(i).unwrap()); }
        let mut fee_bytes = [0u8; 32];
        fee_bytes[..16].copy_from_slice(&(pub_inputs.fee as u128).to_le_bytes());
        public_inputs.push_back(BytesN::from_array(&env, &fee_bytes));
        public_inputs.push_back(BytesN::from_array(&env, &address_to_field_bytes(&env, &pub_inputs.asset_id)));

        let proof_ok = VerifierClient::new(&env, &verifier).verify(
            &circuit,
            &public_inputs,
            &proof,
        );
        if !proof_ok {
            return Err(Error::InvalidProof);
        }

        // ── 7.5. Merkle tree capacity check ──────────────────────────────────
        // This call inserts `n` output commitments; `is_full` only checks the
        // immediate next slot, but at `MAX_LEAVES = u32::MAX` a false negative
        // here (accepting a call that would overflow mid-loop) is not a
        // practical concern for any real value of `n`.
        if merkle::is_full(&env) {
            return Err(Error::MerkleTreeFull);
        }

        // ── 8. Effects: mark nullifiers spent, insert output commitments ─────
        for i in 0..n {
            let nf = nullifiers.get(i).unwrap();
            let nf_key = StorageKey::Nullifier(nf.clone());
            env.storage().persistent().set(&nf_key, &true);
            env.storage().persistent().extend_ttl(&nf_key, 17_280 * 30, 17_280 * 365);
            env.events().publish(
                (symbol_short!("zkella"), symbol_short!("nf")),
                NullifierEvent { nullifier: nf },
            );
        }

        let mut leaf_indices = Vec::new(&env);
        for i in 0..n {
            let cm = commitments.get(i).unwrap();
            let seen_key = StorageKey::CommitmentSeen(cm.clone());
            env.storage().persistent().set(&seen_key, &true);
            env.storage().persistent().extend_ttl(&seen_key, 17_280 * 30, 17_280 * 365);

            let leaf_index = merkle::insert(&env, cm.clone(), &mut hasher);
            leaf_indices.push_back(leaf_index);

            env.events().publish(
                (symbol_short!("zkella"), symbol_short!("note")),
                NoteCommitmentEvent {
                    leaf_index,
                    commitment: cm,
                    encrypted_note: encrypted_notes.get(i).unwrap(),
                },
            );
        }

        env.storage().instance().extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);

        Ok(leaf_indices)
    }

    // ── Unshield ──────────────────────────────────────────────────────────────

    /// Move a note from the shielded pool back to a public address `to`.
    ///
    /// `pub_inputs.recipient_hash` must equal `Poseidon2(address_field(to), binding_tag)`.
    /// This binding is deliberately NOT enforced by the circuit itself (see
    /// `circuits/unshield/unshield.circom`'s comment — it's included as a
    /// public input but unconstrained there), so the contract checks it here.
    /// This is the authoritative definition of that binding — the wallet/SDK's
    /// unshield() implementation (`sdk/src/wallet/wallet.ts`) must compute
    /// `recipient_hash` exactly this way.
    ///
    /// `binding_tag` exists so a caller can cryptographically bind this
    /// specific proof to more than just `to` — critically used by
    /// `contracts/swap::commit_swap`, which reuses this function as its
    /// note-ownership proof and passes `binding_tag =
    /// Poseidon2(intent_commitment, refund_to)`. Without this, the proof was
    /// bound only to (nullifier, amount, asset, `to` = the swap contract's
    /// own fixed address) — identical for every user and every swap, so
    /// anyone who observed a submitted-but-not-yet-final `commit_swap`
    /// transaction (e.g. via a failed/retried submission visible in public
    /// transaction history) could resubmit the exact same proof bytes with
    /// their *own* `refund_to`, stealing the escrowed value once the swap
    /// expired. A direct (non-swap) unshield passes `binding_tag =
    /// [0u8; 32]`, preserving the original `Poseidon2(address_field(to), 0)`
    /// formula exactly.
    pub fn unshield(
        env:         Env,
        nullifier:   BytesN<32>,
        to:          Address,
        binding_tag: BytesN<32>,
        proof:       Bytes,
        pub_inputs:  UnshieldPublicInputs,
    ) -> Result<(), Error> {
        Self::assert_not_paused(&env)?;

        // ── 1. Public inputs must match the call's actual parameters ─────────
        if pub_inputs.nullifier != nullifier {
            return Err(Error::CommitmentMismatch);
        }

        // ── 2. recipient_hash binds `to` (and, via binding_tag, the caller's
        // own additional context — see this function's doc comment) ─────────
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let to_field = address_to_field_bytes(&env, &to);
        let binding_tag_bytes: [u8; 32] = binding_tag.into();
        let expected_recipient_hash = hasher.hash(&to_field, &binding_tag_bytes);
        let provided_recipient_hash: [u8; 32] = pub_inputs.recipient_hash.clone().into();
        if expected_recipient_hash != provided_recipient_hash {
            return Err(Error::RecipientMismatch);
        }

        // ── 3. Anchor must be a recent Merkle root ────────────────────────────
        // See `transfer_internal`'s identical check and `merkle::is_known_root`
        // for why this accepts a small window of recent roots rather than
        // only the exact current one.
        if !merkle::is_known_root(&env, &pub_inputs.anchor, &mut hasher) {
            return Err(Error::InvalidAnchor);
        }

        // ── 4. Nullifier must be unspent ──────────────────────────────────────
        let nf_key = StorageKey::Nullifier(nullifier.clone());
        if env.storage().persistent().has(&nf_key) {
            return Err(Error::NullifierSpent);
        }

        // ── 5. Amount validity ─────────────────────────────────────────────────
        if pub_inputs.pub_value <= 0 {
            return Err(Error::AmountMismatch);
        }

        // ── 6. Groth16 proof verification ─────────────────────────────────────
        let verifier: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Verifier)
            .ok_or(Error::NotInitialized)?;

        let mut value_bytes = [0u8; 32];
        value_bytes[..16].copy_from_slice(&(pub_inputs.pub_value as u128).to_le_bytes());
        let asset_bytes = address_to_field_bytes(&env, &pub_inputs.pub_asset_id);

        let public_inputs = Vec::from_array(
            &env,
            [
                pub_inputs.anchor.clone(),
                nullifier.clone(),
                BytesN::from_array(&env, &value_bytes),
                BytesN::from_array(&env, &asset_bytes),
                pub_inputs.recipient_hash.clone(),
            ],
        );

        let proof_ok = VerifierClient::new(&env, &verifier).verify(
            &CircuitType::Unshield,
            &public_inputs,
            &proof,
        );
        if !proof_ok {
            return Err(Error::InvalidProof);
        }

        // ── 7. Effects: mark nullifier spent, update shielded supply ─────────
        env.storage().persistent().set(&nf_key, &true);
        env.storage().persistent().extend_ttl(&nf_key, 17_280 * 30, 17_280 * 365);

        // shield() increments ShieldedSupply on deposit; unshield() must
        // decrement it symmetrically on withdrawal, or shielded_supply()
        // permanently overstates the contract's real token backing after any
        // unshield. i128 is signed, so `checked_sub` alone only catches
        // actual type-level overflow — it happily returns a negative i128,
        // which is a "valid" value but violates the business invariant that
        // supply can't go negative. Check that explicitly instead.
        let prev_supply: i128 = env
            .storage()
            .instance()
            .get(&StorageKey::ShieldedSupply(pub_inputs.pub_asset_id.clone()))
            .unwrap_or(0);
        if pub_inputs.pub_value > prev_supply {
            return Err(Error::AmountMismatch);
        }
        let new_supply = prev_supply
            .checked_sub(pub_inputs.pub_value)
            .ok_or(Error::AmountMismatch)?;
        env.storage()
            .instance()
            .set(&StorageKey::ShieldedSupply(pub_inputs.pub_asset_id.clone()), &new_supply);

        env.storage().instance().extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("nf")),
            NullifierEvent { nullifier: nullifier.clone() },
        );
        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("unshield")),
            UnshieldEvent {
                to:     to.clone(),
                amount: pub_inputs.pub_value,
                asset:  pub_inputs.pub_asset_id.clone(),
            },
        );

        // ── 8. Interaction: release public tokens (last, checks-effects-interactions) ──
        let token_client = token::Client::new(&env, &pub_inputs.pub_asset_id);
        token_client.transfer(&env.current_contract_address(), &to, &pub_inputs.pub_value);

        Ok(())
    }

    // ── Read-only queries ─────────────────────────────────────────────────────

    /// Current Merkle root of the note commitment tree.
    pub fn merkle_root(env: Env) -> BytesN<32> {
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        merkle::root(&env, &mut hasher)
    }

    /// Returns true if a nullifier has been spent.
    pub fn is_spent(env: Env, nullifier: BytesN<32>) -> bool {
        env.storage()
            .persistent()
            .has(&StorageKey::Nullifier(nullifier))
    }

    /// Total shielded supply of a given asset.
    pub fn shielded_supply(env: Env, asset: Address) -> i128 {
        env.storage()
            .instance()
            .get(&StorageKey::ShieldedSupply(asset))
            .unwrap_or(0)
    }

    /// Merkle authentication path for a leaf, used as circuit witness.
    pub fn merkle_path(env: Env, leaf_index: u32) -> Vec<BytesN<32>> {
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        merkle::get_path(&env, leaf_index, &mut hasher)
    }

    /// Total number of shielded notes ever created.
    pub fn leaf_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&StorageKey::NextLeafIndex)
            .unwrap_or(0)
    }

    // ── Admin ─────────────────────────────────────────────────────────────────

    pub fn pause(env: Env) -> Result<(), Error> {
        Self::require_admin(&env)?;
        env.storage().instance().set(&StorageKey::Paused, &true);
        Ok(())
    }

    pub fn unpause(env: Env) -> Result<(), Error> {
        Self::require_admin(&env)?;
        env.storage().instance().set(&StorageKey::Paused, &false);
        Ok(())
    }

    /// Initiate an admin transfer. The new admin must call `accept_admin` to complete it.
    ///
    /// Two-step transfer prevents locking the contract to an uncontrolled address.
    /// For mainnet, the admin should be a multisig contract (e.g. a Soroban multisig
    /// or a DAO governance contract) rather than a single keypair.
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        Self::require_admin(&env)?;
        env.storage()
            .instance()
            .set(&StorageKey::PendingAdmin, &new_admin);
        Ok(())
    }

    /// Complete the admin transfer initiated by the current admin.
    /// Must be called by the `new_admin` address to confirm acceptance.
    pub fn accept_admin(env: Env) -> Result<(), Error> {
        let pending: Address = env
            .storage()
            .instance()
            .get(&StorageKey::PendingAdmin)
            .ok_or(Error::NotInitialized)?;
        pending.require_auth();
        env.storage().instance().set(&StorageKey::Admin, &pending);
        env.storage().instance().remove(&StorageKey::PendingAdmin);
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn assert_not_paused(env: &Env) -> Result<(), Error> {
        let paused: bool = env
            .storage()
            .instance()
            .get(&StorageKey::Paused)
            .unwrap_or(false);
        if paused { Err(Error::Paused) } else { Ok(()) }
    }

    fn require_admin(env: &Env) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    /// Filler bytes for public inputs must be canonical field elements (< r).
    fn canon(b: u8) -> [u8; 32] {
        let mut a = [b; 32];
        a[31] = 0;
        a
    }

    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    /// Deploys token alongside a real `zkella-verifier` registry, initialized
    /// with `admin` as its admin (no VK registered yet). Tests that need
    /// `shield()` to actually succeed must register a VK via
    /// `prove_and_register_shield` below; tests that only exercise checks
    /// *before* proof verification (amount, note length, duplicate
    /// commitment) don't need to.
    ///
    /// Explicitly enforces `InvocationResourceLimits::mainnet()` — the SDK's
    /// own snapshot of the *current* real network limits (400M instructions,
    /// as of 2026-07-10) — rather than trusting `Env::default()`'s built-in
    /// budget, which is a much more conservative 100M and does not track the
    /// network. Budget-viability claims are only meaningful against the real
    /// figure, and this is that check: does a real shield() call —
    /// commitment computation, Merkle insert, and a genuine on-chain Groth16
    /// verification — actually fit inside it.
    fn setup() -> (Env, Address, Address, Address) {
        let env      = Env::default();
        // 400M cpu / ~40MB mem: the SDK's own `InvocationResourceLimits::mainnet()`
        // values (snapshot 2026-07-10); reproduced here as literals since
        // that type isn't cleanly importable from this SDK version's public
        // surface. `Env::default()`'s own built-in limit (100M) is far more
        // conservative and doesn't track the real network.
        env.cost_estimate().budget().reset_limits(400_000_000, 41_943_040);
        env.mock_all_auths();
        let admin    = Address::generate(&env);
        let token     = env.register(ShieldedToken, ());
        let verifier = env.register(zkella_verifier::VerifierContract, ());
        zkella_verifier::VerifierContractClient::new(&env, &verifier).initialize(&admin);
        (env, admin, token, verifier)
    }

    /// Builds a genuinely valid (if synthetic — see test_groth16.rs) Groth16
    /// proof for the given shield public inputs, registers its VK on
    /// `verifier` for `CircuitType::Shield`, and returns the proof bytes to
    /// pass to `shield()`.
    fn prove_and_register_shield(
        env:          &Env,
        verifier:     &Address,
        commitment:   &BytesN<32>,
        value_commit: &BytesN<32>,
        amount:       i128,
        asset:        &Address,
    ) -> Bytes {
        let mut value_bytes = [0u8; 32];
        value_bytes[..16].copy_from_slice(&(amount as u128).to_le_bytes());
        let asset_bytes = address_to_field_bytes(env, asset);

        let public_inputs_le: [[u8; 32]; 4] = [
            commitment.clone().into(),
            value_commit.clone().into(),
            value_bytes,
            asset_bytes,
        ];

        let (vk_bytes, proof_bytes) = test_groth16::build_valid_shield_proof(env, public_inputs_le);

        let verifier_client = zkella_verifier::VerifierContractClient::new(env, verifier);
        verifier_client.register_verifying_key(&CircuitType::Shield.into(), &vk_bytes);

        proof_bytes
    }

    #[test]
    fn initialize_sets_admin_and_root() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);

        client.initialize(&admin, &verifier);

        let root = client.merkle_root();
        assert_ne!(root, BytesN::from_array(&env, &[0u8; 32]));
    }

    #[test]
    #[should_panic(expected = "already initialized")]
    fn initialize_cannot_be_called_twice() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);
        client.initialize(&admin, &verifier);
    }

    #[test]
    fn merkle_root_changes_after_shield() {
        let (env, admin, token, verifier) = setup();
        env.mock_all_auths();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let root_before = client.merkle_root();

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin.clone());
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);

        use soroban_sdk::testutils::Ledger;
        env.ledger().with_mut(|li| { li.sequence_number = 100; });

        let user = Address::generate(&env);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&user, &1_000_000_000);

        let rho = BytesN::from_array(&env, &canon(1));
        let rcm = BytesN::from_array(&env, &canon(2));

        // Compute commitment using the same function the contract will call
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let computed = compute_commitment(&env, 100_000_000, &token_addr, &rho, &rcm, &mut hasher);
        let commitment = BytesN::from_array(&env, &computed);
        let value_commit = BytesN::from_array(&env, &[0u8; 32]);

        let pub_inputs = ShieldPublicInputs {
            commitment:   commitment.clone(),
            value_commit: value_commit.clone(),
            pub_value:    100_000_000,
            pub_asset_id: token_addr.clone(),
        };

        let proof = prove_and_register_shield(
            &env, &verifier, &commitment, &value_commit, 100_000_000, &token_addr,
        );

        // Encrypted note stub: must be exactly ENCRYPTED_NOTE_LEN bytes
        let mut enc_bytes = [0u8; 176];
        enc_bytes[0] = 0xde; enc_bytes[1] = 0xad; // recognizable marker
        let encrypted_note = Bytes::from_array(&env, &enc_bytes);

        let leaf = client.shield(
            &user,
            &token_addr,
            &100_000_000i128,
            &rho,
            &rcm,
            &commitment,
            &encrypted_note,
            &proof,
            &pub_inputs,
        );

        assert_eq!(leaf, 0u32);

        let root_after = client.merkle_root();
        assert_ne!(root_before, root_after);

        let supply = client.shielded_supply(&token_addr);
        assert_eq!(supply, 100_000_000i128);

        assert_eq!(client.leaf_count(), 1u32);
    }

    /// Shields a fixed amount of a fresh SEP-41 asset from a fresh user, `n`
    /// times in a row, each with a distinct `rho` (so each commitment, and
    /// therefore the Merkle root, differs). Returns the root recorded right
    /// after the *first* shield (leaf 0) — the anchor the root-history-window
    /// tests below check against — plus the token address, in case a caller
    /// wants to shield further with the same asset/user.
    fn shield_n_times(
        env:      &Env,
        client:   &ShieldedTokenClient,
        verifier: &Address,
        n:        u32,
    ) -> (BytesN<32>, Address) {
        env.mock_all_auths();
        let token_admin = Address::generate(env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);

        let user = Address::generate(env);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(env, &token_addr);
        stellar_asset.mint(&user, &(1_000_000_000i128 * (n as i128 + 1)));

        let amount = 100_000_000i128;
        let mut first_root: Option<BytesN<32>> = None;

        for i in 0..n {
            let rho = BytesN::from_array(env, &[i as u8; 32]);
            let rcm = BytesN::from_array(env, &[0x99u8; 32]);

            let mut hasher = poseidon::Poseidon2Hasher::new(env);
            let computed = compute_commitment(env, amount, &token_addr, &rho, &rcm, &mut hasher);
            let commitment = BytesN::from_array(env, &computed);
            let value_commit = BytesN::from_array(env, &[0u8; 32]);

            let pub_inputs = ShieldPublicInputs {
                commitment:   commitment.clone(),
                value_commit: value_commit.clone(),
                pub_value:    amount,
                pub_asset_id: token_addr.clone(),
            };

            // Each iteration's synthetic proof is only valid against its own
            // freshly-derived VK (commitment differs every time via `rho`).
            // `register_verifying_key` only succeeds once per circuit, so
            // every iteration after the first must `update_verifying_key`
            // instead — fine here since no two shields in this loop need to
            // verify against the same VK simultaneously.
            let mut value_bytes = [0u8; 32];
            value_bytes[..16].copy_from_slice(&(amount as u128).to_le_bytes());
            let asset_bytes = address_to_field_bytes(env, &token_addr);
            let public_inputs_le: [[u8; 32]; 4] = [
                commitment.clone().into(),
                value_commit.clone().into(),
                value_bytes,
                asset_bytes,
            ];
            let (vk_bytes, proof) = test_groth16::build_valid_shield_proof(env, public_inputs_le);
            let verifier_client = zkella_verifier::VerifierContractClient::new(env, verifier);
            if i == 0 {
                verifier_client.register_verifying_key(&CircuitType::Shield.into(), &vk_bytes);
            } else {
                verifier_client.update_verifying_key(&CircuitType::Shield.into(), &vk_bytes);
            }
            let encrypted_note = Bytes::from_array(env, &[0u8; 176]);

            client.shield(
                &user, &token_addr, &amount, &rho, &rcm, &commitment,
                &encrypted_note, &proof, &pub_inputs,
            );

            if i == 0 {
                first_root = Some(client.merkle_root());
            }
        }

        (first_root.expect("n must be >= 1"), token_addr)
    }

    #[test]
    fn transfer_accepts_anchor_still_within_root_history_window() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        // Exactly ROOT_HISTORY_SIZE shields land after (and including) the
        // one whose resulting root we anchor to below — the anchor is still
        // the oldest entry in the history window, not yet evicted.
        let (anchor, asset) = shield_n_times(&env, &client, &verifier, merkle::ROOT_HISTORY_SIZE);

        let nullifiers = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(201)),
            BytesN::from_array(&env, &canon(202)),
        ]);
        let out_commitments = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(203)),
            BytesN::from_array(&env, &canon(204)),
        ]);
        let zero_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);
        let fee = 0i128;
        let pub_inputs = TransferPublicInputs {
            anchor: anchor.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments.clone(),
            in_value_commits: zero_commits.clone(),
            out_value_commits: zero_commits.clone(),
            fee,
            asset_id: asset.clone(),
        };
        let mut fee_bytes = [0u8; 32];
        fee_bytes[..16].copy_from_slice(&(fee as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 11] = [
            anchor.clone().into(),
            nullifiers.get(0).unwrap().into(),
            nullifiers.get(1).unwrap().into(),
            out_commitments.get(0).unwrap().into(),
            out_commitments.get(1).unwrap().into(),
            zero_commits.get(0).unwrap().into(),
            zero_commits.get(1).unwrap().into(),
            zero_commits.get(0).unwrap().into(),
            zero_commits.get(1).unwrap().into(),
            fee_bytes,
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Transfer.into(), &vk_bytes);
        let encrypted_notes = Vec::from_array(&env, [
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
        ]);

        // Must succeed: `anchor` (root after leaf 0) is still present in the
        // history window after exactly ROOT_HISTORY_SIZE total insertions.
        let leaf_indices = client.transfer(&nullifiers, &out_commitments, &encrypted_notes, &proof, &pub_inputs);
        assert_eq!(leaf_indices.len(), 2);
    }

    #[test]
    fn transfer_rejects_anchor_evicted_from_root_history_window() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        // ROOT_HISTORY_SIZE + 1 total insertions: the (ROOT_HISTORY_SIZE+1)-th
        // shield evicts leaf 0's root from the history window.
        let (anchor, asset) = shield_n_times(&env, &client, &verifier, merkle::ROOT_HISTORY_SIZE + 1);

        let nullifiers = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(211)),
            BytesN::from_array(&env, &canon(212)),
        ]);
        let out_commitments = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(213)),
            BytesN::from_array(&env, &canon(214)),
        ]);
        let zero_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);
        let fee = 0i128;
        let pub_inputs = TransferPublicInputs {
            anchor: anchor.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments.clone(),
            in_value_commits: zero_commits.clone(),
            out_value_commits: zero_commits.clone(),
            fee,
            asset_id: asset.clone(),
        };
        let mut fee_bytes = [0u8; 32];
        fee_bytes[..16].copy_from_slice(&(fee as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 11] = [
            anchor.clone().into(),
            nullifiers.get(0).unwrap().into(),
            nullifiers.get(1).unwrap().into(),
            out_commitments.get(0).unwrap().into(),
            out_commitments.get(1).unwrap().into(),
            zero_commits.get(0).unwrap().into(),
            zero_commits.get(1).unwrap().into(),
            zero_commits.get(0).unwrap().into(),
            zero_commits.get(1).unwrap().into(),
            fee_bytes,
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Transfer.into(), &vk_bytes);
        let encrypted_notes = Vec::from_array(&env, [
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
        ]);

        // Must fail: `anchor` (root after leaf 0) has aged out of the
        // ROOT_HISTORY_SIZE window by the time this call lands.
        let result = client.try_transfer(&nullifiers, &out_commitments, &encrypted_notes, &proof, &pub_inputs);
        assert_eq!(result, Err(Ok(Error::InvalidAnchor)));
    }

    #[test]
    fn shield_rejects_invalid_proof() {
        // Same commitment/public inputs as a real shield, but the proof
        // registered is for a *different* VK — must be rejected by the
        // verifier, not silently accepted.
        let (env, admin, token, verifier) = setup();
        env.mock_all_auths();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);
        let user        = Address::generate(&env);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&user, &1_000_000_000);

        let rho = BytesN::from_array(&env, &canon(7));
        let rcm = BytesN::from_array(&env, &canon(8));
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let computed   = compute_commitment(&env, 1_000, &token_addr, &rho, &rcm, &mut hasher);
        let commitment = BytesN::from_array(&env, &computed);
        let value_commit = BytesN::from_array(&env, &[0u8; 32]);
        let enc = Bytes::from_array(&env, &[0u8; 176]);
        let pub_inputs = ShieldPublicInputs {
            commitment:   commitment.clone(),
            value_commit: value_commit.clone(),
            pub_value:    1_000,
            pub_asset_id: token_addr.clone(),
        };

        // Register a real VK for these public inputs, but build a
        // deliberately non-matching proof to submit instead.
        let public_inputs_le: [[u8; 32]; 4] = [
            commitment.clone().into(),
            value_commit.clone().into(),
            {
                let mut v = [0u8; 32];
                v[..16].copy_from_slice(&1_000u128.to_le_bytes());
                v
            },
            address_to_field_bytes(&env, &token_addr),
        ];
        let (vk_bytes, valid_proof) = test_groth16::build_valid_shield_proof(&env, public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Shield.into(), &vk_bytes);
        let bad_proof = test_groth16::corrupt_proof(&env, &valid_proof);

        let result = client.try_shield(&user, &token_addr, &1_000i128, &rho, &rcm, &commitment, &enc, &bad_proof, &pub_inputs);
        assert!(result.is_err());
    }

    #[test]
    fn shield_rejects_negative_amount() {
        let (env, admin, token, verifier) = setup();
        env.mock_all_auths();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);
        let user        = Address::generate(&env);

        let rho = BytesN::from_array(&env, &[0u8; 32]);
        let rcm = BytesN::from_array(&env, &[0u8; 32]);
        let cm  = BytesN::from_array(&env, &[0u8; 32]);
        let enc = Bytes::from_array(&env, &[0u8; 176]);
        let pub_inputs = ShieldPublicInputs {
            commitment:   cm.clone(),
            value_commit: BytesN::from_array(&env, &[0u8; 32]),
            pub_value:    -1,
            pub_asset_id: token_addr.clone(),
        };

        // Negative amount is rejected before proof verification is reached,
        // so an empty/garbage proof is fine here.
        let result = client.try_shield(&user, &token_addr, &-1i128, &rho, &rcm, &cm, &enc, &Bytes::new(&env), &pub_inputs);
        assert!(result.is_err());
    }

    #[test]
    fn shield_rejects_duplicate_commitment() {
        let (env, admin, token, verifier) = setup();
        env.mock_all_auths();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);

        use soroban_sdk::testutils::Ledger;
        env.ledger().with_mut(|li| { li.sequence_number = 100; });

        let user = Address::generate(&env);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&user, &1_000_000_000);

        let rho = BytesN::from_array(&env, &canon(3));
        let rcm = BytesN::from_array(&env, &canon(4));
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let computed   = compute_commitment(&env, 1_000, &token_addr, &rho, &rcm, &mut hasher);
        let commitment = BytesN::from_array(&env, &computed);
        let value_commit = BytesN::from_array(&env, &[0u8; 32]);
        let enc        = Bytes::from_array(&env, &[0u8; 176]);
        let pub_inputs = ShieldPublicInputs {
            commitment:   commitment.clone(),
            value_commit: value_commit.clone(),
            pub_value:    1_000,
            pub_asset_id: token_addr.clone(),
        };

        let proof = prove_and_register_shield(
            &env, &verifier, &commitment, &value_commit, 1_000, &token_addr,
        );

        // First shield succeeds
        client.shield(&user, &token_addr, &1_000i128, &rho, &rcm, &commitment, &enc, &proof, &pub_inputs);

        // Second shield with same commitment must fail at the duplicate
        // check, before proof verification is reached again.
        stellar_asset.mint(&user, &1_000_000_000);
        let result = client.try_shield(&user, &token_addr, &1_000i128, &rho, &rcm, &commitment, &enc, &proof, &pub_inputs);
        assert!(result.is_err());
    }

    /// Regression test for a vulnerability class found in review: shield's
    /// public inputs (commitment, value_commit, pub_value, pub_asset_id)
    /// contain nothing recipient- or caller-specific, unlike transfer/unshield
    /// which bind a nullifier to a specific spent note. That shape alone is
    /// what let an unrelated party replay another operation's public inputs
    /// elsewhere in a different confidential-token design this project
    /// compared notes with. Here, a second, unrelated address observes user
    /// A's already-submitted (rho, rcm, commitment, proof) and tries to
    /// replay the exact same tuple under its own authorization and its own
    /// funds. It must be rejected by the duplicate-commitment check — keyed
    /// on the commitment itself, not on the caller — regardless of who
    /// submits it or whose funds back it.
    #[test]
    fn shield_replay_by_different_caller_rejected_at_duplicate_check() {
        let (env, admin, token, verifier) = setup();
        env.mock_all_auths();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);

        use soroban_sdk::testutils::Ledger;
        env.ledger().with_mut(|li| { li.sequence_number = 100; });

        let user_a = Address::generate(&env);
        let attacker = Address::generate(&env);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&user_a, &1_000_000_000);
        stellar_asset.mint(&attacker, &1_000_000_000);

        let rho = BytesN::from_array(&env, &canon(3));
        let rcm = BytesN::from_array(&env, &canon(4));
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let computed   = compute_commitment(&env, 1_000, &token_addr, &rho, &rcm, &mut hasher);
        let commitment = BytesN::from_array(&env, &computed);
        let value_commit = BytesN::from_array(&env, &[0u8; 32]);
        let enc        = Bytes::from_array(&env, &[0u8; 176]);
        let pub_inputs = ShieldPublicInputs {
            commitment:   commitment.clone(),
            value_commit: value_commit.clone(),
            pub_value:    1_000,
            pub_asset_id: token_addr.clone(),
        };

        let proof = prove_and_register_shield(
            &env, &verifier, &commitment, &value_commit, 1_000, &token_addr,
        );

        // User A's genuine shield succeeds and funds come from user A.
        client.shield(&user_a, &token_addr, &1_000i128, &rho, &rcm, &commitment, &enc, &proof, &pub_inputs);
        assert_eq!(client.leaf_count(), 1u32);

        // The attacker, a wholly unrelated address, replays the exact same
        // (rho, rcm, commitment, proof) tuple under its own authorization,
        // funded by its own balance. Knowledge of the public tuple confers
        // no ability to claim or duplicate the note.
        let result = client.try_shield(&attacker, &token_addr, &1_000i128, &rho, &rcm, &commitment, &enc, &proof, &pub_inputs);
        assert!(result.is_err(), "a different caller replaying another user's shield tuple must be rejected");
        assert_eq!(client.leaf_count(), 1u32, "no second note may be inserted from the replayed tuple");
    }

    #[test]
    fn shield_rejects_wrong_encrypted_note_length() {
        let (env, admin, token, verifier) = setup();
        env.mock_all_auths();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);
        let user        = Address::generate(&env);

        let rho = BytesN::from_array(&env, &canon(5));
        let rcm = BytesN::from_array(&env, &canon(6));
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let computed   = compute_commitment(&env, 1_000, &token_addr, &rho, &rcm, &mut hasher);
        let commitment = BytesN::from_array(&env, &computed);
        // Wrong length: 136 instead of 176
        let bad_enc    = Bytes::from_array(&env, &[0u8; 136]);
        let pub_inputs = ShieldPublicInputs {
            commitment:   commitment.clone(),
            value_commit: BytesN::from_array(&env, &[0u8; 32]),
            pub_value:    1_000,
            pub_asset_id: token_addr.clone(),
        };

        // Wrong note length is rejected before proof verification, so an
        // empty/garbage proof is fine here.
        let result = client.try_shield(&user, &token_addr, &1_000i128, &rho, &rcm, &commitment, &bad_enc, &Bytes::new(&env), &pub_inputs);
        assert!(result.is_err());
    }

    #[test]
    fn transfer_and_unshield_reject_malformed_input() {
        // transfer(): empty vecs fail the 2-in-2-out arity check.
        // unshield(): recipient_hash=[0;32] won't match Poseidon2(address, 0)
        // for a real generated address. Neither reaches proof verification.
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let transfer_result = client.try_transfer(
            &Vec::new(&env),
            &Vec::new(&env),
            &Vec::new(&env),
            &Bytes::new(&env),
            &types::TransferPublicInputs {
                anchor:            BytesN::from_array(&env, &[0u8; 32]),
                nullifiers:        Vec::new(&env),
                out_commitments:   Vec::new(&env),
                in_value_commits:  Vec::new(&env),
                out_value_commits: Vec::new(&env),
                fee:               0,
                asset_id:          Address::generate(&env),
            },
        );
        assert!(transfer_result.is_err());

        let unshield_result = client.try_unshield(
            &BytesN::from_array(&env, &[0u8; 32]),
            &Address::generate(&env),
            &BytesN::from_array(&env, &[0u8; 32]),
            &Bytes::new(&env),
            &types::UnshieldPublicInputs {
                anchor:         BytesN::from_array(&env, &[0u8; 32]),
                nullifier:      BytesN::from_array(&env, &[0u8; 32]),
                pub_value:      0,
                pub_asset_id:   Address::generate(&env),
                recipient_hash: BytesN::from_array(&env, &[0u8; 32]),
            },
        );
        assert!(unshield_result.is_err());
    }

    #[test]
    fn transfer_succeeds_with_valid_proof() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let asset = Address::generate(&env);
        client.set_asset_approved(&asset, &true);
        // empty-tree root; transfer() doesn't itself verify input-note
        // membership — that's the circuit's job, bypassed by this synthetic
        // proof (see test_groth16.rs doc comment). merkle::root() touches
        // contract storage, so it must be called through the client, not
        // directly from test code.
        let anchor = client.merkle_root();

        let nullifiers = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(11)),
            BytesN::from_array(&env, &canon(12)),
        ]);
        let out_commitments = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(13)),
            BytesN::from_array(&env, &canon(14)),
        ]);
        let in_value_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);
        let out_value_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);
        let fee = 0i128;

        let pub_inputs = TransferPublicInputs {
            anchor: anchor.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments.clone(),
            in_value_commits: in_value_commits.clone(),
            out_value_commits: out_value_commits.clone(),
            fee,
            asset_id: asset.clone(),
        };

        // Public input order matches transfer.circom's public signal list;
        // see transfer()'s own doc comment.
        let mut fee_bytes = [0u8; 32];
        fee_bytes[..16].copy_from_slice(&(fee as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 11] = [
            anchor.clone().into(),
            nullifiers.get(0).unwrap().into(),
            nullifiers.get(1).unwrap().into(),
            out_commitments.get(0).unwrap().into(),
            out_commitments.get(1).unwrap().into(),
            in_value_commits.get(0).unwrap().into(),
            in_value_commits.get(1).unwrap().into(),
            out_value_commits.get(0).unwrap().into(),
            out_value_commits.get(1).unwrap().into(),
            fee_bytes,
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Transfer.into(), &vk_bytes);

        let encrypted_notes = Vec::from_array(&env, [
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
        ]);

        let leaf_indices = client.transfer(&nullifiers, &out_commitments, &encrypted_notes, &proof, &pub_inputs);
        assert_eq!(leaf_indices.len(), 2);
        assert!(client.is_spent(&nullifiers.get(0).unwrap()));
        assert!(client.is_spent(&nullifiers.get(1).unwrap()));
        assert_eq!(client.leaf_count(), 2u32);
    }

    #[test]
    fn transfer4_succeeds_with_valid_proof() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let asset = Address::generate(&env);
        client.set_asset_approved(&asset, &true);
        let anchor = client.merkle_root();

        let nullifiers = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(51)),
            BytesN::from_array(&env, &canon(52)),
            BytesN::from_array(&env, &canon(53)),
            BytesN::from_array(&env, &canon(54)),
        ]);
        let out_commitments = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(55)),
            BytesN::from_array(&env, &canon(56)),
            BytesN::from_array(&env, &canon(57)),
            BytesN::from_array(&env, &canon(58)),
        ]);
        let zero_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);
        let fee = 0i128;

        let pub_inputs = TransferPublicInputs {
            anchor: anchor.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments.clone(),
            in_value_commits: zero_commits.clone(),
            out_value_commits: zero_commits.clone(),
            fee,
            asset_id: asset.clone(),
        };

        // Public input order matches transfer_4in4out's public signal list:
        // anchor(1) + nullifiers(4) + out_commitments(4) + in_value_commits(4)
        // + out_value_commits(4) + fee(1) + asset_id(1) = 19.
        let mut fee_bytes = [0u8; 32];
        fee_bytes[..16].copy_from_slice(&(fee as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 19] = [
            anchor.clone().into(),
            nullifiers.get(0).unwrap().into(),
            nullifiers.get(1).unwrap().into(),
            nullifiers.get(2).unwrap().into(),
            nullifiers.get(3).unwrap().into(),
            out_commitments.get(0).unwrap().into(),
            out_commitments.get(1).unwrap().into(),
            out_commitments.get(2).unwrap().into(),
            out_commitments.get(3).unwrap().into(),
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            fee_bytes,
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Transfer4x4.into(), &vk_bytes);

        let encrypted_notes = Vec::from_array(&env, [
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
        ]);

        let leaf_indices = client.transfer4(&nullifiers, &out_commitments, &encrypted_notes, &proof, &pub_inputs);
        assert_eq!(leaf_indices.len(), 4);
        for i in 0..4 {
            assert!(client.is_spent(&nullifiers.get(i).unwrap()));
        }
        assert_eq!(client.leaf_count(), 4u32);
    }

    #[test]
    fn transfer4_rejects_duplicate_nullifier_in_same_call() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let asset = Address::generate(&env);
        client.set_asset_approved(&asset, &true);
        let anchor = client.merkle_root();

        let same_nullifier = BytesN::from_array(&env, &canon(77));
        let nullifiers = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(61)),
            same_nullifier.clone(),
            BytesN::from_array(&env, &canon(62)),
            same_nullifier.clone(),
        ]);
        let out_commitments = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(63)),
            BytesN::from_array(&env, &canon(64)),
            BytesN::from_array(&env, &canon(65)),
            BytesN::from_array(&env, &canon(66)),
        ]);
        let zero_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);
        let fee = 0i128;

        let pub_inputs = TransferPublicInputs {
            anchor: anchor.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments.clone(),
            in_value_commits: zero_commits.clone(),
            out_value_commits: zero_commits.clone(),
            fee,
            asset_id: asset.clone(),
        };

        let mut fee_bytes = [0u8; 32];
        fee_bytes[..16].copy_from_slice(&(fee as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 19] = [
            anchor.clone().into(),
            nullifiers.get(0).unwrap().into(),
            nullifiers.get(1).unwrap().into(),
            nullifiers.get(2).unwrap().into(),
            nullifiers.get(3).unwrap().into(),
            out_commitments.get(0).unwrap().into(),
            out_commitments.get(1).unwrap().into(),
            out_commitments.get(2).unwrap().into(),
            out_commitments.get(3).unwrap().into(),
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            fee_bytes,
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Transfer4x4.into(), &vk_bytes);

        let encrypted_notes = Vec::from_array(&env, [
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
        ]);

        let result = client.try_transfer4(&nullifiers, &out_commitments, &encrypted_notes, &proof, &pub_inputs);
        assert!(result.is_err(), "transfer4 with a duplicate nullifier across non-adjacent slots must be rejected");
        assert!(!client.is_spent(&same_nullifier));
        assert_eq!(client.leaf_count(), 0u32);
    }

    /// Regression test for the audit finding: using the same nullifier (i.e.
    /// the same real note) in both input slots of a single transfer() call
    /// must be rejected, even when the accompanying proof is cryptographically
    /// valid for those exact (duplicated) public inputs — the vulnerability
    /// was that a real note of value V could be double-counted as two inputs,
    /// making sum_in = 2V and minting fabricated value in the outputs. This
    /// must be caught before proof verification, not by relying on the proof
    /// to reject it (the circuit itself didn't constrain this either, prior
    /// to the accompanying fix in transfer_2in2out/transfer.circom).
    #[test]
    fn transfer_rejects_duplicate_nullifier_in_same_call() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let asset = Address::generate(&env);
        client.set_asset_approved(&asset, &true);
        let anchor = client.merkle_root();

        let same_nullifier = BytesN::from_array(&env, &canon(99));
        let nullifiers = Vec::from_array(&env, [same_nullifier.clone(), same_nullifier.clone()]);
        let out_commitments = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(15)),
            BytesN::from_array(&env, &canon(16)),
        ]);
        let zero_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);
        let fee = 0i128;

        let pub_inputs = TransferPublicInputs {
            anchor: anchor.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments.clone(),
            in_value_commits: zero_commits.clone(),
            out_value_commits: zero_commits.clone(),
            fee,
            asset_id: asset.clone(),
        };

        // Build a proof that is genuinely valid for these (duplicated)
        // public inputs — proving the rejection comes from the contract's
        // own duplicate check, not from proof verification failing.
        let mut fee_bytes = [0u8; 32];
        fee_bytes[..16].copy_from_slice(&(fee as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 11] = [
            anchor.clone().into(),
            same_nullifier.clone().into(),
            same_nullifier.clone().into(),
            out_commitments.get(0).unwrap().into(),
            out_commitments.get(1).unwrap().into(),
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            fee_bytes,
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Transfer.into(), &vk_bytes);

        let encrypted_notes = Vec::from_array(&env, [
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
        ]);

        let result = client.try_transfer(&nullifiers, &out_commitments, &encrypted_notes, &proof, &pub_inputs);
        assert!(result.is_err(), "transfer with duplicate nullifiers must be rejected");
        assert!(!client.is_spent(&same_nullifier), "the nullifier must not be marked spent by a rejected call");
        assert_eq!(client.leaf_count(), 0u32, "no notes should have been inserted");
    }

    /// Same vulnerability class, output side: duplicate output commitments
    /// within one call must also be rejected (defense-in-depth — not a
    /// value-fabrication vector on its own since nullifier-spent tracking
    /// still prevents re-spending the underlying note, but keeping the tree
    /// free of duplicate leaves is simpler to reason about).
    #[test]
    fn transfer_rejects_duplicate_output_commitment_in_same_call() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let asset = Address::generate(&env);
        client.set_asset_approved(&asset, &true);
        let anchor = client.merkle_root();

        let nullifiers = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(17)),
            BytesN::from_array(&env, &canon(18)),
        ]);
        let same_commitment = BytesN::from_array(&env, &canon(19));
        let out_commitments = Vec::from_array(&env, [same_commitment.clone(), same_commitment.clone()]);
        let zero_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);
        let fee = 0i128;

        let pub_inputs = TransferPublicInputs {
            anchor: anchor.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments.clone(),
            in_value_commits: zero_commits.clone(),
            out_value_commits: zero_commits.clone(),
            fee,
            asset_id: asset.clone(),
        };

        let mut fee_bytes = [0u8; 32];
        fee_bytes[..16].copy_from_slice(&(fee as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 11] = [
            anchor.clone().into(),
            nullifiers.get(0).unwrap().into(),
            nullifiers.get(1).unwrap().into(),
            same_commitment.clone().into(),
            same_commitment.clone().into(),
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            fee_bytes,
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Transfer.into(), &vk_bytes);

        let encrypted_notes = Vec::from_array(&env, [
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
        ]);

        let result = client.try_transfer(&nullifiers, &out_commitments, &encrypted_notes, &proof, &pub_inputs);
        assert!(result.is_err(), "transfer with duplicate output commitments must be rejected");
    }

    #[test]
    fn transfer_rejects_already_spent_nullifier() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let asset = Address::generate(&env);
        client.set_asset_approved(&asset, &true);
        let anchor = client.merkle_root();

        let nullifiers = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(21)),
            BytesN::from_array(&env, &canon(22)),
        ]);
        let out_commitments = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(23)),
            BytesN::from_array(&env, &canon(24)),
        ]);
        let zero_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);

        let pub_inputs = TransferPublicInputs {
            anchor: anchor.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments.clone(),
            in_value_commits: zero_commits.clone(),
            out_value_commits: zero_commits.clone(),
            fee: 0,
            asset_id: asset.clone(),
        };
        let public_inputs_le: [[u8; 32]; 11] = [
            anchor.clone().into(),
            nullifiers.get(0).unwrap().into(),
            nullifiers.get(1).unwrap().into(),
            out_commitments.get(0).unwrap().into(),
            out_commitments.get(1).unwrap().into(),
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            [0u8; 32],
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Transfer.into(), &vk_bytes);
        let encrypted_notes = Vec::from_array(&env, [
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
        ]);

        client.transfer(&nullifiers, &out_commitments, &encrypted_notes, &proof, &pub_inputs);

        // Same nullifiers again (different output commitments, still a fresh
        // valid proof for those inputs) must fail on the spent-nullifier check.
        let out_commitments_2 = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(25)),
            BytesN::from_array(&env, &canon(26)),
        ]);
        let anchor2 = client.merkle_root();
        let pub_inputs_2 = TransferPublicInputs {
            anchor: anchor2.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments_2.clone(),
            in_value_commits: zero_commits.clone(),
            out_value_commits: zero_commits.clone(),
            fee: 0,
            asset_id: asset.clone(),
        };
        let public_inputs_le_2: [[u8; 32]; 11] = [
            anchor2.clone().into(),
            nullifiers.get(0).unwrap().into(),
            nullifiers.get(1).unwrap().into(),
            out_commitments_2.get(0).unwrap().into(),
            out_commitments_2.get(1).unwrap().into(),
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            [0u8; 32],
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes_2, proof_2) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le_2);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .update_verifying_key(&CircuitType::Transfer.into(), &vk_bytes_2);

        let result = client.try_transfer(&nullifiers, &out_commitments_2, &encrypted_notes, &proof_2, &pub_inputs_2);
        assert!(result.is_err());
    }

    #[test]
    fn unshield_succeeds_with_valid_proof_and_releases_tokens() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);
        let recipient   = Address::generate(&env);
        let shielder    = Address::generate(&env);

        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&shielder, &1_000_000_000);

        // Go through a real shield() first (rather than minting straight to
        // the contract) so shielded_supply() accounting is genuinely
        // exercised, not just token balances — this is the regression test
        // for unshield() now symmetrically decrementing what shield()
        // increments (see the audit finding this fixes).
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let shield_rho = BytesN::from_array(&env, &canon(30));
        let shield_rcm = BytesN::from_array(&env, &canon(31));
        let shield_amount: i128 = 1_000_000;
        let commitment_bytes = compute_commitment(&env, shield_amount, &token_addr, &shield_rho, &shield_rcm, &mut hasher);
        let commitment = BytesN::from_array(&env, &commitment_bytes);
        let value_commit = BytesN::from_array(&env, &[0u8; 32]);
        let shield_pub_inputs = ShieldPublicInputs {
            commitment: commitment.clone(),
            value_commit: value_commit.clone(),
            pub_value: shield_amount,
            pub_asset_id: token_addr.clone(),
        };
        let shield_proof = prove_and_register_shield(&env, &verifier, &commitment, &value_commit, shield_amount, &token_addr);
        let encrypted_note = Bytes::from_array(&env, &[0u8; 176]);
        client.shield(&shielder, &token_addr, &shield_amount, &shield_rho, &shield_rcm, &commitment, &encrypted_note, &shield_proof, &shield_pub_inputs);
        assert_eq!(client.shielded_supply(&token_addr), shield_amount);

        let anchor = client.merkle_root();
        let nullifier = BytesN::from_array(&env, &canon(31));
        let recipient_field = address_to_field_bytes(&env, &recipient);
        let recipient_hash_bytes = hasher.hash(&recipient_field, &[0u8; 32]);
        let recipient_hash = BytesN::from_array(&env, &recipient_hash_bytes);

        let pub_value: i128 = 500_000;
        let pub_inputs = UnshieldPublicInputs {
            anchor: anchor.clone(),
            nullifier: nullifier.clone(),
            pub_value,
            pub_asset_id: token_addr.clone(),
            recipient_hash: recipient_hash.clone(),
        };

        let mut value_bytes = [0u8; 32];
        value_bytes[..16].copy_from_slice(&(pub_value as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 5] = [
            anchor.clone().into(),
            nullifier.clone().into(),
            value_bytes,
            address_to_field_bytes(&env, &token_addr),
            recipient_hash_bytes,
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Unshield.into(), &vk_bytes);

        let binding_tag = BytesN::from_array(&env, &[0u8; 32]);
        client.unshield(&nullifier, &recipient, &binding_tag, &proof, &pub_inputs);

        assert!(client.is_spent(&nullifier));
        assert_eq!(stellar_asset.balance(&recipient), 500_000i128);
        assert_eq!(stellar_asset.balance(&token), shield_amount - 500_000i128);
        assert_eq!(
            client.shielded_supply(&token_addr),
            shield_amount - pub_value,
            "shielded_supply must decrease symmetrically with shield()'s increase"
        );
    }

    #[test]
    fn unshield_rejects_when_it_would_underflow_shielded_supply() {
        // No prior shield() for this asset, so shielded_supply() is 0;
        // unshielding any positive amount must fail cleanly rather than
        // wrapping supply negative or succeeding despite no real backing
        // having ever been recorded for this asset.
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);
        let recipient   = Address::generate(&env);

        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&token, &1_000_000_000);

        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let anchor = client.merkle_root();
        let nullifier = BytesN::from_array(&env, &canon(32));
        let recipient_field = address_to_field_bytes(&env, &recipient);
        let recipient_hash_bytes = hasher.hash(&recipient_field, &[0u8; 32]);
        let recipient_hash = BytesN::from_array(&env, &recipient_hash_bytes);

        let pub_value: i128 = 500_000;
        let pub_inputs = UnshieldPublicInputs {
            anchor: anchor.clone(),
            nullifier: nullifier.clone(),
            pub_value,
            pub_asset_id: token_addr.clone(),
            recipient_hash,
        };

        let mut value_bytes = [0u8; 32];
        value_bytes[..16].copy_from_slice(&(pub_value as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 5] = [
            anchor.clone().into(),
            nullifier.clone().into(),
            value_bytes,
            address_to_field_bytes(&env, &token_addr),
            recipient_hash_bytes,
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Unshield.into(), &vk_bytes);

        let binding_tag = BytesN::from_array(&env, &[0u8; 32]);
        let result = client.try_unshield(&nullifier, &recipient, &binding_tag, &proof, &pub_inputs);
        assert!(result.is_err());
        assert!(!client.is_spent(&nullifier), "a rejected unshield must not mark the nullifier spent");
    }

    #[test]
    fn unshield_rejects_wrong_recipient_hash() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);
        let recipient   = Address::generate(&env);
        let wrong_recipient = Address::generate(&env);

        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&token, &1_000_000_000);

        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let anchor = client.merkle_root();
        let nullifier = BytesN::from_array(&env, &canon(41));
        // recipient_hash computed for `recipient`, but the call passes `wrong_recipient`.
        let recipient_field = address_to_field_bytes(&env, &recipient);
        let recipient_hash_bytes = hasher.hash(&recipient_field, &[0u8; 32]);
        let recipient_hash = BytesN::from_array(&env, &recipient_hash_bytes);

        let pub_value: i128 = 100;
        let pub_inputs = UnshieldPublicInputs {
            anchor,
            nullifier: nullifier.clone(),
            pub_value,
            pub_asset_id: token_addr,
            recipient_hash,
        };

        let binding_tag = BytesN::from_array(&env, &[0u8; 32]);
        let result = client.try_unshield(&nullifier, &wrong_recipient, &binding_tag, &Bytes::new(&env), &pub_inputs);
        assert!(result.is_err());
    }

    /// Regression test for a critical audit finding fixed in
    /// `contracts/swap`: `binding_tag` lets a caller cryptographically bind
    /// an unshield proof to more than just `to` — a proof whose
    /// `recipient_hash` was computed with one `binding_tag` must be rejected
    /// if submitted with a *different* `binding_tag`, even though `to` and
    /// every other field are identical. Without this, `contracts/swap`
    /// could not bind its reused ownership proof to a specific
    /// (`intent_commitment`, `refund_to`) pair, letting a replayed proof be
    /// resubmitted with a different `refund_to` to steal escrowed funds.
    #[test]
    fn unshield_binding_tag_changes_the_accepted_recipient_hash() {
        let (env, admin, token, verifier) = setup();
        env.mock_all_auths();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);
        let recipient   = Address::generate(&env);

        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&token, &1_000_000_000);

        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let anchor = client.merkle_root();
        let nullifier = BytesN::from_array(&env, &canon(71));
        let recipient_field = address_to_field_bytes(&env, &recipient);

        let tag_a = BytesN::from_array(&env, &[0xAAu8; 32]);
        let tag_b = BytesN::from_array(&env, &[0xBBu8; 32]);
        let tag_a_bytes: [u8; 32] = tag_a.clone().into();

        // recipient_hash computed for tag_a.
        let recipient_hash_bytes = hasher.hash(&recipient_field, &tag_a_bytes);
        let recipient_hash = BytesN::from_array(&env, &recipient_hash_bytes);

        let pub_value: i128 = 100;
        let pub_inputs = UnshieldPublicInputs {
            anchor,
            nullifier: nullifier.clone(),
            pub_value,
            pub_asset_id: token_addr,
            recipient_hash,
        };

        // Submitting with tag_b (a different binding_tag) must fail, even
        // though `to`/nullifier/amount/asset are all identical and correct.
        let result = client.try_unshield(&nullifier, &recipient, &tag_b, &Bytes::new(&env), &pub_inputs);
        assert!(result.is_err());
        assert!(!client.is_spent(&nullifier));
    }

    /// Regression test for the reviewers' central "budget viability" concern
    /// (docs/POC_IMPLEMENTATION.md's documented `HostError: Error(Budget,
    /// ExceededLimit)` testnet failure): a full shield() call — commitment
    /// computation, Merkle insert, and a genuine on-chain Groth16
    /// verification against a real (if synthetic) proof — must fit inside
    /// `InvocationResourceLimits::mainnet()`, the SDK's snapshot of the
    /// *actual current* network limit (400M instructions as of
    /// 2026-07-10), not just `Env::default()`'s more conservative built-in
    /// 100M, which `setup()` overrides for exactly this reason.
    ///
    /// Measured at the time this test was written: ~104M instructions for
    /// the full call (~30M of which is the verifier's cross-contract Groth16
    /// check alone) — about 26% of the 400M budget, comfortable headroom.
    #[test]
    fn shield_fits_within_mainnet_instruction_budget() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);
        let user        = Address::generate(&env);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&user, &1_000_000_000);

        let rho = BytesN::from_array(&env, &canon(9));
        let rcm = BytesN::from_array(&env, &canon(10));
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let computed = compute_commitment(&env, 1_000, &token_addr, &rho, &rcm, &mut hasher);
        let commitment = BytesN::from_array(&env, &computed);
        let value_commit = BytesN::from_array(&env, &[0u8; 32]);
        let enc = Bytes::from_array(&env, &[0u8; 176]);
        let pub_inputs = ShieldPublicInputs {
            commitment: commitment.clone(),
            value_commit: value_commit.clone(),
            pub_value: 1_000,
            pub_asset_id: token_addr.clone(),
        };

        let proof = prove_and_register_shield(&env, &verifier, &commitment, &value_commit, 1_000, &token_addr);

        env.cost_estimate().budget().reset_tracker();
        client.shield(&user, &token_addr, &1_000i128, &rho, &rcm, &commitment, &enc, &proof, &pub_inputs);
        let used = env.cost_estimate().budget().cpu_instruction_cost();
        assert!(
            used < 400_000_000,
            "shield() used {used} instructions, exceeding the 400M mainnet budget"
        );
    }

    /// Real-WASM cross-check for the measurement above. `cpu_instruction_cost()`'s
    /// own doc comment warns that "CPU instructions are likely to be
    /// underestimated when running Rust code compared to running the WASM
    /// equivalent" — every budget test above registers the contract as a
    /// native Rust struct (`env.register(ShieldedToken, ())`), not compiled
    /// WASM, so that gap was never actually measured. This test instantiates
    /// both `token` and `verifier` from their real, freshly-built
    /// `wasm32v1-none` release binaries instead, to get an honest answer to
    /// "how much does the Rust-vs-WASM gap actually matter here."
    ///
    /// Measured at the time this test was written: 113,170,011 instructions —
    /// about 9% higher than the native-Rust estimate (~104M) above, and still
    /// 28% of the 400M mainnet budget. The gap is real but modest for this
    /// specific call; see `transfer4_real_wasm_instruction_cost` below for
    /// why this same gap matters far more for the heaviest entrypoint.
    const TOKEN_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zkella_token.wasm");
    const VERIFIER_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zkella_verifier.wasm");

    #[test]
    fn shield_real_wasm_instruction_cost_versus_native_rust_estimate() {
        let env = Env::default();
        env.cost_estimate().budget().reset_limits(400_000_000, 41_943_040);
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let token = env.register(TOKEN_WASM, ());
        let verifier = env.register(VERIFIER_WASM, ());
        zkella_verifier::VerifierContractClient::new(&env, &verifier).initialize(&admin);
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);
        let user        = Address::generate(&env);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&user, &1_000_000_000);

        let rho = BytesN::from_array(&env, &canon(9));
        let rcm = BytesN::from_array(&env, &canon(10));
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let computed = compute_commitment(&env, 1_000, &token_addr, &rho, &rcm, &mut hasher);
        let commitment = BytesN::from_array(&env, &computed);
        let value_commit = BytesN::from_array(&env, &[0u8; 32]);
        let enc = Bytes::from_array(&env, &[0u8; 176]);
        let pub_inputs = ShieldPublicInputs {
            commitment: commitment.clone(),
            value_commit: value_commit.clone(),
            pub_value: 1_000,
            pub_asset_id: token_addr.clone(),
        };

        let proof = prove_and_register_shield(&env, &verifier, &commitment, &value_commit, 1_000, &token_addr);

        env.cost_estimate().budget().reset_tracker();
        client.shield(&user, &token_addr, &1_000i128, &rho, &rcm, &commitment, &enc, &proof, &pub_inputs);
        let used = env.cost_estimate().budget().cpu_instruction_cost();
        assert!(
            used < 400_000_000,
            "shield() (real WASM) used {used} instructions, exceeding the 400M mainnet budget"
        );
    }

    /// Regression test for the reviewers' follow-up "budget viability" concern:
    /// shield() is measured above, but the heavier transfer path (Merkle-anchor
    /// check, two nullifier-spent checks, and a genuine on-chain Groth16
    /// verification with an 11-signal public input vs shield's 4) had never
    /// been measured against the real network limit. See
    /// `transfer4_fits_within_mainnet_instruction_budget` below for the
    /// heavier 4-in/4-out case.
    ///
    /// Measured at the time this test was written: ~211M instructions for
    /// the full 2-in/2-out call — about 53% of the 400M budget.
    #[test]
    fn transfer_fits_within_mainnet_instruction_budget() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let asset = Address::generate(&env);
        client.set_asset_approved(&asset, &true);
        let anchor = client.merkle_root();

        let nullifiers = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(111)),
            BytesN::from_array(&env, &canon(112)),
        ]);
        let out_commitments = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(113)),
            BytesN::from_array(&env, &canon(114)),
        ]);
        let zero_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);
        let fee = 0i128;

        let pub_inputs = TransferPublicInputs {
            anchor: anchor.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments.clone(),
            in_value_commits: zero_commits.clone(),
            out_value_commits: zero_commits.clone(),
            fee,
            asset_id: asset.clone(),
        };

        let mut fee_bytes = [0u8; 32];
        fee_bytes[..16].copy_from_slice(&(fee as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 11] = [
            anchor.clone().into(),
            nullifiers.get(0).unwrap().into(),
            nullifiers.get(1).unwrap().into(),
            out_commitments.get(0).unwrap().into(),
            out_commitments.get(1).unwrap().into(),
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            fee_bytes,
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Transfer.into(), &vk_bytes);

        let encrypted_notes = Vec::from_array(&env, [
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
        ]);

        env.cost_estimate().budget().reset_tracker();
        client.transfer(&nullifiers, &out_commitments, &encrypted_notes, &proof, &pub_inputs);
        let used = env.cost_estimate().budget().cpu_instruction_cost();
        assert!(
            used < 400_000_000,
            "transfer() used {used} instructions, exceeding the 400M mainnet budget"
        );
    }

    /// Real-WASM cross-check for 2-in/2-out transfer, closing the one gap
    /// left after shield's and transfer4's real-WASM measurements above.
    ///
    /// Measured at the time this test was written: 228,219,960 instructions —
    /// 57% of the 400M mainnet budget, with real headroom (consistent with
    /// shield's ~9% Rust-to-WASM gap applied to the ~211M Rust estimate).
    #[test]
    fn transfer_real_wasm_instruction_cost() {
        let env = Env::default();
        env.cost_estimate().budget().reset_limits(400_000_000, 41_943_040);
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let token = env.register(TOKEN_WASM, ());
        let verifier = env.register(VERIFIER_WASM, ());
        zkella_verifier::VerifierContractClient::new(&env, &verifier).initialize(&admin);
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let asset = Address::generate(&env);
        client.set_asset_approved(&asset, &true);
        let anchor = client.merkle_root();

        let nullifiers = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(211)),
            BytesN::from_array(&env, &canon(212)),
        ]);
        let out_commitments = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(213)),
            BytesN::from_array(&env, &canon(214)),
        ]);
        let zero_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);
        let fee = 0i128;

        let pub_inputs = TransferPublicInputs {
            anchor: anchor.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments.clone(),
            in_value_commits: zero_commits.clone(),
            out_value_commits: zero_commits.clone(),
            fee,
            asset_id: asset.clone(),
        };

        let mut fee_bytes = [0u8; 32];
        fee_bytes[..16].copy_from_slice(&(fee as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 11] = [
            anchor.clone().into(),
            nullifiers.get(0).unwrap().into(),
            nullifiers.get(1).unwrap().into(),
            out_commitments.get(0).unwrap().into(),
            out_commitments.get(1).unwrap().into(),
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            fee_bytes,
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Transfer.into(), &vk_bytes);

        let encrypted_notes = Vec::from_array(&env, [
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
        ]);

        env.cost_estimate().budget().reset_tracker();
        client.transfer(&nullifiers, &out_commitments, &encrypted_notes, &proof, &pub_inputs);
        let used = env.cost_estimate().budget().cpu_instruction_cost();
        assert!(
            used < 400_000_000,
            "transfer() (real WASM) used {used} instructions, exceeding the 400M mainnet budget"
        );
    }

    /// Same measurement for the heaviest entrypoint on the token contract:
    /// 4-in/4-out transfer, with a 19-signal public input (vs 2-in/2-out's
    /// 11), four nullifier-spent checks, and four Merkle inserts. If any
    /// entrypoint were going to threaten the mainnet budget, this is it.
    ///
    /// Measured at the time this test was written: ~358M instructions for
    /// the full 4-in/4-out call — about 89% of the 400M budget, well inside
    /// the limit but with materially less headroom than shield() (26%) or
    /// 2-in/2-out transfer (53%). This is a real, honest constraint worth
    /// tracking: any future circuit change, additional public input, or
    /// added on-chain check to this specific path has little room left
    /// before it would need a corresponding optimization.
    #[test]
    fn transfer4_fits_within_mainnet_instruction_budget() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let asset = Address::generate(&env);
        client.set_asset_approved(&asset, &true);
        let anchor = client.merkle_root();

        let nullifiers = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(151)),
            BytesN::from_array(&env, &canon(152)),
            BytesN::from_array(&env, &canon(153)),
            BytesN::from_array(&env, &canon(154)),
        ]);
        let out_commitments = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(155)),
            BytesN::from_array(&env, &canon(156)),
            BytesN::from_array(&env, &canon(157)),
            BytesN::from_array(&env, &canon(158)),
        ]);
        let zero_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);
        let fee = 0i128;

        let pub_inputs = TransferPublicInputs {
            anchor: anchor.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments.clone(),
            in_value_commits: zero_commits.clone(),
            out_value_commits: zero_commits.clone(),
            fee,
            asset_id: asset.clone(),
        };

        let mut fee_bytes = [0u8; 32];
        fee_bytes[..16].copy_from_slice(&(fee as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 19] = [
            anchor.clone().into(),
            nullifiers.get(0).unwrap().into(),
            nullifiers.get(1).unwrap().into(),
            nullifiers.get(2).unwrap().into(),
            nullifiers.get(3).unwrap().into(),
            out_commitments.get(0).unwrap().into(),
            out_commitments.get(1).unwrap().into(),
            out_commitments.get(2).unwrap().into(),
            out_commitments.get(3).unwrap().into(),
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            fee_bytes,
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Transfer4x4.into(), &vk_bytes);

        let encrypted_notes = Vec::from_array(&env, [
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
        ]);

        env.cost_estimate().budget().reset_tracker();
        client.transfer4(&nullifiers, &out_commitments, &encrypted_notes, &proof, &pub_inputs);
        let used = env.cost_estimate().budget().cpu_instruction_cost();
        assert!(
            used < 400_000_000,
            "transfer4() used {used} instructions, exceeding the 400M mainnet budget"
        );
    }

    /// Real-WASM cross-check for transfer4, for the same reason as
    /// `shield_real_wasm_instruction_cost_versus_native_rust_estimate` above.
    /// This is the one that actually matters: the native-Rust estimate
    /// already put transfer4 at 89% of the mainnet budget with only ~11%
    /// headroom, and shield's real-WASM run came in ~9% higher than its
    /// Rust estimate — applying a similar gap to transfer4's 358M Rust
    /// figure would land close to or over the 400M limit. Measuring the
    /// real number directly here rather than extrapolating.
    ///
    /// Measured at the time this test was written: 388,076,971 instructions —
    /// **97% of the 400M mainnet budget**, with only ~3% headroom. The
    /// native-Rust estimate (89%) materially understated the real risk here;
    /// this is the number that should be quoted anywhere this deliverable's
    /// budget viability is discussed, not the Rust-only figure.
    ///
    /// **Update:** this margin turned out to be compiler-version sensitive.
    /// Re-measured on a clean build against every currently-available stable
    /// Rust compatible with this workspace's pinned `soroban-sdk` version
    /// (1.92.0, 1.93.0, 1.94.1), this same call measured marginally *over*
    /// the 400M budget on all three (400,000,643 / 400,000,211 / 400,001,136
    /// respectively) — see `docs/SCF_READINESS.md` for the full comparison.
    /// Source and `Cargo.lock` were unchanged from when the 388,076,971
    /// figure was recorded, so the swing was real but small (a few hundred
    /// to ~1,100 instructions across compiler versions), not large enough on
    /// its own to explain the full gap from the original figure. The test
    /// was `#[ignore]`d rather than deleted at that point: a known,
    /// already-tracked gap (the verifier's batched multi-scalar-
    /// multiplication optimization, see `contracts/verifier/src/lib.rs`),
    /// not a flake.
    ///
    /// **Update 2, after the verifier's `g1_msm` optimization landed:**
    /// this call now measures **396,688,826 instructions, 99.17% of the
    /// 400M mainnet budget**. Un-ignored and re-verified: this is back
    /// under the line, and the optimization is real, correct (all 17
    /// verifier tests, covering every real circuit's actual proof, still
    /// pass unchanged), and worth roughly 3.3M instructions relative to the
    /// over-budget, unoptimized measurement above. It is not, however, the
    /// comfortable margin the note above was hoping to confirm: ~0.83%
    /// headroom (3,311,174 instructions) is thin, not safe, and a further
    /// contract change, a longer circuit, or another compiler-version shift
    /// could push this back over the line. Shipping transfer4 to mainnet on
    /// this margin alone is not recommended without either additional
    /// optimization work or a deliberate, documented decision to accept a
    /// sub-1%-headroom budget on this specific entrypoint.
    #[test]
    fn transfer4_real_wasm_instruction_cost() {
        let env = Env::default();
        env.cost_estimate().budget().reset_limits(400_000_000, 41_943_040);
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let token = env.register(TOKEN_WASM, ());
        let verifier = env.register(VERIFIER_WASM, ());
        zkella_verifier::VerifierContractClient::new(&env, &verifier).initialize(&admin);
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let asset = Address::generate(&env);
        client.set_asset_approved(&asset, &true);
        let anchor = client.merkle_root();

        let nullifiers = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(181)),
            BytesN::from_array(&env, &canon(182)),
            BytesN::from_array(&env, &canon(183)),
            BytesN::from_array(&env, &canon(184)),
        ]);
        let out_commitments = Vec::from_array(&env, [
            BytesN::from_array(&env, &canon(185)),
            BytesN::from_array(&env, &canon(186)),
            BytesN::from_array(&env, &canon(187)),
            BytesN::from_array(&env, &canon(188)),
        ]);
        let zero_commits = Vec::from_array(&env, [
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
            BytesN::from_array(&env, &[0u8; 32]),
        ]);
        let fee = 0i128;

        let pub_inputs = TransferPublicInputs {
            anchor: anchor.clone(),
            nullifiers: nullifiers.clone(),
            out_commitments: out_commitments.clone(),
            in_value_commits: zero_commits.clone(),
            out_value_commits: zero_commits.clone(),
            fee,
            asset_id: asset.clone(),
        };

        let mut fee_bytes = [0u8; 32];
        fee_bytes[..16].copy_from_slice(&(fee as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 19] = [
            anchor.clone().into(),
            nullifiers.get(0).unwrap().into(),
            nullifiers.get(1).unwrap().into(),
            nullifiers.get(2).unwrap().into(),
            nullifiers.get(3).unwrap().into(),
            out_commitments.get(0).unwrap().into(),
            out_commitments.get(1).unwrap().into(),
            out_commitments.get(2).unwrap().into(),
            out_commitments.get(3).unwrap().into(),
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            fee_bytes,
            address_to_field_bytes(&env, &asset),
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Transfer4x4.into(), &vk_bytes);

        let encrypted_notes = Vec::from_array(&env, [
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
            Bytes::from_array(&env, &[0u8; 176]),
        ]);

        env.cost_estimate().budget().reset_tracker();
        client.transfer4(&nullifiers, &out_commitments, &encrypted_notes, &proof, &pub_inputs);
        let used = env.cost_estimate().budget().cpu_instruction_cost();
        assert!(
            used < 400_000_000,
            "transfer4() (real WASM) used {used} instructions, exceeding the 400M mainnet budget"
        );
    }

    #[test]
    fn pause_blocks_state_changing_calls_and_unpause_restores_them() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let args = (
            BytesN::from_array(&env, &canon(1)),
            Address::generate(&env),
            BytesN::from_array(&env, &[0u8; 32]),
            Bytes::new(&env),
            UnshieldPublicInputs {
                anchor: client.merkle_root(),
                nullifier: BytesN::from_array(&env, &canon(1)),
                pub_value: 1,
                pub_asset_id: Address::generate(&env),
                recipient_hash: BytesN::from_array(&env, &[0u8; 32]),
            },
        );
        client.pause();
        let paused = client.try_unshield(&args.0, &args.1, &args.2, &args.3, &args.4);
        assert_eq!(paused.err().unwrap().unwrap(), Error::Paused);

        client.unpause();
        let unpaused = client.try_unshield(&args.0, &args.1, &args.2, &args.3, &args.4);
        assert_ne!(unpaused.err().unwrap().unwrap(), Error::Paused);
    }

    #[test]
    fn admin_transfer_is_two_step_and_pending_admin_is_consumed() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        assert!(client.try_accept_admin().is_err(), "no transfer pending yet");

        let new_admin = Address::generate(&env);
        client.transfer_admin(&new_admin);
        client.accept_admin();
        assert!(
            client.try_accept_admin().is_err(),
            "pending admin must be cleared after acceptance"
        );
    }

    #[test]
    fn merkle_path_reconstructs_the_current_root() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let leaf = BytesN::from_array(&env, &canon(7));
        env.as_contract(&token, || {
            let mut hasher = poseidon::Poseidon2Hasher::new(&env);
            merkle::insert(&env, leaf.clone(), &mut hasher);
        });

        let path = client.merkle_path(&0);
        assert_eq!(path.len(), 32);

        // Leaf 0 is always the left child, so fold upward hashing (current, sibling).
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let mut current: [u8; 32] = leaf.into();
        for i in 0..path.len() {
            let sibling: [u8; 32] = path.get(i).unwrap().into();
            current = hasher.hash(&current, &sibling);
        }
        assert_eq!(BytesN::from_array(&env, &current), client.merkle_root());
    }

    /// Closes the last real-WASM instruction-budget gap among the
    /// Groth16-verifying entrypoints: shield, transfer, and transfer4 were
    /// all measured against the real compiled contract above, but unshield
    /// — a first-class, single-proof withdrawal path with its own
    /// recipient-hash/anchor/nullifier/supply checks plus a real
    /// cross-contract token transfer — had never been. A real prior
    /// shield() call (also real WASM) funds real shielded_supply and a real
    /// token balance on the contract first; the cost tracker is reset
    /// immediately before the measured unshield() call so shield()'s own
    /// cost isn't counted.
    ///
    /// Measured when written: 33,887,174 instructions, about 8.5% of the
    /// 400M budget.
    #[test]
    fn unshield_real_wasm_instruction_cost() {
        let env = Env::default();
        env.cost_estimate().budget().reset_limits(400_000_000, 41_943_040);
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let token = env.register(TOKEN_WASM, ());
        let verifier = env.register(VERIFIER_WASM, ());
        zkella_verifier::VerifierContractClient::new(&env, &verifier).initialize(&admin);
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        client.set_asset_approved(&token_addr, &true);
        let recipient   = Address::generate(&env);
        let shielder    = Address::generate(&env);

        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&shielder, &1_000_000_000);

        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let shield_rho = BytesN::from_array(&env, &canon(40));
        let shield_rcm = BytesN::from_array(&env, &canon(41));
        let shield_amount: i128 = 1_000_000;
        let commitment_bytes = compute_commitment(&env, shield_amount, &token_addr, &shield_rho, &shield_rcm, &mut hasher);
        let commitment = BytesN::from_array(&env, &commitment_bytes);
        let value_commit = BytesN::from_array(&env, &[0u8; 32]);
        let shield_pub_inputs = ShieldPublicInputs {
            commitment: commitment.clone(),
            value_commit: value_commit.clone(),
            pub_value: shield_amount,
            pub_asset_id: token_addr.clone(),
        };
        let shield_proof = prove_and_register_shield(&env, &verifier, &commitment, &value_commit, shield_amount, &token_addr);
        let encrypted_note = Bytes::from_array(&env, &[0u8; 176]);
        client.shield(&shielder, &token_addr, &shield_amount, &shield_rho, &shield_rcm, &commitment, &encrypted_note, &shield_proof, &shield_pub_inputs);

        let anchor = client.merkle_root();
        let nullifier = BytesN::from_array(&env, &canon(42));
        let recipient_field = address_to_field_bytes(&env, &recipient);
        let recipient_hash_bytes = hasher.hash(&recipient_field, &[0u8; 32]);
        let recipient_hash = BytesN::from_array(&env, &recipient_hash_bytes);

        let pub_value: i128 = 500_000;
        let pub_inputs = UnshieldPublicInputs {
            anchor: anchor.clone(),
            nullifier: nullifier.clone(),
            pub_value,
            pub_asset_id: token_addr.clone(),
            recipient_hash: recipient_hash.clone(),
        };

        let mut value_bytes = [0u8; 32];
        value_bytes[..16].copy_from_slice(&(pub_value as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 5] = [
            anchor.clone().into(),
            nullifier.clone().into(),
            value_bytes,
            address_to_field_bytes(&env, &token_addr),
            recipient_hash_bytes,
        ];
        let (vk_bytes, proof) = test_groth16::build_valid_groth16_proof(&env, &public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Unshield.into(), &vk_bytes);

        let binding_tag = BytesN::from_array(&env, &[0u8; 32]);

        env.cost_estimate().budget().reset_tracker();
        client.unshield(&nullifier, &recipient, &binding_tag, &proof, &pub_inputs);
        let used = env.cost_estimate().budget().cpu_instruction_cost();
        assert!(
            used < 400_000_000,
            "unshield() (real WASM) used {used} instructions, exceeding the 400M mainnet budget"
        );
    }

    /// Regression test for a real bug caught while deploying to Stellar Testnet:
    /// `address_to_field_bytes` assumed `addr.to_xdr(env)` was a bare `ScAddress`
    /// (discriminant + 32-byte hash), but it's actually the full `ScVal` wrapper
    /// (an extra 4-byte tag ahead of that) — the fixed offset silently included
    /// the `ScAddress` discriminant and dropped the hash's last 4 bytes. Existing
    /// tests never caught this because both sides of every test call the same
    /// function and stayed self-consistent; this test instead cross-checks
    /// against an independently-computed value (raw StrKey decode of a real
    /// testnet contract address, done in Python, cross-referenced against this
    /// crate's own doc comment claiming equivalence to the TS SDK's
    /// addressToField()) and the exact commitment produced by prover-side
    /// arkworks/circomlibjs tooling for the same inputs.
    #[test]
    fn address_to_field_bytes_and_commitment_match_real_testnet_asset() {
        let env = Env::default();
        let asset = Address::from_string(&soroban_sdk::String::from_str(
            &env,
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
        ));

        let asset_bytes = address_to_field_bytes(&env, &asset);
        let expected_asset_bytes: [u8; 32] = [
            // Raw contract ID d7928b72...ce61 reduced mod r; equals the asset public
            // input inside the real shield proof (verifier tests' SHIELD_PUBLIC_INPUTS_LE_HEX[3]).
            0xd5, 0x92, 0x8b, 0x92, 0x9a, 0x85, 0x78, 0x47, 0xc8, 0x16, 0x79, 0xac, 0x63, 0x1f,
            0xe6, 0xff, 0x8f, 0xa4, 0xa5, 0xb6, 0x0c, 0x71, 0xfb, 0xd4, 0xba, 0x61, 0x65, 0x80,
            0xce, 0x34, 0x06, 0x01,
        ];
        assert_eq!(asset_bytes, expected_asset_bytes, "address_to_field_bytes mismatch");

        let value: i128 = 10_000_000;
        let rho_le = {
            let n: u128 = 823746192837465192837465u128;
            let mut b = [0u8; 32];
            b[..16].copy_from_slice(&n.to_le_bytes());
            b
        };
        let rcm_le = {
            let n: u128 = 918273645192837465918273u128;
            let mut b = [0u8; 32];
            b[..16].copy_from_slice(&n.to_le_bytes());
            b
        };
        let rho = BytesN::from_array(&env, &rho_le);
        let rcm = BytesN::from_array(&env, &rcm_le);

        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let commitment = compute_commitment(&env, value, &asset, &rho, &rcm, &mut hasher);

        let expected_from_circuit: [u8; 32] = [
            0xfe, 0x1a, 0x40, 0xc4, 0x22, 0x85, 0x0b, 0x8b, 0x97, 0x02, 0x2d, 0x66, 0xd2, 0x35,
            0x75, 0xd1, 0x18, 0x2e, 0x3f, 0x53, 0x50, 0xac, 0x90, 0x08, 0x0c, 0x6d, 0x9b, 0x6a,
            0x24, 0xb7, 0x3b, 0x07,
        ];
        assert_eq!(commitment, expected_from_circuit);
    }

    // ── Deliverable 1 & 2 success-criteria tests ─────────────────────────────

    #[test]
    fn min_shield_amount_is_governance_settable_without_redeploy() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        assert_eq!(client.min_shield_amount(), DEFAULT_MIN_SHIELD_AMOUNT);

        // Raise the floor without touching the contract's code or WASM.
        let new_min: i128 = 50_000;
        client.set_min_shield_amount(&new_min);
        assert_eq!(client.min_shield_amount(), new_min);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let asset       = token_id.address();
        client.set_asset_approved(&asset, &true);
        let user = Address::generate(&env);
        soroban_sdk::token::StellarAssetClient::new(&env, &asset).mint(&user, &1_000_000_000);

        // An amount that was valid under the old default (1_000) must now be
        // rejected under the new, higher governance-set floor.
        let rho = BytesN::from_array(&env, &canon(201));
        let rcm = BytesN::from_array(&env, &canon(202));
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let below_amount: i128 = 10_000;
        let computed = compute_commitment(&env, below_amount, &asset, &rho, &rcm, &mut hasher);
        let commitment = BytesN::from_array(&env, &computed);
        let value_commit = BytesN::from_array(&env, &[0u8; 32]);
        let pub_inputs = ShieldPublicInputs {
            commitment: commitment.clone(),
            value_commit: value_commit.clone(),
            pub_value: below_amount,
            pub_asset_id: asset.clone(),
        };
        let proof = prove_and_register_shield(&env, &verifier, &commitment, &value_commit, below_amount, &asset);
        let enc = Bytes::from_array(&env, &[0u8; 176]);
        let result = client.try_shield(&user, &asset, &below_amount, &rho, &rcm, &commitment, &enc, &proof, &pub_inputs);
        assert_eq!(result, Err(Ok(Error::AmountMismatch)));

        // The same amount succeeds once the caller meets the new floor.
        let above_amount: i128 = new_min;
        let rho2 = BytesN::from_array(&env, &canon(203));
        let rcm2 = BytesN::from_array(&env, &canon(204));
        let computed2 = compute_commitment(&env, above_amount, &asset, &rho2, &rcm2, &mut hasher);
        let commitment2 = BytesN::from_array(&env, &computed2);
        let pub_inputs2 = ShieldPublicInputs {
            commitment: commitment2.clone(),
            value_commit: value_commit.clone(),
            pub_value: above_amount,
            pub_asset_id: asset.clone(),
        };
        zkella_verifier::VerifierContractClient::new(&env, &verifier).update_verifying_key(
            &CircuitType::Shield.into(),
            &{
                let mut value_bytes = [0u8; 32];
                value_bytes[..16].copy_from_slice(&(above_amount as u128).to_le_bytes());
                let public_inputs_le: [[u8; 32]; 4] = [
                    commitment2.clone().into(),
                    value_commit.clone().into(),
                    value_bytes,
                    address_to_field_bytes(&env, &asset),
                ];
                let (vk, _) = test_groth16::build_valid_shield_proof(&env, public_inputs_le);
                vk
            },
        );
        let mut value_bytes = [0u8; 32];
        value_bytes[..16].copy_from_slice(&(above_amount as u128).to_le_bytes());
        let public_inputs_le: [[u8; 32]; 4] = [
            commitment2.clone().into(),
            value_commit.clone().into(),
            value_bytes,
            address_to_field_bytes(&env, &asset),
        ];
        let (_, proof2) = test_groth16::build_valid_shield_proof(&env, public_inputs_le);
        client.shield(&user, &asset, &above_amount, &rho2, &rcm2, &commitment2, &enc, &proof2, &pub_inputs2);
    }

    #[test]
    fn shield_rejects_unapproved_asset_and_succeeds_after_approval() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let asset       = token_id.address();
        let user = Address::generate(&env);
        soroban_sdk::token::StellarAssetClient::new(&env, &asset).mint(&user, &1_000_000_000);

        assert!(!client.is_asset_approved(&asset));

        let rho = BytesN::from_array(&env, &canon(205));
        let rcm = BytesN::from_array(&env, &canon(206));
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let amount: i128 = 1_000_000;
        let computed = compute_commitment(&env, amount, &asset, &rho, &rcm, &mut hasher);
        let commitment = BytesN::from_array(&env, &computed);
        let value_commit = BytesN::from_array(&env, &[0u8; 32]);
        let pub_inputs = ShieldPublicInputs {
            commitment: commitment.clone(),
            value_commit: value_commit.clone(),
            pub_value: amount,
            pub_asset_id: asset.clone(),
        };
        let proof = prove_and_register_shield(&env, &verifier, &commitment, &value_commit, amount, &asset);
        let enc = Bytes::from_array(&env, &[0u8; 176]);

        // Real, non-native asset, not yet governance-approved: rejected before
        // any custody or Merkle-tree state changes.
        let result = client.try_shield(&user, &asset, &amount, &rho, &rcm, &commitment, &enc, &proof, &pub_inputs);
        assert_eq!(result, Err(Ok(Error::AssetNotApproved)));
        assert_eq!(client.shielded_supply(&asset), 0);

        // Governance approves it — the same call now succeeds.
        client.set_asset_approved(&asset, &true);
        assert!(client.is_asset_approved(&asset));
        client.shield(&user, &asset, &amount, &rho, &rcm, &commitment, &enc, &proof, &pub_inputs);
        assert_eq!(client.shielded_supply(&asset), amount);

        // Governance can revoke it again; already-shielded notes are
        // unaffected, but no further deposits of that asset are accepted.
        client.set_asset_approved(&asset, &false);
        let rho2 = BytesN::from_array(&env, &canon(207));
        let rcm2 = BytesN::from_array(&env, &canon(208));
        let computed2 = compute_commitment(&env, amount, &asset, &rho2, &rcm2, &mut hasher);
        let commitment2 = BytesN::from_array(&env, &computed2);
        let pub_inputs2 = ShieldPublicInputs {
            commitment: commitment2.clone(),
            value_commit: value_commit.clone(),
            pub_value: amount,
            pub_asset_id: asset.clone(),
        };
        // A VK for Shield is already registered from the first proof above;
        // `prove_and_register_shield` always calls `register_verifying_key`,
        // which would fail the second time, so build and swap in this
        // proof's own VK via `update_verifying_key` instead.
        let mut value_bytes2 = [0u8; 32];
        value_bytes2[..16].copy_from_slice(&(amount as u128).to_le_bytes());
        let public_inputs_le2: [[u8; 32]; 4] = [
            commitment2.clone().into(),
            value_commit.clone().into(),
            value_bytes2,
            address_to_field_bytes(&env, &asset),
        ];
        let (vk2, proof2) = test_groth16::build_valid_shield_proof(&env, public_inputs_le2);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .update_verifying_key(&CircuitType::Shield.into(), &vk2);
        let result2 = client.try_shield(&user, &asset, &amount, &rho2, &rcm2, &commitment2, &enc, &proof2, &pub_inputs2);
        assert_eq!(result2, Err(Ok(Error::AssetNotApproved)));
    }

    #[test]
    fn shield_batch_deposits_multiple_notes_in_one_call_with_one_aggregated_transfer() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let asset       = token_id.address();
        client.set_asset_approved(&asset, &true);
        let user = Address::generate(&env);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &asset);
        stellar_asset.mint(&user, &1_000_000_000);

        let amounts: [i128; 3] = [10_000, 20_000, 30_000];
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let mut items = Vec::new(&env);
        for (i, amount) in amounts.iter().enumerate() {
            let seed = 210u8 + (i as u8) * 2;
            let rho = BytesN::from_array(&env, &[seed; 32]);
            let rcm = BytesN::from_array(&env, &[seed + 1; 32]);
            let computed = compute_commitment(&env, *amount, &asset, &rho, &rcm, &mut hasher);
            let commitment = BytesN::from_array(&env, &computed);
            let value_commit = BytesN::from_array(&env, &[0u8; 32]);
            let pub_inputs = ShieldPublicInputs {
                commitment: commitment.clone(),
                value_commit: value_commit.clone(),
                pub_value: *amount,
                pub_asset_id: asset.clone(),
            };
            let mut value_bytes = [0u8; 32];
            value_bytes[..16].copy_from_slice(&(*amount as u128).to_le_bytes());
            let public_inputs_le: [[u8; 32]; 4] = [
                commitment.clone().into(),
                value_commit.clone().into(),
                value_bytes,
                address_to_field_bytes(&env, &asset),
            ];
            let (vk, proof) = test_groth16::build_valid_shield_proof(&env, public_inputs_le);
            let verifier_client = zkella_verifier::VerifierContractClient::new(&env, &verifier);
            if i == 0 {
                verifier_client.register_verifying_key(&CircuitType::Shield.into(), &vk);
            } else {
                verifier_client.update_verifying_key(&CircuitType::Shield.into(), &vk);
            }
            // Re-derive each item's proof against whichever VK is currently
            // registered isn't possible after the fact — instead, register
            // this item's own fresh VK immediately before building its proof
            // so the two always match by construction, matching how a real
            // multi-item batch would be assembled client-side against a
            // single, already-registered VK. This test approximates that by
            // giving every item a matching one-off VK/proof pair.
            let encrypted_note = Bytes::from_array(&env, &[0u8; 176]);
            items.push_back(ShieldBatchItem {
                amount: *amount,
                rho,
                rcm,
                commitment,
                encrypted_note,
                shield_proof: proof,
                shield_pub: pub_inputs,
            });
        }

        let balance_before = soroban_sdk::token::TokenClient::new(&env, &asset).balance(&user);
        let leaf_indices = client.shield_batch(&user, &asset, &items);
        assert_eq!(leaf_indices.len(), 3);

        let total: i128 = amounts.iter().sum();
        assert_eq!(client.shielded_supply(&asset), total);

        let balance_after = soroban_sdk::token::TokenClient::new(&env, &asset).balance(&user);
        assert_eq!(balance_before - balance_after, total, "batch must move exactly the summed amount in one transfer");
    }

    #[test]
    fn shield_batch_rejects_empty_batch() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);
        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let asset       = token_id.address();
        client.set_asset_approved(&asset, &true);
        let user = Address::generate(&env);

        let empty_items: Vec<ShieldBatchItem> = Vec::new(&env);
        let result = client.try_shield_batch(&user, &asset, &empty_items);
        assert_eq!(result, Err(Ok(Error::EmptyBatch)));
    }

    #[test]
    fn merkle_tree_full_is_rejected_gracefully_not_via_panic() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let asset       = token_id.address();
        client.set_asset_approved(&asset, &true);
        let user = Address::generate(&env);
        soroban_sdk::token::StellarAssetClient::new(&env, &asset).mint(&user, &1_000_000_000);

        // Directly set the tree to one slot below capacity rather than
        // actually inserting `MAX_LEAVES` real notes — the point of this
        // test is the boundary behavior, which is identical whether the
        // tree got there via 4 billion real inserts or a direct storage
        // write, and only the latter is practical to run.
        env.as_contract(&token, || {
            env.storage().instance().set(&StorageKey::NextLeafIndex, &merkle::MAX_LEAVES);
        });

        let rho = BytesN::from_array(&env, &canon(220));
        let rcm = BytesN::from_array(&env, &canon(221));
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let amount: i128 = 1_000;
        let computed = compute_commitment(&env, amount, &asset, &rho, &rcm, &mut hasher);
        let commitment = BytesN::from_array(&env, &computed);
        let value_commit = BytesN::from_array(&env, &[0u8; 32]);
        let pub_inputs = ShieldPublicInputs {
            commitment: commitment.clone(),
            value_commit: value_commit.clone(),
            pub_value: amount,
            pub_asset_id: asset.clone(),
        };
        let proof = prove_and_register_shield(&env, &verifier, &commitment, &value_commit, amount, &asset);
        let enc = Bytes::from_array(&env, &[0u8; 176]);

        // A full tree must fail as a typed `Result::Err`, not a host panic —
        // this is the actual fix: `MerkleTreeFull` used to exist only as a
        // declared error variant, never returned.
        let result = client.try_shield(&user, &asset, &amount, &rho, &rcm, &commitment, &enc, &proof, &pub_inputs);
        assert_eq!(result, Err(Ok(Error::MerkleTreeFull)));
        // No state changed: the tree didn't advance, and no funds moved.
        assert_eq!(client.shielded_supply(&asset), 0);
    }

    /// Per-insert Merkle cost as the tree grows, not just the handful of
    /// leaves exercised in every other test here — the fixed 32-level path's
    /// cost is dominated by however many levels actually have a real
    /// (non-empty-subtree) sibling, which only shows up once the tree has
    /// real depth.
    ///
    /// Scoped to `SCALE` real, separate `shield()` calls, each going through
    /// the actual proof-verification and token-transfer path, not a direct,
    /// lower-level `merkle::insert` shortcut. That matters for a reason
    /// discovered while writing this test, not assumed going in: Soroban's
    /// test environment enforces the same per-invocation resource ceiling a
    /// real mainnet transaction would, including a 400-ledger-entry
    /// footprint limit, and it only resets automatically at a real
    /// invocation boundary. Populating the tree via direct storage
    /// manipulation across thousands of iterations inside one measured
    /// session accumulates footprint against that ceiling with no way to
    /// reset it mid-session, and fails, a genuine, previously-undocumented
    /// constraint on how this kind of at-scale test can be structured, not a
    /// harness quirk to work around silently. Going through real `shield()`
    /// calls, each its own invocation, resets correctly between iterations.
    /// `SCALE` is chosen to keep this test's runtime reasonable given each
    /// iteration pays for a real Groth16 proof and a real BN254 pairing
    /// check; it demonstrates the tree's real depth-dependent cost growth,
    /// not the literal thousands the fully-populated production tree may
    /// eventually reach. Published via the assertion's own failure message.
    #[test]
    fn merkle_insert_cost_as_tree_depth_grows() {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let asset       = token_id.address();
        client.set_asset_approved(&asset, &true);
        let user = Address::generate(&env);
        soroban_sdk::token::StellarAssetClient::new(&env, &asset).mint(&user, &10_000_000_000_000i128);

        const SCALE: u32 = 150;
        let mut hasher = poseidon::Poseidon2Hasher::new(&env);
        let verifier_client = zkella_verifier::VerifierContractClient::new(&env, &verifier);
        let mut last_used: u64 = 0;

        for i in 0..SCALE {
            let amount: i128 = 1_000;
            let seed_bytes = i.to_be_bytes();
            let mut rho_arr = [0u8; 32];
            rho_arr[28..].copy_from_slice(&seed_bytes);
            let mut rcm_arr = [0u8; 32];
            rcm_arr[28..].copy_from_slice(&seed_bytes);
            rcm_arr[0] = 0xff;
            let rho = BytesN::from_array(&env, &rho_arr);
            let rcm = BytesN::from_array(&env, &rcm_arr);

            let computed = compute_commitment(&env, amount, &asset, &rho, &rcm, &mut hasher);
            let commitment = BytesN::from_array(&env, &computed);
            let value_commit = BytesN::from_array(&env, &[0u8; 32]);
            let pub_inputs = ShieldPublicInputs {
                commitment: commitment.clone(),
                value_commit: value_commit.clone(),
                pub_value: amount,
                pub_asset_id: asset.clone(),
            };

            let mut value_bytes = [0u8; 32];
            value_bytes[..16].copy_from_slice(&(amount as u128).to_le_bytes());
            let public_inputs_le: [[u8; 32]; 4] = [
                commitment.clone().into(),
                value_commit.clone().into(),
                value_bytes,
                address_to_field_bytes(&env, &asset),
            ];
            let (vk, proof) = test_groth16::build_valid_shield_proof(&env, public_inputs_le);
            if i == 0 {
                verifier_client.register_verifying_key(&CircuitType::Shield.into(), &vk);
            } else {
                verifier_client.update_verifying_key(&CircuitType::Shield.into(), &vk);
            }

            let enc = Bytes::from_array(&env, &[0u8; 176]);
            env.cost_estimate().budget().reset_limits(400_000_000, 41_943_040);
            client.shield(&user, &asset, &amount, &rho, &rcm, &commitment, &enc, &proof, &pub_inputs);
            last_used = env.cost_estimate().budget().cpu_instruction_cost();
        }

        assert_eq!(client.leaf_count(), SCALE);
        assert!(
            last_used < 400_000_000,
            "shield() with the tree at {SCALE} leaves used {last_used} instructions, exceeding the 400M mainnet budget"
        );
    }
}
