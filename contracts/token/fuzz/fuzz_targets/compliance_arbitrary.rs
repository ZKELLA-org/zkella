#![no_main]
//! Arbitrary compliance proofs against the compliance contract, verified by a
//! real verifier. Invariants: a second initialize is rejected, a bogus
//! non-membership proof is never stored, and the owner's record stays empty.
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Bytes, BytesN, Env};
use zkella_compliance::{CompliancePublicInputs, ComplianceContract, ComplianceContractClient};
use zkella_verifier::{CircuitType, VerifierContract, VerifierContractClient};

fuzz_target!(|data: &[u8]| {
    if data.len() < 64 + 1 {
        return;
    }
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let verifier = env.register(VerifierContract, ());
    let vc = VerifierContractClient::new(&env, &verifier);
    vc.initialize(&admin);
    vc.register_verifying_key(&CircuitType::NonMembership, &Bytes::from_array(&env, &[0u8; 64 + 384 + 64 * 3]));

    let cc = env.register(ComplianceContract, ());
    let cclient = ComplianceContractClient::new(&env, &cc);
    cclient.initialize(&admin, &verifier);
    assert!(
        cclient.try_initialize(&admin, &verifier).is_err(),
        "compliance accepted a second initialize"
    );

    let b32 = |i: usize| -> BytesN<32> {
        let mut a = [0u8; 32];
        a.copy_from_slice(&data[1 + 32 * i..1 + 32 * (i + 1)]);
        BytesN::from_array(&env, &a)
    };
    let owner = Address::generate(&env);
    let proof = Bytes::from_slice(&env, &data[65..]);
    let inputs = CompliancePublicInputs {
        sanctions_root: b32(0),
        tk_commitment: b32(1),
    };

    let res = cclient.try_publish_compliance_proof(&owner, &proof, &inputs);
    assert!(
        res.is_err() || res.unwrap().is_err(),
        "compliance accepted a bogus non-membership proof"
    );
    assert!(
        cclient.get_compliance_proof(&owner).is_none(),
        "a rejected proof left a compliance record behind"
    );
});
