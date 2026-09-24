#![no_main]
//! Feeds arbitrary bytes/amounts into `shield()` with a real (shape-only) VK
//! registered. Invariant: any rejected call leaves leaf count, Merkle root and
//! shielded supply untouched, and the harness itself never panics.
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, Bytes, BytesN, Env};
use zkella_token::{ShieldPublicInputs, ShieldedToken, ShieldedTokenClient};
use zkella_verifier::{CircuitType, VerifierContract, VerifierContractClient};

fuzz_target!(|data: &[u8]| {
    if data.len() < 16 + 32 * 3 {
        return;
    }
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let verifier = env.register(VerifierContract, ());
    let vc = VerifierContractClient::new(&env, &verifier);
    vc.initialize(&admin);
    vc.register_verifying_key(&CircuitType::Shield, &Bytes::from_array(&env, &[0u8; 64 + 384 + 64 * 5]));
    let token = env.register(ShieldedToken, ());
    let client = ShieldedTokenClient::new(&env, &token);
    client.initialize(&admin, &verifier);

    let sac = env.register_stellar_asset_contract_v2(Address::generate(&env));
    let asset = sac.address();
    if data[0] & 1 == 1 {
        client.set_asset_approved(&asset, &true);
    }
    let from = Address::generate(&env);
    soroban_sdk::token::StellarAssetClient::new(&env, &asset).mint(&from, &(i128::MAX / 4));

    let mut amt = [0u8; 16];
    amt.copy_from_slice(&data[..16]);
    let amount = i128::from_le_bytes(amt);
    let b32 = |i: usize| -> BytesN<32> {
        let mut a = [0u8; 32];
        a.copy_from_slice(&data[16 + 32 * i..16 + 32 * (i + 1)]);
        BytesN::from_array(&env, &a)
    };
    let rest = &data[16 + 32 * 3..];
    let commitment = b32(2);
    let pub_in = ShieldPublicInputs {
        commitment: commitment.clone(),
        value_commit: b32(0),
        pub_value: if data[0] & 2 == 2 { amount } else { amount.wrapping_add(1) },
        pub_asset_id: asset.clone(),
    };
    let split = rest.len() / 2;
    let (enc, proof) = rest.split_at(split);

    let (leaves, root, supply) =
        (client.leaf_count(), client.merkle_root(), client.shielded_supply(&asset));
    let res = client.try_shield(
        &from, &asset, &amount, &b32(0), &b32(1), &commitment,
        &Bytes::from_slice(&env, enc), &Bytes::from_slice(&env, proof), &pub_in,
    );
    if res.is_err() || res.as_ref().map(|r| r.is_err()).unwrap_or(true) {
        assert_eq!(leaves, client.leaf_count());
        assert_eq!(root, client.merkle_root());
        assert_eq!(supply, client.shielded_supply(&asset));
    } else {
        // Success without a genuine proof would be a soundness break.
        panic!("shield accepted an arbitrary proof");
    }
});
