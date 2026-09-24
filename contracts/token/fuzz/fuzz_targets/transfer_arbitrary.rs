#![no_main]
//! Arbitrary nullifier/commitment/proof inputs into `transfer()`/`transfer4()`.
//! Invariants: no accepted call without a genuine proof; rejected calls leave
//! the tree and every nullifier untouched.
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Bytes, BytesN, Env, Vec};
use zkella_token::{ShieldedToken, ShieldedTokenClient, TransferPublicInputs};
use zkella_verifier::{CircuitType, VerifierContract, VerifierContractClient};

fuzz_target!(|data: &[u8]| {
    if data.len() < 1 + 32 * 9 {
        return;
    }
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let verifier = env.register(VerifierContract, ());
    let vc = VerifierContractClient::new(&env, &verifier);
    vc.initialize(&admin);
    vc.register_verifying_key(&CircuitType::Transfer, &Bytes::from_array(&env, &[0u8; 64 + 384 + 64 * 12]));
    vc.register_verifying_key(&CircuitType::Transfer4x4, &Bytes::from_array(&env, &[0u8; 64 + 384 + 64 * 20]));
    let token = env.register(ShieldedToken, ());
    let client = ShieldedTokenClient::new(&env, &token);
    client.initialize(&admin, &verifier);

    let b32 = |i: usize| -> BytesN<32> {
        let mut a = [0u8; 32];
        a.copy_from_slice(&data[1 + 32 * i..1 + 32 * (i + 1)]);
        BytesN::from_array(&env, &a)
    };
    let four = data[0] & 1 == 1;
    let n = if four { 4 } else { 2 };
    let mut nfs = Vec::new(&env);
    let mut outs = Vec::new(&env);
    let mut encs = Vec::new(&env);
    let mut zeros = Vec::new(&env);
    for i in 0..n {
        // Deliberately reuse bytes so duplicate nullifiers/commitments occur.
        nfs.push_back(b32(i % 3));
        outs.push_back(b32(3 + i % 4));
        encs.push_back(Bytes::from_array(&env, &[0u8; 176]));
        zeros.push_back(b32(8));
    }
    let anchor = if data[0] & 2 == 2 { client.merkle_root() } else { b32(7) };
    let pub_in = TransferPublicInputs {
        anchor,
        nullifiers: nfs.clone(),
        out_commitments: outs.clone(),
        in_value_commits: zeros.clone(),
        out_value_commits: zeros,
        fee: 0,
        asset_id: Address::generate(&env),
    };
    let proof = Bytes::from_slice(&env, &data[1 + 32 * 9..]);
    let (leaves, root) = (client.leaf_count(), client.merkle_root());

    let ok = if four {
        client.try_transfer4(&nfs, &outs, &encs, &proof, &pub_in)
    } else {
        client.try_transfer(&nfs, &outs, &encs, &proof, &pub_in)
    };
    assert!(
        ok.is_err() || ok.unwrap().is_err(),
        "transfer accepted an arbitrary proof"
    );
    assert_eq!(leaves, client.leaf_count());
    assert_eq!(root, client.merkle_root());
    for i in 0..n as u32 {
        assert!(!client.is_spent(&nfs.get(i).unwrap()));
    }
});
