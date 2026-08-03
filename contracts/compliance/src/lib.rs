#![no_std]

//! Sanctions/compliance non-membership proof registry.
//!
//! Split out of `contracts/viewing_keys`, which used to store both viewing-key
//! commitments and unverified compliance-proof blobs in one contract — two
//! unrelated concerns with different lifecycles and, arguably, different
//! access-control needs. This contract owns compliance records only, and —
//! unlike the previous design — actually verifies the non-membership proof
//! against `CircuitType::NonMembership` (via `contracts/verifier`) before
//! storing it, rather than accepting an opaque, unchecked blob.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short,
    Address, Bytes, BytesN, Env, String, Vec,
};
use zkella_verifier_interface::{CircuitType, VerifierClient};

#[contracttype]
pub enum StorageKey {
    Verifier,
    ComplianceRecord(Address),
}

#[contracttype]
#[derive(Clone)]
pub struct ComplianceRecord {
    pub sanctions_root:   BytesN<32>,
    pub tk_commitment:    BytesN<32>,
    pub published_ledger: u32,
    pub version:          String,
}

/// Public inputs for `circuits/compliance/non_membership.circom`:
/// `component main {public [sanctions_root, tk_commitment]}`.
#[contracttype]
#[derive(Clone)]
pub struct CompliancePublicInputs {
    pub sanctions_root: BytesN<32>,
    pub tk_commitment:  BytesN<32>,
}

#[contracterror]
#[derive(Clone, Copy, PartialEq, Debug)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    InvalidProof        = 3,
}

#[contract]
pub struct ComplianceContract;

#[contractimpl]
impl ComplianceContract {
    pub fn initialize(env: Env, verifier: Address) {
        if env.storage().instance().has(&StorageKey::Verifier) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Verifier, &verifier);
    }

    /// Publishes a verified sanctions-list non-membership proof for `owner`.
    /// The proof is checked against the verifier's `CircuitType::NonMembership`
    /// key before anything is stored — the previous design stored `proof` as
    /// an opaque blob with a `// Full Groth16 verification in M2` comment and
    /// never checked it.
    pub fn publish_compliance_proof(
        env:        Env,
        owner:      Address,
        proof:      Bytes,
        pub_inputs: CompliancePublicInputs,
    ) -> Result<(), Error> {
        owner.require_auth();

        let verifier: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Verifier)
            .ok_or(Error::NotInitialized)?;

        let public_inputs = Vec::from_array(
            &env,
            [pub_inputs.sanctions_root.clone(), pub_inputs.tk_commitment.clone()],
        );
        let proof_ok = VerifierClient::new(&env, &verifier).verify(
            &CircuitType::NonMembership,
            &public_inputs,
            &proof,
        );
        if !proof_ok {
            return Err(Error::InvalidProof);
        }

        let record = ComplianceRecord {
            sanctions_root:   pub_inputs.sanctions_root,
            tk_commitment:    pub_inputs.tk_commitment,
            published_ledger: env.ledger().sequence(),
            version:          String::from_str(&env, "1.0"),
        };
        env.storage().instance().set(&StorageKey::ComplianceRecord(owner.clone()), &record);
        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("comply")),
            owner,
        );
        Ok(())
    }

    pub fn get_compliance_proof(env: Env, owner: Address) -> Option<ComplianceRecord> {
        env.storage().instance().get(&StorageKey::ComplianceRecord(owner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    use ark_bn254::{Fq, Fq2, Fr, G1Affine, G1Projective, G2Affine};
    use ark_ec::{AffineRepr, CurveGroup};
    use ark_ff::{BigInteger, PrimeField};
    use ark_std::UniformRand;

    // Same construction technique as zkella-verifier's and ct20's own test
    // suites: pick beta = gamma = delta = H (standard G2 generator), choose
    // alpha/IC freely, set A = alpha + vk_x + C via real curve arithmetic.
    // NOT all-zero bytes: an all-identity VK/proof is a degenerate case that
    // trivially satisfies the pairing check (e(O,O)=1), which would make a
    // "should reject" test built that way silently pass for the wrong
    // reason — this bit the first version of this file's tests.

    fn fq_be(f: &Fq) -> [u8; 32] {
        let mut out = [0u8; 32];
        let be = f.into_bigint().to_bytes_be();
        out[32 - be.len()..].copy_from_slice(&be);
        out
    }
    fn g1_bytes(p: &G1Affine) -> [u8; 64] {
        let mut out = [0u8; 64];
        out[0..32].copy_from_slice(&fq_be(&p.x));
        out[32..64].copy_from_slice(&fq_be(&p.y));
        out
    }
    fn fq2_be(f: &Fq2) -> [u8; 64] {
        let mut out = [0u8; 64];
        out[0..32].copy_from_slice(&fq_be(&f.c1));
        out[32..64].copy_from_slice(&fq_be(&f.c0));
        out
    }
    fn g2_bytes(p: &G2Affine) -> [u8; 128] {
        let mut out = [0u8; 128];
        out[0..64].copy_from_slice(&fq2_be(&p.x));
        out[64..128].copy_from_slice(&fq2_be(&p.y));
        out
    }
    fn fr_from_le(le: &[u8; 32]) -> Fr {
        let mut be = *le;
        be.reverse();
        Fr::from_be_bytes_mod_order(&be)
    }

    /// Builds a self-consistent (VK, proof) for 2 public inputs
    /// (sanctions_root, tk_commitment), plus a corrupted variant of the same
    /// proof (C perturbed) that must fail verification.
    fn build_proofs(env: &Env, public_inputs_le: [[u8; 32]; 2]) -> (Bytes, Bytes, Bytes) {
        let mut rng = ark_std::test_rng();
        let g1 = G1Affine::generator();
        let h = G2Affine::generator();

        let alpha_s = Fr::rand(&mut rng);
        let ic_s: [Fr; 3] = core::array::from_fn(|_| Fr::rand(&mut rng));
        let c_s = Fr::rand(&mut rng);

        let alpha: G1Projective = g1 * alpha_s;
        let ic: [G1Projective; 3] = core::array::from_fn(|i| g1 * ic_s[i]);
        let mut vk_x: G1Projective = ic[0];
        for i in 0..2 {
            vk_x += ic[i + 1] * fr_from_le(&public_inputs_le[i]);
        }
        let c: G1Projective = g1 * c_s;
        let a: G1Projective = alpha + vk_x + c;

        let mut vk = ark_std::vec::Vec::new();
        vk.extend_from_slice(&g1_bytes(&alpha.into_affine()));
        vk.extend_from_slice(&g2_bytes(&h));
        vk.extend_from_slice(&g2_bytes(&h));
        vk.extend_from_slice(&g2_bytes(&h));
        for p in ic.iter() {
            vk.extend_from_slice(&g1_bytes(&p.into_affine()));
        }

        let mut proof = ark_std::vec::Vec::new();
        proof.extend_from_slice(&g1_bytes(&a.into_affine()));
        proof.extend_from_slice(&g2_bytes(&h));
        proof.extend_from_slice(&g1_bytes(&c.into_affine()));

        let corrupted_c: G1Projective = c + g1;
        let mut bad_proof = ark_std::vec::Vec::new();
        bad_proof.extend_from_slice(&g1_bytes(&a.into_affine()));
        bad_proof.extend_from_slice(&g2_bytes(&h));
        bad_proof.extend_from_slice(&g1_bytes(&corrupted_c.into_affine()));

        (
            Bytes::from_slice(env, &vk),
            Bytes::from_slice(env, &proof),
            Bytes::from_slice(env, &bad_proof),
        )
    }

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let owner    = Address::generate(&env);
        let contract = env.register(ComplianceContract, ());
        let verifier = env.register(zkella_verifier::VerifierContract, ());
        zkella_verifier::VerifierContractClient::new(&env, &verifier).initialize(&owner);
        ComplianceContractClient::new(&env, &contract).initialize(&verifier);
        (env, owner, contract, verifier)
    }

    #[test]
    fn rejects_when_no_vk_registered() {
        let (env, owner, contract, _verifier) = setup();
        let client = ComplianceContractClient::new(&env, &contract);

        let garbage_proof = Bytes::from_array(&env, &[0u8; 10]);
        let pub_inputs = CompliancePublicInputs {
            sanctions_root: BytesN::from_array(&env, &[1u8; 32]),
            tk_commitment:  BytesN::from_array(&env, &[2u8; 32]),
        };
        let result = client.try_publish_compliance_proof(&owner, &garbage_proof, &pub_inputs);
        assert!(result.is_err());
        assert!(client.get_compliance_proof(&owner).is_none());
    }

    #[test]
    fn accepts_and_stores_genuine_proof() {
        let (env, owner, contract, verifier) = setup();
        let client = ComplianceContractClient::new(&env, &contract);

        let sanctions_root = BytesN::from_array(&env, &[3u8; 32]);
        let tk_commitment  = BytesN::from_array(&env, &[4u8; 32]);
        let pub_inputs = CompliancePublicInputs {
            sanctions_root: sanctions_root.clone(),
            tk_commitment:  tk_commitment.clone(),
        };
        let public_inputs_le: [[u8; 32]; 2] = [
            sanctions_root.into(),
            tk_commitment.into(),
        ];
        let (vk, proof, _bad_proof) = build_proofs(&env, public_inputs_le);

        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::NonMembership.into(), &vk);

        client.publish_compliance_proof(&owner, &proof, &pub_inputs);

        let stored = client.get_compliance_proof(&owner).unwrap();
        assert_eq!(stored.sanctions_root, pub_inputs.sanctions_root);
        assert_eq!(stored.tk_commitment, pub_inputs.tk_commitment);
    }

    #[test]
    fn rejects_tampered_proof_and_stores_nothing() {
        let (env, owner, contract, verifier) = setup();
        let client = ComplianceContractClient::new(&env, &contract);

        let sanctions_root = BytesN::from_array(&env, &[5u8; 32]);
        let tk_commitment  = BytesN::from_array(&env, &[6u8; 32]);
        let pub_inputs = CompliancePublicInputs {
            sanctions_root: sanctions_root.clone(),
            tk_commitment:  tk_commitment.clone(),
        };
        let public_inputs_le: [[u8; 32]; 2] = [
            sanctions_root.into(),
            tk_commitment.into(),
        ];
        let (vk, _valid_proof, bad_proof) = build_proofs(&env, public_inputs_le);

        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::NonMembership.into(), &vk);

        let result = client.try_publish_compliance_proof(&owner, &bad_proof, &pub_inputs);
        assert!(result.is_err());
        assert!(client.get_compliance_proof(&owner).is_none());
    }
}
