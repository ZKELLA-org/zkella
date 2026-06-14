#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype,
    symbol_short, Address, Bytes, BytesN, Env,
};

#[contracttype]
pub enum StorageKey {
    ViewingKeyCommitment(Address),
    ComplianceRecord(Address),
}

#[contracttype]
pub struct ComplianceRecord {
    pub sanctions_root:  BytesN<32>,
    pub proof:           Bytes,
    pub published_ledger: u32,
    pub version:         soroban_sdk::String,
}

#[contracttype]
pub struct CompliancePublicInputs {
    pub sanctions_root: BytesN<32>,
    pub tk_commitment:  BytesN<32>,
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

    pub fn publish_compliance_proof(
        env:             Env,
        owner:           Address,
        sanctions_root:  BytesN<32>,
        proof:           Bytes,
        _pub_inputs:     CompliancePublicInputs,
    ) {
        owner.require_auth();

        // Verify non-membership proof
        // Full Groth16 verification in M2
        let record = ComplianceRecord {
            sanctions_root,
            proof,
            published_ledger: env.ledger().sequence(),
            version: soroban_sdk::String::from_str(&env, "1.0"),
        };
        env.storage().instance().set(&StorageKey::ComplianceRecord(owner), &record);
    }

    pub fn get_compliance_proof(env: Env, owner: Address) -> Option<ComplianceRecord> {
        env.storage().instance().get(&StorageKey::ComplianceRecord(owner))
    }
}
