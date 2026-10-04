#![no_main]
//! Arbitrary VK-update requests against the governance contract. Invariants:
//! a second initialize is rejected, a queued update cannot execute before its
//! timelock elapses, and accept_admin with no pending transfer is rejected.
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Bytes, Env};
use zkella_governance::{ZKELLAGovernance, ZKELLAGovernanceClient};
use zkella_verifier_interface::CircuitType;

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
    let verifier = Address::generate(&env);
    let gov = env.register(ZKELLAGovernance, ());
    let gc = ZKELLAGovernanceClient::new(&env, &gov);
    let guardian = Address::generate(&env);
    gc.initialize(&admin, &verifier, &guardian);

    assert!(
        gc.try_initialize(&admin, &verifier, &guardian).is_err(),
        "governance accepted a second initialize"
    );

    let c = circuit(data[0]);
    let vk = Bytes::from_slice(&env, &data[1..]);
    let _ = gc.try_queue_vk_update(&c, &vk);
    assert!(
        gc.try_execute_vk_update(&c).is_err(),
        "VK update executed before its timelock elapsed"
    );
    assert!(
        gc.try_accept_admin().is_err(),
        "accept_admin succeeded with no pending admin transfer"
    );
    assert!(gc.timelock_ledgers() > 0, "timelock is zero");
});
