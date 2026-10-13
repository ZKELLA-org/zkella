#![no_std]
// See contracts/token/src/lib.rs for why the `publish` deprecation is
// deferred rather than migrated right now.
#![allow(deprecated)]

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

    /// Registers or replaces `owner`'s viewing-key commitment. Registering a new
    /// commitment is how an owner rotates to a fresh viewing key.
    pub fn register(
        env:           Env,
        owner:         Address,
        vk_commitment: BytesN<32>,
        birthday:      u32,
    ) {
        owner.require_auth();
        let key = StorageKey::ViewingKeyCommitment(owner.clone());
        env.storage().persistent().set(&key, &vk_commitment);
        env.storage().persistent().extend_ttl(&key, STATE_TTL_THRESHOLD, STATE_TTL_EXTEND_TO);
        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("vkreg")),
            (owner, vk_commitment, birthday),
        );
    }

    /// Removes `owner`'s commitment. Revoking does not change what a holder of
    /// an already-granted viewing key can decrypt; it withdraws the registry's
    /// advertised commitment, and rotating to a new key is what cuts off notes
    /// encrypted after the rotation.
    pub fn revoke(env: Env, owner: Address) {
        owner.require_auth();
        env.storage().persistent().remove(&StorageKey::ViewingKeyCommitment(owner.clone()));
        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("vkrev")),
            owner,
        );
    }

    pub fn get_viewing_key_commitment(env: Env, owner: Address) -> Option<BytesN<32>> {
        env.storage().persistent().get(&StorageKey::ViewingKeyCommitment(owner))
    }
}

const STATE_TTL_THRESHOLD: u32 = 17_280 * 30;
const STATE_TTL_EXTEND_TO: u32 = 17_280 * 365;

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

    #[test]
    fn revoke_removes_the_registered_commitment() {
        let env = Env::default();
        env.mock_all_auths();
        let owner = Address::generate(&env);
        let contract = env.register(ViewingKeyRegistry, ());
        let client = ViewingKeyRegistryClient::new(&env, &contract);

        client.register(&owner, &BytesN::from_array(&env, &[7u8; 32]), &100);
        client.revoke(&owner);

        assert_eq!(client.get_viewing_key_commitment(&owner), None);
    }

    #[test]
    fn rotating_by_registering_a_new_commitment_replaces_the_old_one() {
        let env = Env::default();
        env.mock_all_auths();
        let owner = Address::generate(&env);
        let contract = env.register(ViewingKeyRegistry, ());
        let client = ViewingKeyRegistryClient::new(&env, &contract);

        client.register(&owner, &BytesN::from_array(&env, &[7u8; 32]), &100);
        let rotated = BytesN::from_array(&env, &[8u8; 32]);
        client.register(&owner, &rotated, &200);

        assert_eq!(client.get_viewing_key_commitment(&owner), Some(rotated));
    }

    #[test]
    fn register_and_revoke_require_the_owners_authorization() {
        let env = Env::default();
        let owner = Address::generate(&env);
        let contract = env.register(ViewingKeyRegistry, ());
        let client = ViewingKeyRegistryClient::new(&env, &contract);

        let commitment = BytesN::from_array(&env, &[7u8; 32]);
        assert!(client.try_register(&owner, &commitment, &100).is_err(), "register must require owner auth");
        assert!(client.try_revoke(&owner).is_err(), "revoke must require owner auth");
    }
}
