#![no_main]
//! Arbitrary inputs into every swap entrypoint, against a real token and
//! verifier. Invariants: a second initialize is always rejected; a commit
//! carrying a bogus ownership proof is rejected; execute, reveal, cancel and
//! reclaim against an unknown swap id are rejected. No call may crash the host.
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Bytes, BytesN, Env};
use zkella_swap::{ShieldedSwap, ShieldedSwapClient, SwapFairnessPublicInputs};
use zkella_token::{ShieldedToken, ShieldedTokenClient};
use zkella_verifier::{CircuitType, VerifierContract, VerifierContractClient};

const HEAD: usize = 1 + 32 * 10;

fuzz_target!(|data: &[u8]| {
    if data.len() < HEAD {
        return;
    }
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let verifier = env.register(VerifierContract, ());
    let vc = VerifierContractClient::new(&env, &verifier);
    vc.initialize(&admin);
    vc.register_verifying_key(&CircuitType::Unshield, &Bytes::from_array(&env, &[0u8; 64 + 384 + 64 * 8]));
    vc.register_verifying_key(&CircuitType::SwapFairness, &Bytes::from_array(&env, &[0u8; 64 + 384 + 64 * 6]));

    let token = env.register(ShieldedToken, ());
    ShieldedTokenClient::new(&env, &token).initialize(&admin, &verifier);

    let swap = env.register(ShieldedSwap, ());
    let sc = ShieldedSwapClient::new(&env, &swap);
    sc.initialize(&admin, &verifier, &token);

    assert!(
        sc.try_initialize(&admin, &verifier, &token).is_err(),
        "swap accepted a second initialize"
    );

    let b32 = |i: usize| -> BytesN<32> {
        let mut a = [0u8; 32];
        a.copy_from_slice(&data[1 + 32 * i..1 + 32 * (i + 1)]);
        BytesN::from_array(&env, &a)
    };
    let rest = &data[HEAD..];
    let split = rest.len() / 2;
    let (enc, proof) = rest.split_at(split);
    let asset = Address::generate(&env);
    let refund = Address::generate(&env);
    let expiry = env.ledger().sequence() + 100;

    let commit = sc.try_commit_swap(
        &b32(0),
        &b32(1),
        &asset,
        &asset,
        &1_000i128,
        &b32(2),
        &refund,
        &b32(3),
        &b32(6),
        &(data[0] as i128),
        &b32(4),
        &b32(5),
        &Bytes::from_slice(&env, enc),
        &Bytes::from_slice(&env, proof),
        &expiry,
    );
    assert!(
        commit.is_err() || commit.unwrap().is_err(),
        "commit_swap accepted a bogus ownership proof"
    );

    let unknown = b32(6);
    let relayer = Address::generate(&env);
    assert!(
        sc.try_execute_swap(&unknown, &1_000i128, &relayer).is_err(),
        "execute_swap accepted an unknown swap id"
    );

    let fairness_pub = SwapFairnessPublicInputs {
        intent_commitment: b32(7),
        asset_in: asset.clone(),
        asset_out: asset.clone(),
        amount_out: 1_000,
        min_amount_out: data[0] as i128,
    };
    let reveal = sc.try_reveal_and_claim(
        &unknown,
        &b32(8),
        &b32(9),
        &b32(0),
        &b32(1),
        &b32(2),
        &Bytes::from_slice(&env, enc),
        &Bytes::from_slice(&env, proof),
        &fairness_pub,
        &Bytes::from_slice(&env, enc),
    );
    assert!(
        reveal.is_err() || reveal.unwrap().is_err(),
        "reveal_and_claim accepted an unknown swap id"
    );

    assert!(sc.try_cancel_swap(&unknown).is_err(), "cancel_swap accepted an unknown swap id");
    assert!(
        sc.try_reclaim_expired_swap(&unknown).is_err(),
        "reclaim_expired_swap accepted an unknown swap id"
    );

    let _ = sc.try_set_relayer(&relayer, &(data[0] & 1 == 1));
});
