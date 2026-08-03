#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    Address, Bytes, Env,
};
use zkella_verifier_interface::{CircuitType, VerifierClient};

const VK_TIMELOCK_LEDGERS: u32 = 120_960; // 7 days at 5s/ledger

#[contracttype]
pub enum StorageKey {
    Admin,
    Verifier, // address of the zkella-verifier registry this governance contract administers
    PendingAdmin,
    PendingVkUpdate(CircuitType),
}

#[contracttype]
pub struct PendingVkUpdate {
    pub circuit:    CircuitType,
    pub new_vk:     Bytes,
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
    pub fn initialize(env: Env, admin: Address, verifier: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage().instance().set(&StorageKey::Verifier, &verifier);
    }

    /// Registers a verifying key for the first time. No timelock: this
    /// establishes initial state rather than replacing a key already relied
    /// upon, so it doesn't carry the same soundness risk as `execute_vk_update`.
    pub fn register_vk(env: Env, circuit: CircuitType, vk: Bytes) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();

        let verifier: Address = env.storage().instance().get(&StorageKey::Verifier).unwrap();
        VerifierClient::new(&env, &verifier).register_verifying_key(&circuit, &vk);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("vkreg")),
            circuit,
        );
    }

    /// Queue a verifying key update — enforces 7-day timelock
    pub fn queue_vk_update(env: Env, circuit: CircuitType, new_vk: Bytes) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();

        let eta = env.ledger().sequence() + VK_TIMELOCK_LEDGERS;
        let update = PendingVkUpdate { circuit, new_vk, eta_ledger: eta };
        env.storage().instance().set(&StorageKey::PendingVkUpdate(circuit), &update);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("vkqueue")),
            (circuit, eta),
        );
    }

    /// Execute a queued VK update after the timelock has passed. Actually
    /// rotates the key in the verifier registry — this used to just return
    /// the bytes without writing them anywhere, leaving governance and the
    /// verifier disconnected.
    pub fn execute_vk_update(env: Env, circuit: CircuitType) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();

        let update: PendingVkUpdate = env.storage().instance()
            .get(&StorageKey::PendingVkUpdate(circuit))
            .expect("no pending update");
        assert!(env.ledger().sequence() >= update.eta_ledger, "timelock not elapsed");

        env.storage().instance().remove(&StorageKey::PendingVkUpdate(circuit));

        let verifier: Address = env.storage().instance().get(&StorageKey::Verifier).unwrap();
        VerifierClient::new(&env, &verifier)
            .update_verifying_key(&circuit, &update.new_vk);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("vkexec")),
            circuit,
        );
    }

    /// Cancel a queued VK update before it is executed
    pub fn cancel_vk_update(env: Env, circuit: CircuitType) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        env.storage().instance().remove(&StorageKey::PendingVkUpdate(circuit));
    }

    pub fn transfer_admin(env: Env, new_admin: Address) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        env.storage().instance().set(&StorageKey::PendingAdmin, &new_admin);
    }

    pub fn accept_admin(env: Env) {
        let pending: Address = env.storage().instance()
            .get(&StorageKey::PendingAdmin).expect("no pending admin");
        pending.require_auth();
        env.storage().instance().set(&StorageKey::Admin, &pending);
        env.storage().instance().remove(&StorageKey::PendingAdmin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use zkella_verifier::{VerifierContract, VerifierContractClient};

    /// Deploys governance + verifier wired together, with governance's own
    /// contract address as the verifier's admin (per this module's contract).
    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let governance_id = env.register(ZKELLAGovernance, ());
        let verifier_id = env.register(VerifierContract, ());

        VerifierContractClient::new(&env, &verifier_id).initialize(&governance_id);
        ZKELLAGovernanceClient::new(&env, &governance_id).initialize(&admin, &verifier_id);

        (env, admin, governance_id, verifier_id)
    }

    fn vk_bytes(env: &Env, len: u32) -> Bytes {
        let mut b = Bytes::new(env);
        for i in 0..len {
            b.push_back((i % 256) as u8);
        }
        b
    }

    #[test]
    fn register_vk_reaches_the_verifier_contract() {
        let (env, _admin, governance_id, verifier_id) = setup();
        let gov = ZKELLAGovernanceClient::new(&env, &governance_id);
        let verifier = VerifierContractClient::new(&env, &verifier_id);

        // VK_FIXED_LEN (448) + one IC point (64) = 512, valid shape for 0 public inputs.
        let vk = vk_bytes(&env, 512);
        gov.register_vk(&CircuitType::Shield, &vk);

        let stored = verifier.get_verifying_key(&CircuitType::Shield.into());
        assert_eq!(stored, vk);
    }

    #[test]
    fn execute_vk_update_actually_rotates_the_verifier_key() {
        let (env, _admin, governance_id, verifier_id) = setup();
        let gov = ZKELLAGovernanceClient::new(&env, &governance_id);
        let verifier = VerifierContractClient::new(&env, &verifier_id);

        let original_vk = vk_bytes(&env, 512);
        gov.register_vk(&CircuitType::Shield, &original_vk);

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
        gov.register_vk(&CircuitType::Shield, &original_vk);

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
}
