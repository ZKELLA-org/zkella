#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    Address, Bytes, Env,
};

const VK_TIMELOCK_LEDGERS: u32 = 120_960; // 7 days at 5s/ledger

#[contracttype]
pub enum StorageKey {
    Admin,
    PendingAdmin,
    PendingVkUpdate(u32),
}

#[contracttype]
pub struct PendingVkUpdate {
    pub circuit_id:    u32,
    pub new_vk:        Bytes,
    pub eta_ledger:    u32,
}

#[contract]
pub struct ZKELLAGovernance;

#[contractimpl]
impl ZKELLAGovernance {

    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
    }

    /// Queue a verifying key update — enforces 7-day timelock
    pub fn queue_vk_update(env: Env, circuit_id: u32, new_vk: Bytes) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();

        let eta = env.ledger().sequence() + VK_TIMELOCK_LEDGERS;
        let update = PendingVkUpdate { circuit_id, new_vk, eta_ledger: eta };
        env.storage().instance().set(&StorageKey::PendingVkUpdate(circuit_id), &update);

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("vkqueue")),
            (circuit_id, eta),
        );
    }

    /// Execute a queued VK update after the timelock has passed
    pub fn execute_vk_update(env: Env, circuit_id: u32) -> Bytes {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();

        let update: PendingVkUpdate = env.storage().instance()
            .get(&StorageKey::PendingVkUpdate(circuit_id))
            .expect("no pending update");
        assert!(env.ledger().sequence() >= update.eta_ledger, "timelock not elapsed");

        env.storage().instance().remove(&StorageKey::PendingVkUpdate(circuit_id));

        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("vkexec")),
            circuit_id,
        );

        update.new_vk
    }

    /// Cancel a queued VK update before it is executed
    pub fn cancel_vk_update(env: Env, circuit_id: u32) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        env.storage().instance().remove(&StorageKey::PendingVkUpdate(circuit_id));
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
