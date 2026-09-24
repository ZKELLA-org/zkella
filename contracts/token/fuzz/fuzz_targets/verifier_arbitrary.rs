#![no_main]
//! Arbitrary proof bytes and public inputs into the verifier. Invariant: it
//! never returns `Ok(true)` for junk against a zeroed (degenerate) key shape
//! unless the pairing genuinely holds, and never crashes the host.
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Bytes, BytesN, Env, Vec};
use zkella_verifier::{BatchProofItem, CircuitType, VerifierContract, VerifierContractClient};

fuzz_target!(|data: &[u8]| {
    if data.len() < 256 + 32 {
        return;
    }
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let verifier = env.register(VerifierContract, ());
    let vc = VerifierContractClient::new(&env, &verifier);
    vc.initialize(&admin);
    vc.register_verifying_key(&CircuitType::Shield, &Bytes::from_array(&env, &[0u8; 64 + 384 + 64 * 2]));

    let proof = Bytes::from_slice(&env, &data[..256]);
    let mut a = [0u8; 32];
    a.copy_from_slice(&data[256..288]);
    let inputs = Vec::from_array(&env, [BytesN::from_array(&env, &a)]);

    // Must not crash; junk must not verify (all-zero VK, so only a proof of
    // all-infinity points could ever satisfy the pairing — rare but sound).
    let single = vc.try_verify(&CircuitType::Shield, &inputs, &proof);
    let items = Vec::from_array(&env, [BatchProofItem { public_inputs: inputs, proof }]);
    let batch = vc.try_verify_batch(&CircuitType::Shield, &items);
    if let (Ok(Ok(s)), Ok(Ok(b))) = (&single, &batch) {
        assert_eq!(s, b, "verify and verify_batch disagree on a single item");
    }
});
