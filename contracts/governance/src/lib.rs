#![no_std]
// See contracts/token/src/lib.rs for why the `publish` deprecation is
// deferred rather than migrated right now.
#![allow(deprecated)]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    Address, Bytes, Env,
};
use zkella_token_interface::TokenClient;
use zkella_verifier_interface::{CircuitType, VerifierClient};

// TESTNET-ONLY OVERRIDE — see the `testnet-fast-timelock` feature's doc
// comment in Cargo.toml. A production/mainnet build must never enable this
// feature; the real, intended value is the 7-day one in the `#[cfg(not(...))]`
// arm below.
#[cfg(feature = "testnet-fast-timelock")]
const VK_TIMELOCK_LEDGERS: u32 = 60; // ~5 minutes at 5s/ledger — demo only
#[cfg(not(feature = "testnet-fast-timelock"))]
const VK_TIMELOCK_LEDGERS: u32 = 120_960; // 7 days at 5s/ledger — real, intended value

#[contracttype]
pub enum StorageKey {
    Admin,
    Verifier, // address of the zkella-verifier registry this governance contract administers
    PendingAdmin,
    PendingVkUpdate(CircuitType),
    Guardian,
    Paused,
    Token,
    PendingTokenAction,
}

#[contracttype]
pub struct PendingVkUpdate {
    pub circuit:    CircuitType,
    pub new_vk:     Bytes,
    pub eta_ledger: u32,
}

/// An admin change to the token contract, which governance executes as the
/// token's admin. Each variant is timelocked and guardian-cancellable.
#[contracttype]
#[derive(Clone)]
pub enum TokenAdminAction {
    MinShieldAmount(i128),
    AssetApproval(Address, bool),
    Relayer(Address, bool),
}

#[contracttype]
pub struct PendingTokenAction {
    pub action:     TokenAdminAction,
    pub eta_ledger: u32,
}

#[contract]
pub struct ZKELLAGovernance;

#[contractimpl]
impl ZKELLAGovernance {

    /// `verifier` must have been deployed with *this contract's own address*
    /// as its admin, so that the cross-contract calls below (which run with
    /// this contract as the calling context) satisfy the verifier's
    /// `admin.require_auth()` implicitly, without a signature.
    /// `guardian` may cancel a queued VK update and nothing else. Keeping it
    /// separate from `admin` means a compromised admin key can still be
    /// stopped from completing a rotation by a party that cannot itself rotate.
    /// `token` must have been deployed with this contract's address as its
    /// admin, so that `execute_token_action` can change the token's admin-only
    /// settings (minimum shield amount, asset approvals, relayers) through the
    /// same timelock as a verifying-key update.
    pub fn initialize(env: Env, admin: Address, verifier: Address, guardian: Address, token: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage().instance().set(&StorageKey::Verifier, &verifier);
        env.storage().instance().set(&StorageKey::Guardian, &guardian);
        env.storage().instance().set(&StorageKey::Token, &token);
    }

    /// Queues a change to the token. A new queued action replaces any pending
    /// one and restarts the timelock, as with verifying-key updates.
    pub fn queue_token_action(env: Env, action: TokenAdminAction) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        Self::assert_not_paused(&env);
        if let TokenAdminAction::MinShieldAmount(amount) = &action {
            assert!(*amount > 0, "amount must be positive");
        }

        let eta = env.ledger().sequence().checked_add(VK_TIMELOCK_LEDGERS).expect("eta overflow");
        let pending = PendingTokenAction { action, eta_ledger: eta };
        env.storage().instance().set(&StorageKey::PendingTokenAction, &pending);
    }

    pub fn execute_token_action(env: Env) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        Self::assert_not_paused(&env);

        let pending: PendingTokenAction = env.storage().instance()
            .get(&StorageKey::PendingTokenAction)
            .expect("no pending token action");
        assert!(env.ledger().sequence() >= pending.eta_ledger, "timelock not elapsed");
        env.storage().instance().remove(&StorageKey::PendingTokenAction);

        let token: Address = env.storage().instance().get(&StorageKey::Token).unwrap();
        let client = TokenClient::new(&env, &token);
        match pending.action {
            TokenAdminAction::MinShieldAmount(amount) => client.set_min_shield_amount(&amount),
            TokenAdminAction::AssetApproval(asset, approved) => client.set_asset_approved(&asset, &approved),
            TokenAdminAction::Relayer(relayer, approved) => client.set_relayer(&relayer, &approved),
        }
    }

    pub fn guardian_cancel_token_action(env: Env) {
        let guardian: Address = env.storage().instance().get(&StorageKey::Guardian).unwrap();
        guardian.require_auth();
        env.storage().instance().remove(&StorageKey::PendingTokenAction);
    }

    pub fn pause(env: Env) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        env.storage().instance().set(&StorageKey::Paused, &true);
    }

    pub fn unpause(env: Env) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        env.storage().instance().set(&StorageKey::Paused, &false);
    }

    /// Queue a verifying key for `circuit` — enforces the 7-day timelock
    /// whether this is the circuit's first-ever key or a replacement of one
    /// already relied upon. There used to be a separate `register_vk`
    /// offering instant, untimelocked first-time activation, on the theory
    /// that a brand-new circuit "doesn't carry the same soundness risk"
    /// since nothing was relying on the old key. That reasoning breaks down
    /// the moment *any* real value exists anywhere in the system: every
    /// circuit's proofs are ultimately checked against the same
    /// `ShieldedToken` Merkle tree and `shielded_supply` bookkeeping, so a
    /// malicious VK activated instantly for *any* circuit — even one that
    /// never had a key before — can forge output notes and drain value
    /// already resting in the pool via an already-legitimate circuit like
    /// `Unshield`. A compromised admin should never get an instant win
    /// against funds it doesn't itself own; removing the fast path removes
    /// that case entirely rather than trying to reason about which specific
    /// circuit registrations are "safe" this time.
    pub fn queue_vk_update(env: Env, circuit: CircuitType, new_vk: Bytes) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        Self::assert_not_paused(&env);

        let eta = env.ledger().sequence().checked_add(VK_TIMELOCK_LEDGERS).expect("eta overflow");
        let update = PendingVkUpdate { circuit, new_vk, eta_ledger: eta };
        env.storage().instance().set(&StorageKey::PendingVkUpdate(circuit), &update);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("vkqueue")),
            (circuit, eta),
        );
    }

    /// Execute a queued VK update after the timelock has passed. Actually
    /// rotates (or, for a circuit with no key yet, registers) the key in the
    /// verifier registry — this used to just return the bytes without
    /// writing them anywhere, leaving governance and the verifier
    /// disconnected. Handles both first-time registration and replacement
    /// through this same timelocked path — see `queue_vk_update`'s doc
    /// comment for why there's no separate, faster path for the first case.
    pub fn execute_vk_update(env: Env, circuit: CircuitType) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        Self::assert_not_paused(&env);

        let update: PendingVkUpdate = env.storage().instance()
            .get(&StorageKey::PendingVkUpdate(circuit))
            .expect("no pending update");
        assert!(env.ledger().sequence() >= update.eta_ledger, "timelock not elapsed");

        env.storage().instance().remove(&StorageKey::PendingVkUpdate(circuit));

        let verifier: Address = env.storage().instance().get(&StorageKey::Verifier).unwrap();
        let verifier_client = VerifierClient::new(&env, &verifier);
        if verifier_client.try_get_verifying_key(&circuit).is_ok() {
            verifier_client.update_verifying_key(&circuit, &update.new_vk);
        } else {
            verifier_client.register_verifying_key(&circuit, &update.new_vk);
        }

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("vkexec")),
            circuit,
        );
    }

    /// Immediately drops the verifier's retained previous key for `circuit`.
    /// The verifier keeps a rotated-out key valid for about a day so in-flight
    /// proofs still verify; when the rotation itself was a security fix (the
    /// old key or circuit is unsound), that grace period must be ended at
    /// once. This is deliberately NOT timelocked: revoking only ever
    /// *narrows* what the verifier accepts.
    pub fn revoke_previous_vk(env: Env, circuit: CircuitType) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        let verifier: Address = env.storage().instance().get(&StorageKey::Verifier).unwrap();
        VerifierClient::new(&env, &verifier).revoke_previous_vk(&circuit);
    }

    /// Pauses the verifier contract this governance administers. The
    /// verifier was deployed with *this contract's own address* as its
    /// admin (see `initialize`'s doc comment), so `verifier::pause`'s
    /// `admin.require_auth()` can only ever be satisfied by a direct call
    /// from this contract's own code — no private key exists that could
    /// sign for a contract address. Without this entrypoint,
    /// `verifier.pause()` (the runbook's primary fail-closed lever for an
    /// actively-exploited verification bug: it stops `verify`/`verify_batch`
    /// from accepting any proof) was permanently unreachable. Deliberately
    /// not gated by this contract's own `pause`: an incident responder must
    /// be able to halt the verifier regardless of whether governance itself
    /// has been paused yet.
    pub fn pause_verifier(env: Env) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        let verifier: Address = env.storage().instance().get(&StorageKey::Verifier).unwrap();
        VerifierClient::new(&env, &verifier).pause();
    }

    /// Reverses `pause_verifier`, for the same reachability reason.
    pub fn unpause_verifier(env: Env) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        let verifier: Address = env.storage().instance().get(&StorageKey::Verifier).unwrap();
        VerifierClient::new(&env, &verifier).unpause();
    }

    /// Same reachability problem as `pause_verifier`, same reason: `token`
    /// is also deployed with this contract's own address as its admin, so
    /// `token::pause`'s `admin.require_auth()` could likewise only ever be
    /// satisfied by a direct call from this contract's own code. Without
    /// this entrypoint, `token.pause()` — the runbook's primary lever for
    /// halting `shield`/`transfer`/`transfer4`/`unshield` — was permanently
    /// unreachable too. Deliberately not gated by this contract's own
    /// `pause`, for the same incident-response reason as `pause_verifier`.
    pub fn pause_token(env: Env) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        let token: Address = env.storage().instance().get(&StorageKey::Token).unwrap();
        TokenClient::new(&env, &token).pause();
    }

    /// Reverses `pause_token`, for the same reachability reason.
    pub fn unpause_token(env: Env) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        let token: Address = env.storage().instance().get(&StorageKey::Token).unwrap();
        TokenClient::new(&env, &token).unpause();
    }

    /// The timelock length this build enforces, in ledgers. Lets anyone check
    /// on-chain that a deployment is not the short-timelock testnet build.
    pub fn timelock_ledgers(_env: Env) -> u32 {
        VK_TIMELOCK_LEDGERS
    }

    /// Cancel a queued VK update before it is executed
    pub fn cancel_vk_update(env: Env, circuit: CircuitType) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        env.storage().instance().remove(&StorageKey::PendingVkUpdate(circuit));
    }

    /// Cancels a queued update. Deliberately not gated by `pause`: a pause is
    /// often declared because of a bad queued update, and cancelling it must
    /// remain possible while paused.
    pub fn guardian_cancel_vk_update(env: Env, circuit: CircuitType) {
        let guardian: Address = env.storage().instance().get(&StorageKey::Guardian).unwrap();
        guardian.require_auth();
        env.storage().instance().remove(&StorageKey::PendingVkUpdate(circuit));
    }

    pub fn transfer_admin(env: Env, new_admin: Address) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        Self::assert_not_paused(&env);
        env.storage().instance().set(&StorageKey::PendingAdmin, &new_admin);
    }

    pub fn accept_admin(env: Env) {
        let pending: Address = env.storage().instance()
            .get(&StorageKey::PendingAdmin).expect("no pending admin");
        pending.require_auth();
        Self::assert_not_paused(&env);
        env.storage().instance().set(&StorageKey::Admin, &pending);
        env.storage().instance().remove(&StorageKey::PendingAdmin);
    }

    fn assert_not_paused(env: &Env) {
        let paused: bool = env.storage().instance().get(&StorageKey::Paused).unwrap_or(false);
        assert!(!paused, "paused");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::BytesN;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use zkella_verifier::{VerifierContract, VerifierContractClient};

    /// Deploys governance + verifier wired together, with governance's own
    /// contract address as the verifier's admin (per this module's contract).
    fn setup() -> (Env, Address, Address, Address) {
        let (env, admin, _guardian, governance_id, verifier_id) = setup_with_guardian();
        (env, admin, governance_id, verifier_id)
    }

    fn setup_with_guardian() -> (Env, Address, Address, Address, Address) {
        let (env, admin, guardian, governance_id, verifier_id, _token_id) = setup_full();
        (env, admin, guardian, governance_id, verifier_id)
    }

    /// Like `setup_with_guardian`, plus a real `ShieldedToken` whose admin is governance.
    fn setup_full() -> (Env, Address, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let governance_id = env.register(ZKELLAGovernance, ());
        let verifier_id = env.register(VerifierContract, ());
        let token_id = env.register(zkella_token::ShieldedToken, ());

        VerifierContractClient::new(&env, &verifier_id).initialize(&governance_id);
        zkella_token::ShieldedTokenClient::new(&env, &token_id).initialize(&governance_id, &verifier_id);
        ZKELLAGovernanceClient::new(&env, &governance_id)
            .initialize(&admin, &verifier_id, &guardian, &token_id);

        (env, admin, guardian, governance_id, verifier_id, token_id)
    }

    fn vk_bytes(env: &Env, len: u32) -> Bytes {
        let mut b = Bytes::new(env);
        for i in 0..len {
            b.push_back((i % 256) as u8);
        }
        b
    }

    const GOVERNANCE_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zkella_governance.wasm");
    const VERIFIER_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zkella_verifier.wasm");

    /// Instruction costs of `queue_vk_update` and `execute_vk_update` (which forwards a real
    /// 768-byte Shield VK to the verifier), native or on the compiled WASM artefacts.
    fn governance_costs(wasm: bool) -> (u64, u64) {
        extern crate std;
        let env = Env::default();
        env.mock_all_auths();
        env.cost_estimate().budget().reset_limits(2_000_000_000, 100_000_000);
        let admin = Address::generate(&env);
        let (gov_id, verifier_id) = if wasm {
            (env.register(GOVERNANCE_WASM, ()), env.register(VERIFIER_WASM, ()))
        } else {
            (env.register(ZKELLAGovernance, ()), env.register(VerifierContract, ()))
        };
        VerifierContractClient::new(&env, &verifier_id).initialize(&gov_id);
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        let guardian = Address::generate(&env);
        let token_placeholder = Address::generate(&env);
        gov.initialize(&admin, &verifier_id, &guardian, &token_placeholder);
        let vk = vk_bytes(&env, 768);
        let mut budget = env.cost_estimate().budget();
        budget.reset_limits(2_000_000_000, 100_000_000);
        gov.queue_vk_update(&CircuitType::Shield, &vk);
        let queue = env.cost_estimate().budget().cpu_instruction_cost();
        env.ledger().with_mut(|li| { li.sequence_number += VK_TIMELOCK_LEDGERS; });
        let mut budget = env.cost_estimate().budget();
        budget.reset_limits(2_000_000_000, 100_000_000);
        gov.execute_vk_update(&CircuitType::Shield);
        let exec = env.cost_estimate().budget().cpu_instruction_cost();
        (queue, exec)
    }

    /// Real-WASM vs native cost of the governance entrypoints; fails above the 400M limit or
    /// more than 25% plus 2M over native. These calls cost under 2M, so the fixed WASM
    /// instantiation overhead (about 0.3M to 1.3M) dominates and a purely relative rule would
    /// be meaningless; the 2M allowance covers it.
    #[test]
    fn cost_parity_governance_queue_and_execute() {
        extern crate std;
        let (nq, ne) = governance_costs(false);
        let (wq, we) = governance_costs(true);
        std::println!("PARITY governance queue_vk_update native={nq} wasm={wq}; execute_vk_update native={ne} wasm={we}");
        for (name, n, w) in [("queue_vk_update", nq, wq), ("execute_vk_update", ne, we)] {
            assert!(w <= 400_000_000, "{name} on WASM uses {w}, over the 400M limit");
            assert!(w * 100 <= n * 125 + 2_000_000 * 100, "{name}: WASM {w} is more than 25% + 2M above native {n}");
        }
    }

    /// Queues `vk` for `circuit` and immediately executes it, fast-forwarding
    /// the ledger past the timelock first — the standard "just get a VK live"
    /// path most tests below only care about as a precondition, not as the
    /// thing under test.
    fn queue_and_execute(env: &Env, gov: &ZKELLAGovernanceClient, circuit: CircuitType, vk: &Bytes) {
        gov.queue_vk_update(&circuit, vk);
        env.ledger().with_mut(|li| { li.sequence_number += VK_TIMELOCK_LEDGERS; });
        gov.execute_vk_update(&circuit);
    }

    /// Regression test for a critical audit finding: there used to be a
    /// separate `register_vk` that activated a circuit's *first* VK
    /// instantly, no timelock — reasoned to be safe since nothing was
    /// relying on the old key yet. That's false once any real value exists
    /// anywhere in the system (every circuit ultimately writes into the same
    /// shared `ShieldedToken` pool), so first-time registration now goes
    /// through the exact same timelocked `queue_vk_update`/`execute_vk_update`
    /// path as a replacement — this test is the regression check that
    /// `execute_vk_update` correctly performs a first-time *registration*
    /// (not an update, which would fail against a circuit with no key yet).
    #[test]
    fn revoke_previous_vk_is_admin_gated_and_forwards_to_the_verifier() {
        let (env, _admin, gov_id, verifier_id) = setup();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        let vk_a = vk_bytes(&env, 64 + 384 + 64 * 2);
        let vk_b = vk_bytes(&env, 64 + 384 + 64 * 3);
        queue_and_execute(&env, &gov, CircuitType::Shield, &vk_a);
        queue_and_execute(&env, &gov, CircuitType::Shield, &vk_b);
        // Rotation retained vk_a; revoking must succeed through governance
        // (which is the verifier's admin) and leave the current key intact.
        gov.revoke_previous_vk(&CircuitType::Shield);
        assert_eq!(
            VerifierContractClient::new(&env, &verifier_id).get_verifying_key(&CircuitType::Shield.into()),
            vk_b
        );
        assert_eq!(gov.timelock_ledgers(), VK_TIMELOCK_LEDGERS);
    }

    #[test]
    fn execute_vk_update_performs_first_time_registration_through_the_timelock() {
        let (env, _admin, governance_id, verifier_id) = setup();
        let gov = ZKELLAGovernanceClient::new(&env, &governance_id);
        let verifier = VerifierContractClient::new(&env, &verifier_id);

        // VK_FIXED_LEN (448) + one IC point (64) = 512, valid shape for 0 public inputs.
        let vk = vk_bytes(&env, 512);

        // Executing before the timelock elapses must fail — first-time
        // registration is no longer exempt from the delay.
        gov.queue_vk_update(&CircuitType::Shield, &vk);
        let early = gov.try_execute_vk_update(&CircuitType::Shield);
        assert!(early.is_err());

        env.ledger().with_mut(|li| { li.sequence_number += VK_TIMELOCK_LEDGERS; });
        gov.execute_vk_update(&CircuitType::Shield);

        let stored = verifier.get_verifying_key(&CircuitType::Shield.into());
        assert_eq!(stored, vk);
    }

    #[test]
    fn execute_vk_update_actually_rotates_the_verifier_key() {
        let (env, _admin, governance_id, verifier_id) = setup();
        let gov = ZKELLAGovernanceClient::new(&env, &governance_id);
        let verifier = VerifierContractClient::new(&env, &verifier_id);

        let original_vk = vk_bytes(&env, 512);
        queue_and_execute(&env, &gov, CircuitType::Shield, &original_vk);

        let new_vk = vk_bytes(&env, 576); // different shape/content
        gov.queue_vk_update(&CircuitType::Shield, &new_vk);

        // Executing before the timelock elapses must fail.
        let early = gov.try_execute_vk_update(&CircuitType::Shield);
        assert!(early.is_err());

        env.ledger().with_mut(|li| {
            li.sequence_number += VK_TIMELOCK_LEDGERS;
        });

        gov.execute_vk_update(&CircuitType::Shield);

        // This is the actual regression check for the bug that prompted this
        // rewrite: execute_vk_update used to return the bytes without writing
        // them anywhere, leaving the verifier's stored key untouched.
        let stored = verifier.get_verifying_key(&CircuitType::Shield.into());
        assert_eq!(stored, new_vk);
        assert_ne!(stored, original_vk);
    }

    #[test]
    fn cancel_vk_update_prevents_execution() {
        let (env, _admin, governance_id, verifier_id) = setup();
        let gov = ZKELLAGovernanceClient::new(&env, &governance_id);
        let verifier = VerifierContractClient::new(&env, &verifier_id);

        let original_vk = vk_bytes(&env, 512);
        queue_and_execute(&env, &gov, CircuitType::Shield, &original_vk);

        let new_vk = vk_bytes(&env, 576);
        gov.queue_vk_update(&CircuitType::Shield, &new_vk);
        gov.cancel_vk_update(&CircuitType::Shield);

        env.ledger().with_mut(|li| {
            li.sequence_number += VK_TIMELOCK_LEDGERS;
        });

        let result = gov.try_execute_vk_update(&CircuitType::Shield);
        assert!(result.is_err());

        let stored = verifier.get_verifying_key(&CircuitType::Shield.into());
        assert_eq!(stored, original_vk);
    }

    #[test]
    #[should_panic(expected = "paused")]
    fn pause_blocks_queue_vk_update() {
        let (env, _admin, _guardian, gov_id, _verifier) = setup_with_guardian();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        gov.pause();
        gov.queue_vk_update(&CircuitType::Shield, &vk_bytes(&env, 768));
    }

    #[test]
    #[should_panic(expected = "paused")]
    fn pause_blocks_execute_vk_update() {
        let (env, _admin, _guardian, gov_id, _verifier) = setup_with_guardian();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        gov.queue_vk_update(&CircuitType::Shield, &vk_bytes(&env, 768));
        env.ledger().with_mut(|li| { li.sequence_number += VK_TIMELOCK_LEDGERS; });
        gov.pause();
        gov.execute_vk_update(&CircuitType::Shield);
    }

    #[test]
    #[should_panic(expected = "paused")]
    fn pause_blocks_transfer_admin() {
        let (env, _admin, _guardian, gov_id, _verifier) = setup_with_guardian();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        gov.pause();
        gov.transfer_admin(&Address::generate(&env));
    }

    #[test]
    #[should_panic(expected = "paused")]
    fn pause_blocks_accept_admin() {
        let (env, _admin, _guardian, gov_id, _verifier) = setup_with_guardian();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        gov.transfer_admin(&Address::generate(&env));
        gov.pause();
        gov.accept_admin();
    }

    #[test]
    fn pause_does_not_block_cancel_or_revoke() {
        let (env, _admin, _guardian, gov_id, _verifier) = setup_with_guardian();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        gov.queue_vk_update(&CircuitType::Shield, &vk_bytes(&env, 768));
        gov.pause();
        gov.cancel_vk_update(&CircuitType::Shield);
        gov.revoke_previous_vk(&CircuitType::Shield);
    }

    #[test]
    fn guardian_can_cancel_a_queued_update_even_while_paused() {
        let (env, _admin, _guardian, gov_id, _verifier) = setup_with_guardian();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        gov.queue_vk_update(&CircuitType::Shield, &vk_bytes(&env, 768));
        gov.pause();
        gov.guardian_cancel_vk_update(&CircuitType::Shield);
        env.ledger().with_mut(|li| { li.sequence_number += VK_TIMELOCK_LEDGERS; });
        assert!(gov.try_execute_vk_update(&CircuitType::Shield).is_err(), "cancelled update must not execute");
    }

    #[test]
    fn unpause_restores_queue_vk_update() {
        let (env, _admin, _guardian, gov_id, _verifier) = setup_with_guardian();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        gov.pause();
        gov.unpause();
        gov.queue_vk_update(&CircuitType::Shield, &vk_bytes(&env, 768));
    }

    #[test]
    fn min_shield_amount_changes_on_the_real_token_only_after_the_timelock() {
        let (env, _admin, _guardian, gov_id, _verifier, token_id) = setup_full();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        let token = zkella_token::ShieldedTokenClient::new(&env, &token_id);
        let before = token.min_shield_amount();

        gov.queue_token_action(&TokenAdminAction::MinShieldAmount(before * 5));
        assert!(gov.try_execute_token_action().is_err(), "must not execute before the timelock");
        assert_eq!(token.min_shield_amount(), before);

        env.ledger().with_mut(|li| { li.sequence_number += VK_TIMELOCK_LEDGERS; });
        gov.execute_token_action();
        assert_eq!(token.min_shield_amount(), before * 5);
    }

    #[test]
    #[should_panic(expected = "amount must be positive")]
    fn queue_token_action_rejects_non_positive_min_shield() {
        let (env, _admin, _guardian, gov_id, _verifier, _token) = setup_full();
        ZKELLAGovernanceClient::new(&env, &gov_id).queue_token_action(&TokenAdminAction::MinShieldAmount(0));
    }

    #[test]
    #[should_panic(expected = "paused")]
    fn pause_blocks_queue_token_action() {
        let (env, _admin, _guardian, gov_id, _verifier, _token) = setup_full();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        gov.pause();
        gov.queue_token_action(&TokenAdminAction::MinShieldAmount(1_000));
    }

    #[test]
    fn guardian_can_cancel_a_queued_min_shield_update() {
        let (env, _admin, _guardian, gov_id, _verifier, token_id) = setup_full();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        let token = zkella_token::ShieldedTokenClient::new(&env, &token_id);
        let before = token.min_shield_amount();

        gov.queue_token_action(&TokenAdminAction::MinShieldAmount(before * 5));
        gov.guardian_cancel_token_action();
        env.ledger().with_mut(|li| { li.sequence_number += VK_TIMELOCK_LEDGERS; });
        assert!(gov.try_execute_token_action().is_err(), "cancelled update must not execute");
        assert_eq!(token.min_shield_amount(), before);
    }

    #[test]
    fn pause_unpause_and_guardian_cancel_require_authorization() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let verifier = Address::generate(&env);
        let token = Address::generate(&env);
        let gov_id = env.register(ZKELLAGovernance, ());
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        gov.initialize(&admin, &verifier, &guardian, &token);
        assert!(gov.try_pause().is_err(), "pause must require admin authorization");
        assert!(gov.try_unpause().is_err(), "unpause must require admin authorization");
        assert!(
            gov.try_guardian_cancel_vk_update(&CircuitType::Shield).is_err(),
            "guardian cancel must require guardian authorization"
        );
    }

    /// Regression test for the audit finding that `verifier.pause()`/
    /// `unpause()` were unreachable: the verifier's admin is this contract's
    /// own address, so only a direct call from governance's own code can
    /// ever satisfy `admin.require_auth()` there. Before `pause_verifier`/
    /// `unpause_verifier` existed, nothing in this contract made that call,
    /// so the runbook's documented "pause the verifier" incident lever
    /// could never actually be exercised by anyone, admin or otherwise.
    #[test]
    fn pause_verifier_actually_pauses_and_unpause_verifier_restores_it() {
        let (env, _admin, gov_id, verifier_id) = setup();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        let verifier = VerifierContractClient::new(&env, &verifier_id);
        let vk = vk_bytes(&env, 512);

        gov.pause_verifier();
        assert_eq!(
            verifier.try_register_verifying_key(&CircuitType::Shield.into(), &vk),
            Err(Ok(zkella_verifier::Error::Paused)),
            "pause_verifier must actually pause the verifier contract it administers"
        );

        gov.unpause_verifier();
        assert_eq!(
            verifier.try_register_verifying_key(&CircuitType::Shield.into(), &vk),
            Ok(Ok(())),
            "unpause_verifier must restore normal verifier operation"
        );
    }

    #[test]
    fn pause_verifier_and_unpause_verifier_require_admin_authorization() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let verifier = Address::generate(&env);
        let token = Address::generate(&env);
        let gov_id = env.register(ZKELLAGovernance, ());
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        gov.initialize(&admin, &verifier, &guardian, &token);
        assert!(gov.try_pause_verifier().is_err(), "pause_verifier must require admin authorization");
        assert!(gov.try_unpause_verifier().is_err(), "unpause_verifier must require admin authorization");
    }

    /// Same reachability gap as `pause_verifier`'s own test, for `token`:
    /// its admin is also this contract's own address (see `setup_full`), so
    /// without `pause_token` nothing could ever satisfy `token::pause`'s
    /// `admin.require_auth()`. Dummy, deliberately-invalid arguments are
    /// enough here: `shield`'s pause check runs immediately after
    /// `require_auth`, before any asset-approval or proof check, so a
    /// `Paused` result (and only that result) proves the flag, not the
    /// argument values, is what's being exercised.
    #[test]
    fn pause_token_actually_pauses_and_unpause_token_restores_it() {
        let (env, _admin, _guardian, gov_id, _verifier_id, token_id) = setup_full();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        let token = zkella_token::ShieldedTokenClient::new(&env, &token_id);
        let from = Address::generate(&env);
        let asset = Address::generate(&env);
        let dummy32 = BytesN::from_array(&env, &[0u8; 32]);
        let shield_pub = zkella_token::ShieldPublicInputs {
            commitment: dummy32.clone(), value_commit: dummy32.clone(), pub_value: 0, pub_asset_id: asset.clone(),
        };

        gov.pause_token();
        assert_eq!(
            token.try_shield(
                &from, &asset, &0, &dummy32, &dummy32, &dummy32, &dummy32,
                &Bytes::new(&env), &Bytes::new(&env), &shield_pub,
            ),
            Err(Ok(zkella_token::Error::Paused)),
            "pause_token must actually pause the token contract it administers"
        );

        gov.unpause_token();
        assert_ne!(
            token.try_shield(
                &from, &asset, &0, &dummy32, &dummy32, &dummy32, &dummy32,
                &Bytes::new(&env), &Bytes::new(&env), &shield_pub,
            ),
            Err(Ok(zkella_token::Error::Paused)),
            "unpause_token must restore normal operation (no longer rejected merely for being paused)"
        );
    }

    #[test]
    fn pause_token_and_unpause_token_require_admin_authorization() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let verifier = Address::generate(&env);
        let token = Address::generate(&env);
        let gov_id = env.register(ZKELLAGovernance, ());
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        gov.initialize(&admin, &verifier, &guardian, &token);
        assert!(gov.try_pause_token().is_err(), "pause_token must require admin authorization");
        assert!(gov.try_unpause_token().is_err(), "unpause_token must require admin authorization");
    }

    #[test]
    fn unpause_restores_execute_vk_update() {
        let (env, _admin, _guardian, gov_id, _verifier) = setup_with_guardian();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        gov.queue_vk_update(&CircuitType::Shield, &vk_bytes(&env, 768));
        gov.pause();
        gov.unpause();
        env.ledger().with_mut(|li| { li.sequence_number += VK_TIMELOCK_LEDGERS; });
        gov.execute_vk_update(&CircuitType::Shield);
    }

    #[test]
    fn unpause_restores_transfer_and_accept_admin() {
        let (env, _admin, _guardian, gov_id, _verifier) = setup_with_guardian();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        let new_admin = Address::generate(&env);
        gov.pause();
        gov.unpause();
        gov.transfer_admin(&new_admin);
        gov.accept_admin();
    }

    #[test]
    fn unpause_restores_queue_and_execute_token_action() {
        let (env, _admin, _guardian, gov_id, _verifier, token_id) = setup_full();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        let token = zkella_token::ShieldedTokenClient::new(&env, &token_id);
        gov.pause();
        gov.unpause();
        let target = token.min_shield_amount() * 2;
        gov.queue_token_action(&TokenAdminAction::MinShieldAmount(target));
        env.ledger().with_mut(|li| { li.sequence_number += VK_TIMELOCK_LEDGERS; });
        gov.execute_token_action();
        assert_eq!(token.min_shield_amount(), target);
    }

    #[test]
    fn governance_approves_an_asset_and_a_relayer_on_the_token_after_the_timelock() {
        let (env, _admin, _guardian, gov_id, _verifier, token_id) = setup_full();
        let gov = ZKELLAGovernanceClient::new(&env, &gov_id);
        let token = zkella_token::ShieldedTokenClient::new(&env, &token_id);
        let asset = Address::generate(&env);
        let relayer = Address::generate(&env);

        gov.queue_token_action(&TokenAdminAction::AssetApproval(asset.clone(), true));
        env.ledger().with_mut(|li| { li.sequence_number += VK_TIMELOCK_LEDGERS; });
        gov.execute_token_action();
        assert!(token.is_asset_approved(&asset));

        gov.queue_token_action(&TokenAdminAction::Relayer(relayer.clone(), true));
        env.ledger().with_mut(|li| { li.sequence_number += VK_TIMELOCK_LEDGERS; });
        gov.execute_token_action();
        assert!(token.is_approved_relayer(&relayer));
    }
}
