#![no_std]

//! Viewing-key commitment registry.
//!
//! Scoped to viewing-key commitments only. Compliance/sanctions
//! non-membership proofs used to be stored here too, under an unrelated
//! `ComplianceRecord` key with no verification (`// Full Groth16
//! verification in M2`) — that's now `contracts/compliance`, which actually
//! verifies proofs against `contracts/verifier` before storing them. Two
//! concerns with different lifecycles and access-control needs belong in two
//! contracts.

use soroban_sdk::{
    contract, contractimpl, contracttype,
    symbol_short, Address, BytesN, Env,
};

#[contracttype]
pub enum StorageKey {
    ViewingKeyCommitment(Address),
}

#[contract]
pub struct ViewingKeyRegistry;

#[contractimpl]
impl ViewingKeyRegistry {

    pub fn register(
        env:           Env,
        owner:         Address,
        vk_commitment: BytesN<32>,
        birthday:      u32,
    ) {
        owner.require_auth();
        env.storage().instance().set(&StorageKey::ViewingKeyCommitment(owner.clone()), &vk_commitment);
        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("vkreg")),
            (owner, vk_commitment, birthday),
        );
    }

    pub fn get_viewing_key_commitment(env: Env, owner: Address) -> Option<BytesN<32>> {
        env.storage().instance().get(&StorageKey::ViewingKeyCommitment(owner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn register_then_get_roundtrips() {
        let env = Env::default();
        env.mock_all_auths();
        let owner = Address::generate(&env);
        let contract = env.register(ViewingKeyRegistry, ());
        let client = ViewingKeyRegistryClient::new(&env, &contract);

        let vk_commitment = BytesN::from_array(&env, &[7u8; 32]);
        client.register(&owner, &vk_commitment, &100);

        assert_eq!(client.get_viewing_key_commitment(&owner), Some(vk_commitment));
    }

    #[test]
    fn get_returns_none_for_unregistered_owner() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let contract = env.register(ViewingKeyRegistry, ());
        let client = ViewingKeyRegistryClient::new(&env, &contract);

        assert_eq!(client.get_viewing_key_commitment(&owner), None);
    }
}
