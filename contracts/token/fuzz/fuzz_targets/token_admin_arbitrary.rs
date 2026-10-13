#![no_main]
//! Arbitrary inputs into the token's admin, pause, allowlist and relayer
//! entrypoints, plus unshield and shield_batch. Invariants: each setter reads
//! back the value it stored; a rejected unshield or shield_batch leaves the
//! tree and nullifiers untouched; a paused contract rejects proof-consuming
//! calls; a second accept_admin with no pending transfer is rejected.
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Bytes, BytesN, Env, Vec};
use zkella_token::{ShieldBatchItem, ShieldPublicInputs, ShieldedToken, ShieldedTokenClient, UnshieldPublicInputs};
use zkella_verifier::{CircuitType, VerifierContract, VerifierContractClient};

fuzz_target!(|data: &[u8]| {
    if data.len() < 1 + 32 * 8 {
        return;
    }
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let verifier = env.register(VerifierContract, ());
    let vc = VerifierContractClient::new(&env, &verifier);
    vc.initialize(&admin);
    vc.register_verifying_key(&CircuitType::Shield, &Bytes::from_array(&env, &[0u8; 64 + 384 + 64 * 5]));
    vc.register_verifying_key(&CircuitType::Unshield, &Bytes::from_array(&env, &[0u8; 64 + 384 + 64 * 8]));

    let token = env.register(ShieldedToken, ());
    let tc = ShieldedTokenClient::new(&env, &token);
    tc.initialize(&admin, &verifier);

    let b32 = |i: usize| -> BytesN<32> {
        let mut a = [0u8; 32];
        a.copy_from_slice(&data[1 + 32 * i..1 + 32 * (i + 1)]);
        BytesN::from_array(&env, &a)
    };
    let amount = data[0] as i128;
    let asset = Address::generate(&env);
    let other = Address::generate(&env);

    if let Ok(Ok(())) = tc.try_set_min_shield_amount(&amount) {
        assert_eq!(tc.min_shield_amount(), amount, "min_shield_amount did not read back");
    }
    let approved = data[0] & 1 == 1;
    if let Ok(Ok(())) = tc.try_set_asset_approved(&asset, &approved) {
        assert_eq!(tc.is_asset_approved(&asset), approved, "asset approval did not read back");
    }
    if let Ok(Ok(())) = tc.try_set_relayer(&other, &approved) {
        assert_eq!(tc.is_approved_relayer(&other), approved, "relayer approval did not read back");
    }

    let root = tc.merkle_root();
    let leaves = tc.leaf_count();
    let nullifier = b32(0);
    let unshield = tc.try_unshield(
        &nullifier,
        &other,
        &b32(1),
        &b32(2),
        &Bytes::from_slice(&env, &data[1 + 32 * 3..1 + 32 * 4]),
        &Bytes::from_slice(&env, &data[1 + 32 * 4..]),
        &UnshieldPublicInputs {
            anchor: b32(5),
            nullifier: nullifier.clone(),
            pub_value: amount,
            pub_asset_id: asset.clone(),
            recipient_hash: b32(6),
            change_commitment: b32(7),
            change_value_commit: b32(7),
        },
    );
    assert!(unshield.is_err() || unshield.unwrap().is_err(), "unshield accepted a bogus proof");
    assert_eq!(root, tc.merkle_root(), "rejected unshield changed the root");
    assert_eq!(leaves, tc.leaf_count(), "rejected unshield changed the leaf count");
    assert!(!tc.is_spent(&nullifier), "rejected unshield spent a nullifier");

    let batch_root = tc.merkle_root();
    let batch_leaves = tc.leaf_count();
    let items: Vec<ShieldBatchItem> = Vec::from_array(
        &env,
        [ShieldBatchItem {
            amount,
            rho: b32(0),
            rcm: b32(1),
            owner_pk: b32(2),
            commitment: b32(3),
            encrypted_note: Bytes::new(&env),
            shield_proof: Bytes::new(&env),
            shield_pub: ShieldPublicInputs {
                commitment: b32(3),
                value_commit: b32(4),
                pub_value: amount,
                pub_asset_id: asset.clone(),
            },
        }],
    );
    let batch = tc.try_shield_batch(&other, &asset, &items);
    if batch.is_err() || batch.as_ref().unwrap().is_err() {
        assert_eq!(batch_root, tc.merkle_root(), "rejected shield_batch changed the root");
        assert_eq!(batch_leaves, tc.leaf_count(), "rejected shield_batch changed the leaf count");
    }

    tc.try_pause().ok();
    assert!(
        tc.try_unshield(
            &b32(0),
            &other,
            &b32(1),
            &b32(2),
            &Bytes::new(&env),
            &Bytes::new(&env),
            &UnshieldPublicInputs {
                anchor: b32(5),
                nullifier: b32(0),
                pub_value: amount,
                pub_asset_id: asset.clone(),
                recipient_hash: b32(6),
                change_commitment: b32(7),
                change_value_commit: b32(7),
            },
        )
        .is_err(),
        "unshield succeeded while paused"
    );
    tc.try_unpause().ok();

    tc.try_transfer_admin(&other).ok();
    tc.try_accept_admin().ok();
    assert!(tc.try_accept_admin().is_err(), "accept_admin succeeded with no pending transfer");
});
