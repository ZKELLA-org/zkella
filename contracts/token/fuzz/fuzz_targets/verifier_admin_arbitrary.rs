#![no_main]
//! Arbitrary verifying-key management. Invariants: a second initialize is
//! rejected; the first registration for a circuit is read back exactly; a
//! second registration for the same circuit is rejected; a successful update
//! is read back exactly.
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Bytes, Env};
use zkella_verifier::{CircuitType, VerifierContract, VerifierContractClient};

fn circuit(tag: u8) -> CircuitType {
    match tag % 6 {
        0 => CircuitType::Shield,
        1 => CircuitType::Transfer,
        2 => CircuitType::Unshield,
        3 => CircuitType::NonMembership,
        4 => CircuitType::Transfer4x4,
        _ => CircuitType::SwapFairness,
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let verifier = env.register(VerifierContract, ());
    let vc = VerifierContractClient::new(&env, &verifier);
    vc.initialize(&admin);
    assert!(vc.try_initialize(&admin).is_err(), "verifier accepted a second initialize");

    let c = circuit(data[0]);
    let vk = Bytes::from_slice(&env, &data[1..]);
    let first = vc.try_register_verifying_key(&c, &vk);
    if matches!(first, Ok(Ok(()))) {
        assert!(
            matches!(vc.try_get_verifying_key(&c), Ok(Ok(ref b)) if *b == vk),
            "registered verifying key did not read back"
        );
        assert!(
            vc.try_register_verifying_key(&c, &vk).is_err(),
            "second registration for the same circuit was accepted"
        );
    }

    let next = Bytes::from_slice(&env, &data[1..data.len().min(33)]);
    if matches!(vc.try_update_verifying_key(&c, &next), Ok(Ok(()))) {
        assert!(
            matches!(vc.try_get_verifying_key(&c), Ok(Ok(ref b)) if *b == next),
            "updated verifying key did not read back"
        );
    }
    let _ = vc.try_revoke_previous_vk(&c);
});
