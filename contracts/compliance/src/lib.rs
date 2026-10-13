#![no_std]
// See contracts/token/src/lib.rs for why the `publish` deprecation is
// deferred rather than migrated right now.
#![allow(deprecated)]

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
    Admin,
    Verifier,
    Paused,
    SanctionsRoot,
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
    Paused             = 4,
    UnknownSanctionsRoot = 5,
}

#[contract]
pub struct ComplianceContract;

#[contractimpl]
impl ComplianceContract {
    pub fn initialize(env: Env, admin: Address, verifier: Address) {
        if env.storage().instance().has(&StorageKey::Verifier) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage().instance().set(&StorageKey::Verifier, &verifier);
    }

    /// Sets the sanctions-list root that `publish_compliance_proof` will accept.
    /// Only the admin (the list maintainer) can change it, so a proof computed
    /// against any other root, including one the prover chose, is rejected.
    pub fn set_sanctions_root(env: Env, root: BytesN<32>) {
        let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
        admin.require_auth();
        env.storage().instance().set(&StorageKey::SanctionsRoot, &root);
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
        let paused: bool = env.storage().instance().get(&StorageKey::Paused).unwrap_or(false);
        if paused {
            return Err(Error::Paused);
        }

        let authorized_root: BytesN<32> = env
            .storage()
            .instance()
            .get(&StorageKey::SanctionsRoot)
            .ok_or(Error::UnknownSanctionsRoot)?;
        if pub_inputs.sanctions_root != authorized_root {
            return Err(Error::UnknownSanctionsRoot);
        }

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
        // One entry per owner, each with its own TTL — instance storage is a
        // single size-capped entry loaded on every call and would let the
        // record set grow without bound.
        let key = StorageKey::ComplianceRecord(owner.clone());
        env.storage().persistent().set(&key, &record);
        env.storage().persistent().extend_ttl(&key, 17_280 * 30, 17_280 * 365);
        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("comply")),
            owner,
        );
        Ok(())
    }

    pub fn get_compliance_proof(env: Env, owner: Address) -> Option<ComplianceRecord> {
        env.storage().persistent().get(&StorageKey::ComplianceRecord(owner))
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

    // Same construction technique as zkella-verifier's and token's own test
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
        ComplianceContractClient::new(&env, &contract).initialize(&owner, &verifier);
        (env, owner, contract, verifier)
    }

    const COMPLIANCE_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zkella_compliance.wasm");
    const VERIFIER_WASM: &[u8] = include_bytes!("../../target/wasm32v1-none/release/zkella_verifier.wasm");

    /// Instruction cost of one `publish_compliance_proof` (on-chain Groth16 verification plus
    /// storage), on the native contracts or on the compiled WASM artefacts.
    fn publish_cost(wasm: bool) -> u64 {
        extern crate std;
        let env = Env::default();
        env.mock_all_auths();
        env.cost_estimate().budget().reset_limits(2_000_000_000, 100_000_000);
        let owner = Address::generate(&env);
        let (contract, verifier) = if wasm {
            (env.register(COMPLIANCE_WASM, ()), env.register(VERIFIER_WASM, ()))
        } else {
            (env.register(ComplianceContract, ()), env.register(zkella_verifier::VerifierContract, ()))
        };
        zkella_verifier::VerifierContractClient::new(&env, &verifier).initialize(&owner);
        let client = ComplianceContractClient::new(&env, &contract);
        client.initialize(&owner, &verifier);
        let sanctions_root = BytesN::from_array(&env, &[3u8; 32]);
        let tk_commitment = BytesN::from_array(&env, &[4u8; 32]);
        let pub_inputs = CompliancePublicInputs { sanctions_root: sanctions_root.clone(), tk_commitment: tk_commitment.clone() };
        let (vk, proof, _bad) = build_proofs(&env, [sanctions_root.into(), tk_commitment.into()]);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::NonMembership.into(), &vk);
        let mut budget = env.cost_estimate().budget();
        budget.reset_limits(2_000_000_000, 100_000_000);
        client.set_sanctions_root(&pub_inputs.sanctions_root);
        client.publish_compliance_proof(&owner, &proof, &pub_inputs);
        env.cost_estimate().budget().cpu_instruction_cost()
    }

    /// Real-WASM vs native cost of `publish_compliance_proof`; fails above the 400M limit or
    /// more than 25% over native (same rule as the token's parity tests).
    #[test]
    fn cost_parity_publish_compliance_proof() {
        extern crate std;
        let (n, w) = (publish_cost(false), publish_cost(true));
        std::println!("PARITY compliance publish_compliance_proof native={n} wasm={w}");
        assert!(w <= 400_000_000, "WASM cost {w} is over the 400M limit");
        assert!(w * 100 <= n * 125, "WASM {w} is more than 25% above native {n}");
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
        client.set_sanctions_root(&pub_inputs.sanctions_root);
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

        client.set_sanctions_root(&pub_inputs.sanctions_root);
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

        client.set_sanctions_root(&pub_inputs.sanctions_root);
        let result = client.try_publish_compliance_proof(&owner, &bad_proof, &pub_inputs);
        assert!(result.is_err());
        assert!(client.get_compliance_proof(&owner).is_none());
    }

    fn empty_inputs(env: &Env) -> CompliancePublicInputs {
        CompliancePublicInputs {
            sanctions_root: BytesN::from_array(env, &[0u8; 32]),
            tk_commitment:  BytesN::from_array(env, &[0u8; 32]),
        }
    }

    #[test]
    fn pause_and_unpause_require_admin_authorization() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let verifier = Address::generate(&env);
        let contract = env.register(ComplianceContract, ());
        let client = ComplianceContractClient::new(&env, &contract);
        client.initialize(&admin, &verifier);
        assert!(client.try_pause().is_err(), "pause must require admin authorization");
        assert!(client.try_unpause().is_err(), "unpause must require admin authorization");
    }

    #[test]
    fn pause_blocks_publish_compliance_proof() {
        let (env, owner, contract, _verifier) = setup();
        let client = ComplianceContractClient::new(&env, &contract);
        client.pause();
        client.set_sanctions_root(&empty_inputs(&env).sanctions_root);
        let result = client.try_publish_compliance_proof(&owner, &Bytes::new(&env), &empty_inputs(&env));
        assert_eq!(result, Err(Ok(Error::Paused)));
        assert!(client.get_compliance_proof(&owner).is_none());
    }

    #[test]
    fn unpause_restores_publish_compliance_proof() {
        let (env, owner, contract, _verifier) = setup();
        let client = ComplianceContractClient::new(&env, &contract);
        client.pause();
        client.unpause();
        client.set_sanctions_root(&empty_inputs(&env).sanctions_root);
        let result = client.try_publish_compliance_proof(&owner, &Bytes::new(&env), &empty_inputs(&env));
        assert_ne!(result, Err(Ok(Error::Paused)), "publish must no longer be blocked after unpause");
    }

    #[test]
    fn rejects_a_root_the_admin_has_not_authorized() {
        let (env, owner, contract, verifier) = setup();
        let client = ComplianceContractClient::new(&env, &contract);

        let prover_root = BytesN::from_array(&env, &[3u8; 32]);
        let tk_commitment = BytesN::from_array(&env, &[4u8; 32]);
        let public_inputs_le: [[u8; 32]; 2] = [prover_root.clone().into(), tk_commitment.clone().into()];
        let (vk, proof, _bad) = build_proofs(&env, public_inputs_le);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::NonMembership.into(), &vk);

        client.set_sanctions_root(&BytesN::from_array(&env, &[9u8; 32]));
        let pub_inputs = CompliancePublicInputs { sanctions_root: prover_root, tk_commitment };
        assert_eq!(
            client.try_publish_compliance_proof(&owner, &proof, &pub_inputs),
            Err(Ok(Error::UnknownSanctionsRoot)),
        );
        assert!(client.get_compliance_proof(&owner).is_none());
    }

    #[test]
    fn publishing_before_any_root_is_set_is_rejected() {
        let (env, owner, contract, _verifier) = setup();
        let client = ComplianceContractClient::new(&env, &contract);
        assert_eq!(
            client.try_publish_compliance_proof(&owner, &Bytes::new(&env), &empty_inputs(&env)),
            Err(Ok(Error::UnknownSanctionsRoot)),
        );
    }

    #[test]
    fn set_sanctions_root_requires_admin_authorization() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let verifier = Address::generate(&env);
        let contract = env.register(ComplianceContract, ());
        let client = ComplianceContractClient::new(&env, &contract);
        client.initialize(&admin, &verifier);
        assert!(client.try_set_sanctions_root(&BytesN::from_array(&env, &[1u8; 32])).is_err());
    }

    #[test]
    fn pause_does_not_block_reads() {
        let (env, owner, contract, _verifier) = setup();
        let client = ComplianceContractClient::new(&env, &contract);
        client.pause();
        assert!(client.get_compliance_proof(&owner).is_none());
    }

    // Real non-membership proof produced by the SDK prover (sdk/src/prover/compliance.ts)
    // against circuits/compliance/build, with a fixed test spending key and list.
    const NM_VK_HEX: &str = "0b291fdaaa28add7553e94df40614c894ca8fb22a2b6b4ed7351d325cad7068e1242afa10511b208e98200b835350f44a0b2641bf06744f87f3960b79f6122880041e3d1d3043bbf9687e1c198b5fe1f3f597c26b7a97127b33b64938c49887e0c65ed7e66ecf358b07f11fc7eb9cb3ecb88ec0dcfb88c12938f1ef0fa330e601c122704e90921beaa1548ea5efcd702fa0689a866360fd874cec4d1f0507f7430573487cca5aaa0c8a3417a831694d86b1171e39d821f5d456f9be5a2d7457f198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa1ccc5c0d7119761a53031371e0c2aea83977372a65d425d5d1b089120064f62120cf74009fead184417b98a5723060cd56ead300a7584c13a9b7c22a4b56434b025ea2f25c54b6405eab8e327f86fcee6ab54646bf2d152956ad21e05e8d2aca024942c9869db3681c627d23c3f75fbbd92d35fbc5ee45fb2f4d6eae7390b584136dcdbb4eeb47c328783cd8c7ac92dec5d5b51de616bad35845d7845ebb261023225d302134a931c399123f18f4a5d3a59d90d15211108f9bcd2da69ad4507b222daf8eb1f9418ea273cb8dd4fd4453a0f74babbdb73eea54996dabad19174c1f1314e913b59b9717cb5de4ef59e0e36b3325e5a70da912eafe530463a626b0172d6598e7e9f8bc0c953d7d307c2f4b2b44864a612f2629fa012c03a87c02260b94a30fb149187548a6abbf0ecb5f4d1277127fc9697694ccb76ae8937dd566";
    const NM_PROOF_HEX: &str = "00a4f0be06d79bc46905e0b6ae6e4da08f33782858153638fa74d61db1d4f178218c0172b2c91407f6973580d0c9829f5240c9fbf54dfd643532e245d2e1c08b0fcd1762aa4cfc5b9866003a63e8d7cae1d7af3d335c5bce33033666ee2ce26d2131abc66207024639bf9b9799f4f75f900eb753d9083c6dcb4439973a5f088408698cf4806db4fb33e211d7302a861849a386b1f7e21997b71eb5ea45bb44681d9d36de437664f873340b0fe3d26fbaca5cad8069f1382c0f9d27de067578742db476a492d5fde47712b3c5149209e3c6ed67920bd1573bf41779d320342bd4127abd238891a0bf1ad6efcac30cf537dfeb7b608b2c1499f19f086618eb637c";
    const NM_ROOT_LE_HEX: &str = "ba7a4a5ae8c255ea1f0849513dcb552409196855a454cb29045e436526471d14";
    const NM_TK_LE_HEX: &str = "a6abd547620d29273243fa363a15b23d4e49812307b5256ff2a37fefc2c3a715";

    fn hex_to_bytes(env: &Env, s: &str) -> Bytes {
        let mut out = Bytes::new(env);
        for i in (0..s.len()).step_by(2) {
            out.push_back(u8::from_str_radix(&s[i..i + 2], 16).unwrap());
        }
        out
    }

    fn hex_to_32(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn accepts_and_stores_a_real_sdk_proof() {
        let (env, owner, contract, verifier) = setup();
        let client = ComplianceContractClient::new(&env, &contract);

        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::NonMembership.into(), &hex_to_bytes(&env, NM_VK_HEX));

        let root = BytesN::from_array(&env, &hex_to_32(NM_ROOT_LE_HEX));
        let tk = BytesN::from_array(&env, &hex_to_32(NM_TK_LE_HEX));
        client.set_sanctions_root(&root);

        let pub_inputs = CompliancePublicInputs { sanctions_root: root.clone(), tk_commitment: tk.clone() };
        client.publish_compliance_proof(&owner, &hex_to_bytes(&env, NM_PROOF_HEX), &pub_inputs);

        let stored = client.get_compliance_proof(&owner).unwrap();
        assert_eq!(stored.sanctions_root, root);
        assert_eq!(stored.tk_commitment, tk);
    }

    #[test]
    fn rejects_a_real_sdk_proof_against_a_different_authorized_root() {
        let (env, owner, contract, verifier) = setup();
        let client = ComplianceContractClient::new(&env, &contract);

        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::NonMembership.into(), &hex_to_bytes(&env, NM_VK_HEX));

        client.set_sanctions_root(&BytesN::from_array(&env, &[0xabu8; 32]));
        let pub_inputs = CompliancePublicInputs {
            sanctions_root: BytesN::from_array(&env, &hex_to_32(NM_ROOT_LE_HEX)),
            tk_commitment:  BytesN::from_array(&env, &hex_to_32(NM_TK_LE_HEX)),
        };
        assert_eq!(
            client.try_publish_compliance_proof(&owner, &hex_to_bytes(&env, NM_PROOF_HEX), &pub_inputs),
            Err(Ok(Error::UnknownSanctionsRoot)),
        );
        assert!(client.get_compliance_proof(&owner).is_none());
    }
}
