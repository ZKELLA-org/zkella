#![no_std]

mod merkle;
mod poseidon;
mod types;

use soroban_sdk::{
    contract, contractimpl, symbol_short,
    token, Address, Bytes, BytesN, Env, Vec,
    xdr::ToXdr,
};

use types::{
    Error, NoteCommitmentEvent, ShieldEvent, ShieldPublicInputs,
    StorageKey, TransferPublicInputs, UnshieldPublicInputs,
};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Minimum shield amount in base units. Prevents spam note insertion at near-zero cost.
const MIN_SHIELD_AMOUNT: i128 = 1_000;

/// Expected byte length of an encrypted note bundle (ephemeral_pk || chacha-poly ciphertext).
/// 32 (ephemeral pk) + 128 (plaintext) + 16 (Poly1305 MAC) = 176.
const ENCRYPTED_NOTE_LEN: u32 = 176;

/// Instance storage TTL parameters (Stellar ledger ≈ 5 s).
const INSTANCE_TTL_THRESHOLD: u32 = 17_280 * 30;  // 30 days: only bump if below this
const INSTANCE_TTL_EXTEND_TO: u32 = 17_280 * 365; // extend to 1 year from now

// ── Note commitment ───────────────────────────────────────────────────────────

/// Compute note commitment: Poseidon2(Poseidon2(value, asset_field), Poseidon2(rho, rcm))
///
/// Field encoding:
///   value       — little-endian u128, zero-padded to 32 bytes (safe for u64 amounts)
///   asset_field — raw 32-byte contract ID extracted from the Address XDR
///                 (matches SDK's addressToField = StrKey binary decode → 32 bytes)
///   rho / rcm   — passed as-is (caller ensures they are valid field elements)
///
/// This encoding is cross-validated with the TypeScript SDK via test vectors in
/// circuits/shield/shield_test_vectors.json.
fn compute_commitment(
    env:    &Env,
    value:  i128,
    asset:  &Address,
    rho:    &BytesN<32>,
    rcm:    &BytesN<32>,
) -> [u8; 32] {
    let mut value_bytes = [0u8; 32];
    value_bytes[..16].copy_from_slice(&(value as u128).to_le_bytes());

    let asset_bytes = address_to_field_bytes(env, asset);

    let rho_bytes: [u8; 32] = rho.clone().into();
    let rcm_bytes: [u8; 32] = rcm.clone().into();

    let h1 = poseidon::poseidon2_bytes(&value_bytes, &asset_bytes);
    let h2 = poseidon::poseidon2_bytes(&rho_bytes, &rcm_bytes);
    poseidon::poseidon2_bytes(&h1, &h2)
}

/// Extract the raw 32-byte contract ID from a Soroban Address via XDR.
///
/// XDR layout of ScAddress::Contract:
///   discriminant (4 bytes, big-endian) = 0x00000001
///   contract hash (32 bytes)
///
/// This produces the same bytes as the TypeScript SDK's addressToField():
///   StrKey base32-decode → skip 1-byte version + 2-byte checksum → 32-byte payload
/// Both paths yield the same underlying 32-byte contract ID.
fn address_to_field_bytes(env: &Env, addr: &Address) -> [u8; 32] {
    let xdr = addr.to_xdr(env);
    let mut out = [0u8; 32];
    // Contract address: discriminant occupies bytes [0..4], hash at [4..36].
    for i in 0..32u32 {
        out[i as usize] = xdr.get(4 + i).unwrap_or(0) as u8;
    }
    out
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct CT20Contract;

#[contractimpl]
impl CT20Contract {

    /// Initialize the contract. Can only be called once.
    /// `verifying_key` must be a well-formed Groth16 verifying key (or empty bytes for dev).
    pub fn initialize(
        env:           Env,
        admin:         Address,
        verifying_key: Bytes,
    ) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        // A real Groth16 verifying key for BN254 is at least 256 bytes.
        // Empty bytes are permitted only in dev/test builds.
        // TODO(M2): enforce non-empty + validate structure when proof verification lands.
        let vk_len = verifying_key.len();
        assert!(vk_len == 0 || vk_len >= 256, "verifying key too short");

        env.storage().instance().set(&StorageKey::Admin, &admin);
        env.storage().instance().set(&StorageKey::VerifyingKey, &verifying_key);
        env.storage().instance().set(&StorageKey::Paused, &false);
        env.storage().instance().set(&StorageKey::NextLeafIndex, &0u32);
        // Seed TTL for the freshly created instance storage entries.
        env.storage().instance().extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
    }

    // ── Shield ────────────────────────────────────────────────────────────────

    /// Move public SEP-41 tokens into the shielded pool.
    ///
    /// Security properties enforced on-chain:
    ///   • `amount > 0` and >= MIN_SHIELD_AMOUNT
    ///   • `encrypted_note` must be exactly ENCRYPTED_NOTE_LEN bytes
    ///   • `shield_pub.pub_value` == `amount` and `pub_asset_id` == `asset`
    ///   • commitment == Poseidon2(Poseidon2(value_bytes, asset_bytes), Poseidon2(rho, rcm))
    ///   • commitment has not been seen before (prevents replay / double-spend)
    ///
    /// TODO(M2): add Groth16 proof verification via bn254_multi_pairing_check.
    ///
    /// Returns the leaf index assigned in the Merkle tree.
    pub fn shield(
        env:            Env,
        from:           Address,
        asset:          Address,
        amount:         i128,
        rho:            BytesN<32>,
        rcm:            BytesN<32>,
        commitment:     BytesN<32>,
        encrypted_note: Bytes,
        _shield_proof:  Bytes,
        shield_pub:     ShieldPublicInputs,
    ) -> Result<u32, Error> {
        // ── 1. Auth & pause check ───────────────────────────────────────────
        from.require_auth();
        Self::assert_not_paused(&env)?;

        // ── 2. Validate amount ──────────────────────────────────────────────
        if amount <= 0 {
            return Err(Error::AmountMismatch);
        }
        if amount < MIN_SHIELD_AMOUNT {
            return Err(Error::AmountMismatch);
        }

        // ── 3. Validate encrypted note length ───────────────────────────────
        if encrypted_note.len() != ENCRYPTED_NOTE_LEN {
            return Err(Error::InvalidNote);
        }

        // ── 4. Validate public inputs match tx params ───────────────────────
        if shield_pub.pub_value != amount {
            return Err(Error::AmountMismatch);
        }
        if shield_pub.pub_asset_id != asset {
            return Err(Error::AssetMismatch);
        }

        // ── 5. Verify commitment matches Poseidon2 re-computation ───────────
        let computed  = compute_commitment(&env, amount, &asset, &rho, &rcm);
        let provided: [u8; 32] = commitment.clone().into();
        if computed != provided {
            return Err(Error::CommitmentMismatch);
        }

        // ── 6. Duplicate commitment check (prevents replay / Merkle pollution) ──
        let seen_key = StorageKey::CommitmentSeen(commitment.clone());
        if env.storage().persistent().has(&seen_key) {
            return Err(Error::DuplicateCommitment);
        }

        // ── 7. TODO(M2): Groth16 proof verification ─────────────────────────
        // assert!(Self::verify_groth16(&env, &_shield_proof, &shield_pub));

        // ── 8. Effects: record commitment, update supply, insert into tree ──
        // Mark commitment as seen before external token call (reentrancy safety).
        env.storage().persistent().set(&seen_key, &true);
        env.storage().persistent().extend_ttl(&seen_key, 17_280 * 30, 17_280 * 365);

        let prev_supply: i128 = env
            .storage()
            .instance()
            .get(&StorageKey::ShieldedSupply(asset.clone()))
            .unwrap_or(0);
        let new_supply = prev_supply
            .checked_add(amount)
            .ok_or(Error::AmountMismatch)?;
        env.storage()
            .instance()
            .set(&StorageKey::ShieldedSupply(asset.clone()), &new_supply);

        let leaf_index = merkle::insert(&env, commitment.clone());

        // Bump instance storage TTL on every shield (keeps root + counter alive).
        env.storage().instance().extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);

        // ── 9. Emit events (before external call so observers see them atomically) ──
        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("shield")),
            ShieldEvent {
                leaf_index,
                asset:      asset.clone(),
                commitment: commitment.clone(),
            },
        );
        env.events().publish(
            (symbol_short!("zkella"), symbol_short!("note")),
            NoteCommitmentEvent {
                leaf_index,
                commitment,
                encrypted_note,
            },
        );

        // ── 10. Interaction: pull tokens from caller (last, after all state changes) ──
        // Doing the transfer last follows checks-effects-interactions and ensures that
        // a reentrant call on a malicious token cannot exploit partially-committed state.
        let token_client = token::Client::new(&env, &asset);
        token_client.transfer(&from, &env.current_contract_address(), &amount);

        Ok(leaf_index)
    }

    // ── Transfer — stub (M2) ──────────────────────────────────────────────────

    /// Private note-to-note transfer. Implemented in M2.
    pub fn transfer(
        _env:             Env,
        _nullifiers:      Vec<BytesN<32>>,
        _commitments:     Vec<BytesN<32>>,
        _encrypted_notes: Vec<Bytes>,
        _proof:           Bytes,
        _pub_inputs:      TransferPublicInputs,
    ) -> Result<Vec<u32>, Error> {
        Err(Error::NotImplemented)
    }

    // ── Unshield — stub (M2) ──────────────────────────────────────────────────

    /// Move tokens from the shielded pool back to a public address. Implemented in M2.
    pub fn unshield(
        _env:        Env,
        _nullifier:  BytesN<32>,
        _to:         Address,
        _proof:      Bytes,
        _pub_inputs: UnshieldPublicInputs,
    ) -> Result<(), Error> {
        Err(Error::NotImplemented)
    }

    // ── Read-only queries ─────────────────────────────────────────────────────

    /// Current Merkle root of the note commitment tree.
    pub fn merkle_root(env: Env) -> BytesN<32> {
        merkle::root(&env)
    }

    /// Returns true if a nullifier has been spent.
    pub fn is_spent(env: Env, nullifier: BytesN<32>) -> bool {
        env.storage()
            .persistent()
            .has(&StorageKey::Nullifier(nullifier))
    }

    /// Total shielded supply of a given asset.
    pub fn shielded_supply(env: Env, asset: Address) -> i128 {
        env.storage()
            .instance()
            .get(&StorageKey::ShieldedSupply(asset))
            .unwrap_or(0)
    }

    /// Merkle authentication path for a leaf, used as circuit witness.
    pub fn merkle_path(env: Env, leaf_index: u32) -> Vec<BytesN<32>> {
        merkle::get_path(&env, leaf_index)
    }

    /// Total number of shielded notes ever created.
    pub fn leaf_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&StorageKey::NextLeafIndex)
            .unwrap_or(0)
    }

    // ── Admin ─────────────────────────────────────────────────────────────────

    pub fn pause(env: Env) -> Result<(), Error> {
        Self::require_admin(&env)?;
        env.storage().instance().set(&StorageKey::Paused, &true);
        Ok(())
    }

    pub fn unpause(env: Env) -> Result<(), Error> {
        Self::require_admin(&env)?;
        env.storage().instance().set(&StorageKey::Paused, &false);
        Ok(())
    }

    /// Initiate an admin transfer. The new admin must call `accept_admin` to complete it.
    ///
    /// Two-step transfer prevents locking the contract to an uncontrolled address.
    /// For mainnet, the admin should be a multisig contract (e.g. a Soroban multisig
    /// or a DAO governance contract) rather than a single keypair.
    pub fn transfer_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        Self::require_admin(&env)?;
        env.storage()
            .instance()
            .set(&StorageKey::PendingAdmin, &new_admin);
        Ok(())
    }

    /// Complete the admin transfer initiated by the current admin.
    /// Must be called by the `new_admin` address to confirm acceptance.
    pub fn accept_admin(env: Env) -> Result<(), Error> {
        let pending: Address = env
            .storage()
            .instance()
            .get(&StorageKey::PendingAdmin)
            .ok_or(Error::NotInitialized)?;
        pending.require_auth();
        env.storage().instance().set(&StorageKey::Admin, &pending);
        env.storage().instance().remove(&StorageKey::PendingAdmin);
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn assert_not_paused(env: &Env) -> Result<(), Error> {
        let paused: bool = env
            .storage()
            .instance()
            .get(&StorageKey::Paused)
            .unwrap_or(false);
        if paused { Err(Error::Paused) } else { Ok(()) }
    }

    fn require_admin(env: &Env) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&StorageKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn setup() -> (Env, Address, Address) {
        let env   = Env::default();
        let admin = Address::generate(&env);
        let ct20  = env.register_contract(None, CT20Contract);
        (env, admin, ct20)
    }

    #[test]
    fn initialize_sets_admin_and_root() {
        let (env, admin, ct20) = setup();
        let client = CT20ContractClient::new(&env, &ct20);

        client.initialize(&admin, &Bytes::new(&env));

        let root = client.merkle_root();
        assert_ne!(root, BytesN::from_array(&env, &[0u8; 32]));
    }

    #[test]
    #[should_panic(expected = "already initialized")]
    fn initialize_cannot_be_called_twice() {
        let (env, admin, ct20) = setup();
        let client = CT20ContractClient::new(&env, &ct20);
        client.initialize(&admin, &Bytes::new(&env));
        client.initialize(&admin, &Bytes::new(&env));
    }

    #[test]
    fn merkle_root_changes_after_shield() {
        let (env, admin, ct20) = setup();
        env.mock_all_auths();
        let client = CT20ContractClient::new(&env, &ct20);
        client.initialize(&admin, &Bytes::new(&env));

        let root_before = client.merkle_root();

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin.clone());
        let token_addr  = token_id.address();

        use soroban_sdk::testutils::Ledger;
        env.ledger().with_mut(|li| { li.sequence_number = 100; });

        let user = Address::generate(&env);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&user, &1_000_000_000);

        let rho = BytesN::from_array(&env, &[1u8; 32]);
        let rcm = BytesN::from_array(&env, &[2u8; 32]);

        // Compute commitment using the same function the contract will call
        let computed = compute_commitment(&env, 100_000_000, &token_addr, &rho, &rcm);
        let commitment = BytesN::from_array(&env, &computed);

        let pub_inputs = ShieldPublicInputs {
            commitment:   commitment.clone(),
            value_commit: BytesN::from_array(&env, &[0u8; 32]),
            pub_value:    100_000_000,
            pub_asset_id: token_addr.clone(),
        };

        // Encrypted note stub: must be exactly ENCRYPTED_NOTE_LEN bytes
        let mut enc_bytes = [0u8; 176];
        enc_bytes[0] = 0xde; enc_bytes[1] = 0xad; // recognizable marker
        let encrypted_note = Bytes::from_array(&env, &enc_bytes);

        let leaf = client.shield(
            &user,
            &token_addr,
            &100_000_000i128,
            &rho,
            &rcm,
            &commitment,
            &encrypted_note,
            &Bytes::new(&env),
            &pub_inputs,
        );

        assert_eq!(leaf, 0u32);

        let root_after = client.merkle_root();
        assert_ne!(root_before, root_after);

        let supply = client.shielded_supply(&token_addr);
        assert_eq!(supply, 100_000_000i128);

        assert_eq!(client.leaf_count(), 1u32);
    }

    #[test]
    fn shield_rejects_negative_amount() {
        let (env, admin, ct20) = setup();
        env.mock_all_auths();
        let client = CT20ContractClient::new(&env, &ct20);
        client.initialize(&admin, &Bytes::new(&env));

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        let user        = Address::generate(&env);

        let rho = BytesN::from_array(&env, &[0u8; 32]);
        let rcm = BytesN::from_array(&env, &[0u8; 32]);
        let cm  = BytesN::from_array(&env, &[0u8; 32]);
        let enc = Bytes::from_array(&env, &[0u8; 176]);
        let pub_inputs = ShieldPublicInputs {
            commitment:   cm.clone(),
            value_commit: BytesN::from_array(&env, &[0u8; 32]),
            pub_value:    -1,
            pub_asset_id: token_addr.clone(),
        };

        let result = client.try_shield(&user, &token_addr, &-1i128, &rho, &rcm, &cm, &enc, &Bytes::new(&env), &pub_inputs);
        assert!(result.is_err());
    }

    #[test]
    fn shield_rejects_duplicate_commitment() {
        let (env, admin, ct20) = setup();
        env.mock_all_auths();
        let client = CT20ContractClient::new(&env, &ct20);
        client.initialize(&admin, &Bytes::new(&env));

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();

        use soroban_sdk::testutils::Ledger;
        env.ledger().with_mut(|li| { li.sequence_number = 100; });

        let user = Address::generate(&env);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset.mint(&user, &1_000_000_000);

        let rho = BytesN::from_array(&env, &[3u8; 32]);
        let rcm = BytesN::from_array(&env, &[4u8; 32]);
        let computed   = compute_commitment(&env, 1_000, &token_addr, &rho, &rcm);
        let commitment = BytesN::from_array(&env, &computed);
        let enc        = Bytes::from_array(&env, &[0u8; 176]);
        let pub_inputs = ShieldPublicInputs {
            commitment:   commitment.clone(),
            value_commit: BytesN::from_array(&env, &[0u8; 32]),
            pub_value:    1_000,
            pub_asset_id: token_addr.clone(),
        };

        // First shield succeeds
        client.shield(&user, &token_addr, &1_000i128, &rho, &rcm, &commitment, &enc, &Bytes::new(&env), &pub_inputs);

        // Second shield with same commitment must fail
        stellar_asset.mint(&user, &1_000_000_000);
        let result = client.try_shield(&user, &token_addr, &1_000i128, &rho, &rcm, &commitment, &enc, &Bytes::new(&env), &pub_inputs);
        assert!(result.is_err());
    }

    #[test]
    fn shield_rejects_wrong_encrypted_note_length() {
        let (env, admin, ct20) = setup();
        env.mock_all_auths();
        let client = CT20ContractClient::new(&env, &ct20);
        client.initialize(&admin, &Bytes::new(&env));

        let token_admin = Address::generate(&env);
        let token_id    = env.register_stellar_asset_contract_v2(token_admin);
        let token_addr  = token_id.address();
        let user        = Address::generate(&env);

        let rho = BytesN::from_array(&env, &[5u8; 32]);
        let rcm = BytesN::from_array(&env, &[6u8; 32]);
        let computed   = compute_commitment(&env, 1_000, &token_addr, &rho, &rcm);
        let commitment = BytesN::from_array(&env, &computed);
        // Wrong length: 136 instead of 176
        let bad_enc    = Bytes::from_array(&env, &[0u8; 136]);
        let pub_inputs = ShieldPublicInputs {
            commitment:   commitment.clone(),
            value_commit: BytesN::from_array(&env, &[0u8; 32]),
            pub_value:    1_000,
            pub_asset_id: token_addr.clone(),
        };

        let result = client.try_shield(&user, &token_addr, &1_000i128, &rho, &rcm, &commitment, &bad_enc, &Bytes::new(&env), &pub_inputs);
        assert!(result.is_err());
    }

    #[test]
    fn transfer_and_unshield_return_not_implemented() {
        let (env, admin, ct20) = setup();
        let client = CT20ContractClient::new(&env, &ct20);
        client.initialize(&admin, &Bytes::new(&env));

        let transfer_result = client.try_transfer(
            &Vec::new(&env),
            &Vec::new(&env),
            &Vec::new(&env),
            &Bytes::new(&env),
            &types::TransferPublicInputs {
                anchor:            BytesN::from_array(&env, &[0u8; 32]),
                nullifiers:        Vec::new(&env),
                out_commitments:   Vec::new(&env),
                in_value_commits:  Vec::new(&env),
                out_value_commits: Vec::new(&env),
                fee:               0,
                asset_id:          Address::generate(&env),
            },
        );
        assert!(transfer_result.is_err());

        let unshield_result = client.try_unshield(
            &BytesN::from_array(&env, &[0u8; 32]),
            &Address::generate(&env),
            &Bytes::new(&env),
            &types::UnshieldPublicInputs {
                anchor:         BytesN::from_array(&env, &[0u8; 32]),
                nullifier:      BytesN::from_array(&env, &[0u8; 32]),
                pub_value:      0,
                pub_asset_id:   Address::generate(&env),
                recipient_hash: BytesN::from_array(&env, &[0u8; 32]),
            },
        );
        assert!(unshield_result.is_err());
    }
}
