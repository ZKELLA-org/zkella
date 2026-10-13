#![no_main]
//! Arbitrary viewing-key registrations. Invariant: a successful register is
//! read back exactly by get_viewing_key_commitment for the same owner.
use libfuzzer_sys::fuzz_target;
use soroban_sdk::{testutils::Address as _, Address, BytesN, Env};
use zkella_viewing_keys::{ViewingKeyRegistry, ViewingKeyRegistryClient};

fuzz_target!(|data: &[u8]| {
    if data.len() < 33 {
        return;
    }
    let env = Env::default();
    env.mock_all_auths();
    let reg = env.register(ViewingKeyRegistry, ());
    let client = ViewingKeyRegistryClient::new(&env, &reg);

    let owner = Address::generate(&env);
    let mut a = [0u8; 32];
    a.copy_from_slice(&data[1..33]);
    let commitment = BytesN::from_array(&env, &a);
    client.register(&owner, &commitment, &(data[0] as u32 * 1000));

    assert_eq!(
        client.get_viewing_key_commitment(&owner),
        Some(commitment),
        "registered viewing-key commitment did not read back"
    );
    client.revoke(&owner);
    assert!(
        client.get_viewing_key_commitment(&owner).is_none(),
        "revoked viewing-key commitment is still readable"
    );
});
