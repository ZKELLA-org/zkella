//! Shield / shield_batch flow tests: exact error branches, atomicity, events,
//! auth, Merkle correctness and boundaries.
use super::*;
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{IntoVal, Val};

type ShieldRes = Result<Result<u32, soroban_sdk::ConversionError>, Result<Error, soroban_sdk::InvokeError>>;
type BatchRes = Result<Result<Vec<u32>, soroban_sdk::ConversionError>, Result<Error, soroban_sdk::InvokeError>>;

/// (leaf_count, merkle_root, shielded_supply, user balance, contract balance)
type Snap = (u32, BytesN<32>, i128, i128, i128);

struct Fx {
    env: Env,
    admin: Address,
    token: Address,
    asset: Address,
    user: Address,
}

impl Fx {
    /// Initialised token + verifier (Shield VK registered), approved asset,
    /// and a funded user.
    fn new() -> Self {
        let (env, admin, token, verifier) = setup();
        let client = ShieldedTokenClient::new(&env, &token);
        client.initialize(&admin, &verifier);
        let asset = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
        client.set_asset_approved(&asset, &true);
        let user = Address::generate(&env);
        soroban_sdk::token::StellarAssetClient::new(&env, &asset).mint(&user, &1_000_000_000);
        // The synthetic Groth16 VK does not depend on the public inputs (fixed
        // RNG seed), so registering once serves every proof built below.
        let (vk, _) = test_groth16::build_valid_shield_proof(&env, [[0u8; 32]; 4]);
        zkella_verifier::VerifierContractClient::new(&env, &verifier)
            .register_verifying_key(&CircuitType::Shield.into(), &vk);
        Fx { env, admin, token, asset, user }
    }

    fn client(&self) -> ShieldedTokenClient<'_> {
        ShieldedTokenClient::new(&self.env, &self.token)
    }

    fn balance(&self, who: &Address) -> i128 {
        soroban_sdk::token::TokenClient::new(&self.env, &self.asset).balance(who)
    }

    fn snap(&self) -> Snap {
        let c = self.client();
        (
            c.leaf_count(),
            c.merkle_root(),
            c.shielded_supply(&self.asset),
            self.balance(&self.user),
            self.balance(&self.token),
        )
    }

    /// A fully valid item (correct commitment, valid proof) for `amount`.
    fn item(&self, seed: u8, amount: i128) -> ShieldBatchItem {
        self.item_for(seed, amount, &self.asset)
    }

    fn item_for(&self, seed: u8, amount: i128, asset: &Address) -> ShieldBatchItem {
        let env = &self.env;
        let rho = BytesN::from_array(env, &canon(seed));
        let rcm = BytesN::from_array(env, &canon(seed.wrapping_add(100)));
        let owner_pk = test_pk(env);
        let mut hasher = poseidon::Poseidon2Hasher::new(env);
        let computed = compute_commitment(env, amount, asset, &rho, &rcm, &owner_pk, &mut hasher);
        let commitment = BytesN::from_array(env, &computed);
        let value_commit = BytesN::from_array(env, &[0u8; 32]);
        let mut value_bytes = [0u8; 32];
        value_bytes[..16].copy_from_slice(&(amount as u128).to_le_bytes());
        let (_vk, proof) = test_groth16::build_valid_shield_proof(
            env,
            [
                commitment.clone().into(),
                value_commit.clone().into(),
                value_bytes,
                address_to_field_bytes(env, asset),
            ],
        );
        ShieldBatchItem {
            amount,
            rho,
            rcm,
            owner_pk,
            commitment: commitment.clone(),
            encrypted_note: Bytes::from_array(env, &[seed; 176]),
            shield_proof: proof,
            shield_pub: ShieldPublicInputs {
                commitment,
                value_commit,
                pub_value: amount,
                pub_asset_id: asset.clone(),
            },
        }
    }

    fn try_shield_as(&self, from: &Address, it: &ShieldBatchItem) -> ShieldRes {
        self.client().try_shield(
            from,
            &self.asset,
            &it.amount,
            &it.rho,
            &it.rcm,
            &it.owner_pk,
            &it.commitment,
            &it.encrypted_note,
            &it.shield_proof,
            &it.shield_pub,
        )
    }

    fn try_shield(&self, it: &ShieldBatchItem) -> ShieldRes {
        self.try_shield_as(&self.user, it)
    }

    fn try_batch(&self, items: &[ShieldBatchItem]) -> BatchRes {
        let mut v = Vec::new(&self.env);
        for i in items {
            v.push_back(i.clone());
        }
        self.client().try_shield_batch(&self.user, &self.asset, &v)
    }

    /// Asserts `shield(it)` fails with exactly `err` and changes no state.
    fn assert_shield_err(&self, it: &ShieldBatchItem, err: Error) {
        let before = self.snap();
        assert_eq!(self.try_shield(it), Err(Ok(err)));
        assert_eq!(self.snap(), before, "state changed by failed shield");
    }

    /// Asserts a batch [valid item0, bad] fails with exactly `err`, state
    /// unchanged (so item0's effects were rolled back too).
    fn assert_batch_err(&self, items: &[ShieldBatchItem], err: Error) {
        let before = self.snap();
        assert_eq!(self.try_batch(items), Err(Ok(err)));
        assert_eq!(self.snap(), before, "state changed by failed shield_batch");
    }

    fn set_next_leaf(&self, n: u32) {
        self.env.as_contract(&self.token, || {
            self.env.storage().instance().set(&StorageKey::NextLeafIndex, &n);
        });
    }
}

fn shield_topics(env: &Env, name: soroban_sdk::Symbol) -> soroban_sdk::Vec<Val> {
    (symbol_short!("zkella"), name).into_val(env)
}

fn expected_events(fx: &Fx, items: &[ShieldBatchItem], first_leaf: u32) -> soroban_sdk::Vec<(Address, soroban_sdk::Vec<Val>, Val)> {
    let env = &fx.env;
    let mut out = soroban_sdk::Vec::new(env);
    for (n, it) in items.iter().enumerate() {
        let leaf_index = first_leaf + n as u32;
        out.push_back((
            fx.token.clone(),
            shield_topics(env, symbol_short!("shield")),
            ShieldEvent { leaf_index, asset: fx.asset.clone(), commitment: it.commitment.clone() }.into_val(env),
        ));
        out.push_back((
            fx.token.clone(),
            shield_topics(env, symbol_short!("note")),
            NoteCommitmentEvent {
                leaf_index,
                commitment: it.commitment.clone(),
                encrypted_note: it.encrypted_note.clone(),
            }
            .into_val(env),
        ));
    }
    out
}

// ── 1. events ─────────────────────────────────────────────────────────────────

#[test]
fn shield_emits_shield_and_note_events() {
    let fx = Fx::new();
    let it = fx.item(1, 5_000);
    assert_eq!(fx.try_shield(&it), Ok(Ok(0)));
    let ev = fx.env.events().all().filter_by_contract(&fx.token);
    assert_eq!(ev, expected_events(&fx, &[it.clone()], 0));

    // Second shield gets leaf 1 and its own event pair.
    let it2 = fx.item(2, 7_000);
    assert_eq!(fx.try_shield(&it2), Ok(Ok(1)));
    let ev = fx.env.events().all().filter_by_contract(&fx.token);
    assert_eq!(ev, expected_events(&fx, &[it2], 1));
}

#[test]
fn shield_batch_emits_one_event_pair_per_item_in_order() {
    let fx = Fx::new();
    let items = [fx.item(1, 5_000), fx.item(2, 6_000), fx.item(3, 7_000)];
    let res = fx.try_batch(&items);
    let leaves = res.unwrap().unwrap();
    assert_eq!(leaves.len(), 3);
    assert_eq!(leaves.get(0), Some(0));
    assert_eq!(leaves.get(2), Some(2));
    let ev = fx.env.events().all().filter_by_contract(&fx.token);
    assert_eq!(ev, expected_events(&fx, &items, 0));
}

// ── 2. atomicity ─────────────────────────────────────────────────────────────

#[test]
fn shield_batch_mid_batch_failure_is_atomic() {
    let fx = Fx::new();
    let empty_root = fx.client().merkle_root();
    let good = fx.item(1, 5_000);
    let mut bad = fx.item(2, 6_000);
    bad.shield_proof = test_groth16::corrupt_proof(&fx.env, &bad.shield_proof);

    fx.assert_batch_err(&[good.clone(), bad], Error::InvalidProof);
    assert_eq!(fx.client().leaf_count(), 0);
    assert_eq!(fx.client().merkle_root(), empty_root);
    assert_eq!(fx.client().shielded_supply(&fx.asset), 0);
    assert_eq!(fx.balance(&fx.user), 1_000_000_000);

    // item0's CommitmentSeen write was reverted: it shields normally now.
    assert_eq!(fx.try_shield(&good), Ok(Ok(0)));
    assert_eq!(fx.client().leaf_count(), 1);
}

// ── 3. shield_batch rejection branches ───────────────────────────────────────

#[test]
fn shield_batch_rejects_duplicate_commitment_within_batch() {
    let fx = Fx::new();
    let a = fx.item(1, 5_000);
    fx.assert_batch_err(&[a.clone(), a], Error::DuplicateCommitment);
}

#[test]
fn shield_batch_rejects_invalid_proof() {
    let fx = Fx::new();
    let mut bad = fx.item(2, 6_000);
    bad.shield_proof = test_groth16::corrupt_proof(&fx.env, &bad.shield_proof);
    fx.assert_batch_err(&[fx.item(1, 5_000), bad], Error::InvalidProof);
}

#[test]
fn shield_batch_rejects_wrong_note_length() {
    let fx = Fx::new();
    let mut bad = fx.item(2, 6_000);
    bad.encrypted_note = Bytes::from_array(&fx.env, &[0u8; 175]);
    fx.assert_batch_err(&[fx.item(1, 5_000), bad], Error::InvalidNote);
}

#[test]
fn shield_batch_rejects_zero_amount() {
    let fx = Fx::new();
    let bad = fx.item(2, 0);
    fx.assert_batch_err(&[fx.item(1, 5_000), bad], Error::AmountMismatch);
}

#[test]
fn shield_batch_rejects_below_min_amount() {
    let fx = Fx::new();
    let bad = fx.item(2, 999);
    fx.assert_batch_err(&[fx.item(1, 5_000), bad], Error::AmountMismatch);
}

#[test]
fn shield_batch_rejects_pub_value_mismatch() {
    let fx = Fx::new();
    let mut bad = fx.item(2, 6_000);
    bad.shield_pub.pub_value = 6_001;
    fx.assert_batch_err(&[fx.item(1, 5_000), bad], Error::AmountMismatch);
}

#[test]
fn shield_batch_rejects_pub_asset_mismatch() {
    let fx = Fx::new();
    let mut bad = fx.item(2, 6_000);
    bad.shield_pub.pub_asset_id = Address::generate(&fx.env);
    fx.assert_batch_err(&[fx.item(1, 5_000), bad], Error::AssetMismatch);
}

#[test]
fn shield_batch_rejects_commitment_mismatch() {
    let fx = Fx::new();
    let mut bad = fx.item(2, 6_000);
    bad.commitment = BytesN::from_array(&fx.env, &canon(200));
    fx.assert_batch_err(&[fx.item(1, 5_000), bad], Error::CommitmentMismatch);
}

#[test]
fn shield_batch_rejects_unapproved_asset() {
    let fx = Fx::new();
    let items = [fx.item(1, 5_000)];
    fx.client().set_asset_approved(&fx.asset, &false);
    fx.assert_batch_err(&items, Error::AssetNotApproved);
}

#[test]
fn shield_batch_rejects_when_paused() {
    let fx = Fx::new();
    let items = [fx.item(1, 5_000)];
    fx.client().pause();
    fx.assert_batch_err(&items, Error::Paused);
}

#[test]
fn shield_batch_rejects_when_tree_full() {
    let fx = Fx::new();
    let items = [fx.item(1, 5_000)];
    fx.set_next_leaf(merkle::MAX_LEAVES);
    fx.assert_batch_err(&items, Error::MerkleTreeFull);
}

// ── 4. single shield mismatches ──────────────────────────────────────────────

#[test]
fn shield_rejects_pub_value_mismatch() {
    let fx = Fx::new();
    let mut it = fx.item(1, 5_000);
    it.shield_pub.pub_value = 5_001;
    fx.assert_shield_err(&it, Error::AmountMismatch);
}

#[test]
fn shield_rejects_pub_asset_mismatch() {
    let fx = Fx::new();
    let mut it = fx.item(1, 5_000);
    it.shield_pub.pub_asset_id = Address::generate(&fx.env);
    fx.assert_shield_err(&it, Error::AssetMismatch);
}

#[test]
fn shield_rejects_commitment_mismatch() {
    let fx = Fx::new();
    let mut it = fx.item(1, 5_000);
    it.commitment = BytesN::from_array(&fx.env, &canon(200));
    fx.assert_shield_err(&it, Error::CommitmentMismatch);
}

// ── 5. zero / negative amounts ───────────────────────────────────────────────

#[test]
fn shield_rejects_zero_and_negative_amount() {
    let fx = Fx::new();
    for amt in [0i128, -1, -5_000, i128::MIN] {
        let it = fx.item(1, amt);
        fx.assert_shield_err(&it, Error::AmountMismatch);
    }
    // Below the minimum is also AmountMismatch.
    fx.assert_shield_err(&fx.item(1, 999), Error::AmountMismatch);
}

// ── 6. proof rejection ───────────────────────────────────────────────────────

#[test]
fn shield_rejects_invalid_proof_with_exact_error() {
    let fx = Fx::new();
    let mut it = fx.item(1, 5_000);
    it.shield_proof = test_groth16::corrupt_proof(&fx.env, &it.shield_proof);
    fx.assert_shield_err(&it, Error::InvalidProof);

    // Garbage/empty proof bytes must not succeed either (exact error is up to
    // the verifier's parsing, but state must be untouched).
    let mut it2 = fx.item(2, 5_000);
    it2.shield_proof = Bytes::new(&fx.env);
    let before = fx.snap();
    assert!(fx.try_shield(&it2).is_err());
    assert_eq!(fx.snap(), before);
}

#[test]
fn shield_rejects_valid_proof_for_different_public_value() {
    let fx = Fx::new();
    let proof_for_1000 = fx.item(1, 1_000).shield_proof;
    // Fully consistent 2_000 note, but carrying the proof made for 1_000.
    let mut it = fx.item(1, 2_000);
    it.shield_proof = proof_for_1000.clone();
    fx.assert_shield_err(&it, Error::InvalidProof);

    // Valid proof, but value_commit (a public input) altered.
    let mut it = fx.item(1, 1_000);
    it.shield_pub.value_commit = BytesN::from_array(&fx.env, &canon(33));
    fx.assert_shield_err(&it, Error::InvalidProof);

    // Proof for one commitment presented with another note's commitment.
    let other = fx.item(9, 1_000);
    let mut it = other.clone();
    it.shield_proof = proof_for_1000;
    fx.assert_shield_err(&it, Error::InvalidProof);

    // Sanity: the untampered note still shields.
    assert_eq!(fx.try_shield(&fx.item(1, 1_000)), Ok(Ok(0)));
}

// ── 7. extreme amounts ───────────────────────────────────────────────────────

#[test]
fn shield_amount_extremes_fail_cleanly_and_leave_state_unchanged() {
    let fx = Fx::new();
    // i128::MAX, 2^64 + 1, 2^127 - 1 (== i128::MAX; also MAX - 1 for variety).
    for (seed, amt) in [(1u8, i128::MAX), (2, (1i128 << 64) + 1), (3, (1i128 << 127).wrapping_sub(1)), (4, i128::MAX - 1)] {
        let it = fx.item(seed, amt);
        let before = fx.snap();
        // All checks pass; only the SEP-41 pull (user holds 1e9) can fail, and
        // it must roll back every effect.
        let res = fx.try_shield(&it);
        assert!(res.is_err(), "amount {amt} must not succeed with a 1e9 balance: {res:?}");
        assert_eq!(fx.snap(), before, "state changed for amount {amt}");
    }
    // The commitments were not burned by the failed attempts.
    soroban_sdk::token::StellarAssetClient::new(&fx.env, &fx.asset).mint(&fx.user, &((1i128 << 64) + 1));
    assert_eq!(fx.try_shield(&fx.item(2, (1i128 << 64) + 1)), Ok(Ok(0)));
    assert_eq!(fx.client().shielded_supply(&fx.asset), (1i128 << 64) + 1);
}

#[test]
fn shield_supply_overflow_is_a_defined_error() {
    let fx = Fx::new();
    assert_eq!(fx.try_shield(&fx.item(1, 5_000)), Ok(Ok(0)));
    let before = fx.snap();
    let it = fx.item(2, i128::MAX);
    assert_eq!(fx.try_shield(&it), Err(Ok(Error::AmountMismatch)));
    assert_eq!(fx.snap(), before);
}

#[test]
fn shield_batch_total_overflow_fails_cleanly_with_state_unchanged() {
    let fx = Fx::new();
    // Sum of items overflows i128.
    let items = [fx.item(1, i128::MAX), fx.item(2, 1_000)];
    fx.assert_batch_err(&items, Error::AmountMismatch);

    // Batch total overflowing the existing supply.
    assert_eq!(fx.try_shield(&fx.item(3, 5_000)), Ok(Ok(0)));
    fx.assert_batch_err(&[fx.item(4, i128::MAX)], Error::AmountMismatch);

    // A single giant item that does not overflow still fails cleanly (balance).
    let fresh = Fx::new();
    let before = fresh.snap();
    assert!(fresh.try_batch(&[fresh.item(5, i128::MAX)]).is_err());
    assert_eq!(fresh.snap(), before);
}

// ── 8. auth ──────────────────────────────────────────────────────────────────

#[test]
fn shield_requires_from_auth() {
    let fx = Fx::new();
    let it = fx.item(1, 5_000);
    let before = fx.snap();

    // No authorization entries at all: must fail.
    fx.env.set_auths(&[]);
    assert!(matches!(fx.try_shield(&it), Err(Err(_))), "shield without from-auth must fail");
    assert!(matches!(fx.try_batch(&[it.clone()]), Err(Err(_))), "shield_batch without from-auth must fail");
    assert_eq!(fx.snap(), before);

    // With mocking on, the address whose auth is required is `from`.
    fx.env.mock_all_auths();
    assert_eq!(fx.try_shield(&it), Ok(Ok(0)));
    let auths = fx.env.auths();
    assert!(auths.iter().any(|(a, _)| *a == fx.user), "user auth must be recorded");
}

#[test]
fn admin_functions_reject_non_admin() {
    let fx = Fx::new();
    let other = Address::generate(&fx.env);
    let c = fx.client();

    // Auth for a *different* address than the admin does not satisfy admin functions.
    fx.env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &other,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &fx.token,
            fn_name: "pause",
            args: ().into_val(&fx.env),
            sub_invokes: &[],
        },
    }]);
    assert!(matches!(c.try_pause(), Err(Err(_))));

    // And with no authorization at all every admin function is rejected.
    fx.env.set_auths(&[]);
    assert!(matches!(c.try_pause(), Err(Err(_))));
    assert!(matches!(c.try_unpause(), Err(Err(_))));
    assert!(matches!(c.try_set_asset_approved(&fx.asset, &false), Err(Err(_))));
    assert!(matches!(c.try_set_min_shield_amount(&5), Err(Err(_))));
    assert!(matches!(c.try_transfer_admin(&other), Err(Err(_))));

    // Nothing changed.
    assert!(c.is_asset_approved(&fx.asset));
    assert_eq!(c.min_shield_amount(), 1_000);
    fx.env.mock_all_auths();
    assert_eq!(fx.try_shield(&fx.item(1, 5_000)), Ok(Ok(0)), "not paused");
}

#[test]
fn admin_calls_require_the_admin_address_and_accept_admin_only_pending() {
    let fx = Fx::new();
    let c = fx.client();
    let new_admin = Address::generate(&fx.env);

    // Admin-gated call records the admin as the required authorizer.
    c.pause();
    assert_eq!(fx.env.auths()[0].0, fx.admin);
    c.unpause();

    // No pending transfer: accept_admin fails.
    assert!(c.try_accept_admin().is_err());

    c.transfer_admin(&new_admin);
    assert_eq!(fx.env.auths()[0].0, fx.admin);

    // accept_admin under no authorization fails (only the pending admin can sign).
    fx.env.set_auths(&[]);
    assert!(matches!(c.try_accept_admin(), Err(Err(_))));

    fx.env.mock_all_auths();
    c.accept_admin();
    assert_eq!(fx.env.auths()[0].0, new_admin, "accept_admin must require the pending admin");

    // After the completed transfer, admin functions require the NEW admin.
    c.pause();
    assert_eq!(fx.env.auths()[0].0, new_admin);
    c.unpause();
    c.set_min_shield_amount(&2_000);
    assert_eq!(fx.env.auths()[0].0, new_admin);
    assert_ne!(fx.env.auths()[0].0, fx.admin, "old admin is no longer the authorizer");

    // Under enforcement with no auths, both old-admin and new-admin flows fail.
    fx.env.set_auths(&[]);
    assert!(matches!(c.try_pause(), Err(Err(_))));
    // Pending admin was consumed.
    fx.env.mock_all_auths();
    assert!(c.try_accept_admin().is_err());
}

// ── 9. merkle independent recomputation ──────────────────────────────────────

fn ref_empty_roots() -> std::vec::Vec<[u8; 32]> {
    let mut e = std::vec![crate::poseidon::poseidon2_bytes(&[0u8; 32], &[0u8; 32])];
    for l in 0..32 {
        let last = e[l];
        e.push(crate::poseidon::poseidon2_bytes(&last, &last));
    }
    e
}

/// Returns (root, per-level node vectors).
fn ref_tree(leaves: &[[u8; 32]]) -> ([u8; 32], std::vec::Vec<std::vec::Vec<[u8; 32]>>) {
    let empty = ref_empty_roots();
    let mut levels = std::vec![leaves.to_vec()];
    for l in 0..32usize {
        let cur = levels[l].clone();
        let mut next = std::vec::Vec::new();
        let mut i = 0;
        while i < cur.len() {
            let left = cur[i];
            let right = if i + 1 < cur.len() { cur[i + 1] } else { empty[l] };
            next.push(crate::poseidon::poseidon2_bytes(&left, &right));
            i += 2;
        }
        levels.push(next);
    }
    let root = levels[32].first().copied().unwrap_or(empty[32]);
    (root, levels)
}

fn ref_path(levels: &[std::vec::Vec<[u8; 32]>], empty: &[[u8; 32]], leaf: usize) -> std::vec::Vec<[u8; 32]> {
    let mut idx = leaf;
    let mut out = std::vec::Vec::new();
    for l in 0..32 {
        let sib = idx ^ 1;
        out.push(levels[l].get(sib).copied().unwrap_or(empty[l]));
        idx /= 2;
    }
    out
}

#[test]
fn merkle_root_matches_independent_recomputation() {
    let fx = Fx::new();
    let empty = ref_empty_roots();
    let (empty_root, _) = ref_tree(&[]);
    assert_eq!(empty_root, empty[32]);
    let root0: [u8; 32] = fx.client().merkle_root().into();
    assert_eq!(root0, empty[32], "empty-tree root must be the depth-32 empty root");

    let mut leaves: std::vec::Vec<[u8; 32]> = std::vec::Vec::new();
    for i in 0..5u8 {
        let it = fx.item(10 + i, 2_000 + i as i128);
        assert_eq!(fx.try_shield(&it), Ok(Ok(i as u32)));
        leaves.push(it.commitment.clone().into());
        // Root tracks the reference after every insertion.
        let (expected, _) = ref_tree(&leaves);
        let got: [u8; 32] = fx.client().merkle_root().into();
        assert_eq!(got, expected, "root mismatch after {} leaves", i + 1);
    }

    let (root, levels) = ref_tree(&leaves);
    for i in [1usize, 2, 3] {
        let path = fx.client().merkle_path(&(i as u32));
        assert_eq!(path.len(), 32);
        let expected = ref_path(&levels, &empty, i);
        for l in 0..32 {
            let got: [u8; 32] = path.get(l as u32).unwrap().into();
            assert_eq!(got, expected[l], "path[{l}] mismatch for leaf {i}");
        }
        // Odd indices have a real (left) sibling at level 0.
        if i % 2 == 1 {
            assert_eq!(expected[0], leaves[i - 1]);
        }
        // The path folds up to the on-chain root.
        let mut sibs = [[0u8; 32]; 32];
        sibs.copy_from_slice(&expected);
        assert!(merkle::verify_path(&leaves[i], &sibs, i as u32, &root));
    }
}

// ── 10. last-slot boundary ───────────────────────────────────────────────────

#[test]
fn merkle_last_slot_boundary() {
    let fx = Fx::new();
    fx.set_next_leaf(merkle::MAX_LEAVES - 1);
    let it = fx.item(1, 5_000);
    assert_eq!(fx.try_shield(&it), Ok(Ok(merkle::MAX_LEAVES - 1)));
    assert_eq!(fx.client().leaf_count(), merkle::MAX_LEAVES);

    let it2 = fx.item(2, 5_000);
    fx.assert_shield_err(&it2, Error::MerkleTreeFull);
}

// ── 11. encrypted note length ────────────────────────────────────────────────

#[test]
fn encrypted_note_len_boundaries() {
    let fx = Fx::new();
    for len in [0usize, 175, 177] {
        let mut it = fx.item(1, 5_000);
        it.encrypted_note = Bytes::from_slice(&fx.env, &std::vec![7u8; len]);
        fx.assert_shield_err(&it, Error::InvalidNote);
        fx.assert_batch_err(&[it], Error::InvalidNote);
    }
    // 176 is accepted.
    assert_eq!(fx.try_shield(&fx.item(1, 5_000)), Ok(Ok(0)));
}

// ── 12. min amount setter ────────────────────────────────────────────────────

#[test]
fn set_min_shield_amount_rejects_nonpositive() {
    let fx = Fx::new();
    let c = fx.client();
    assert_eq!(c.min_shield_amount(), 1_000);
    assert_eq!(c.try_set_min_shield_amount(&0), Err(Ok(Error::AmountMismatch)));
    assert_eq!(c.try_set_min_shield_amount(&-5), Err(Ok(Error::AmountMismatch)));
    assert_eq!(c.min_shield_amount(), 1_000);
    assert_eq!(c.try_set_min_shield_amount(&1), Ok(Ok(())));
    assert_eq!(c.min_shield_amount(), 1);
}

// ── 13. token-transfer failure reverts effects ───────────────────────────────

#[test]
fn shield_effects_reverted_when_token_transfer_fails() {
    let fx = Fx::new();
    let poor = Address::generate(&fx.env);
    let it = fx.item(1, 5_000);
    let before = fx.snap();

    assert!(fx.try_shield_as(&poor, &it).is_err());
    assert_eq!(fx.snap(), before);
    let mut v = Vec::new(&fx.env);
    v.push_back(it.clone());
    assert!(fx.client().try_shield_batch(&poor, &fx.asset, &v).is_err());
    assert_eq!(fx.snap(), before);

    // Nothing persisted: after minting, the same commitment shields at leaf 0.
    soroban_sdk::token::StellarAssetClient::new(&fx.env, &fx.asset).mint(&poor, &5_000);
    assert_eq!(fx.try_shield_as(&poor, &it), Ok(Ok(0)));
    assert_eq!(fx.client().shielded_supply(&fx.asset), 5_000);
    assert_eq!(fx.balance(&poor), 0);
}

// ── 14. pause ────────────────────────────────────────────────────────────────

#[test]
fn pause_blocks_shield_and_shield_batch_and_unpause_restores() {
    let fx = Fx::new();
    let a = fx.item(1, 5_000);
    let b = fx.item(2, 6_000);
    fx.client().pause();
    fx.assert_shield_err(&a, Error::Paused);
    fx.assert_batch_err(&[a.clone()], Error::Paused);

    fx.client().unpause();
    assert_eq!(fx.try_shield(&a), Ok(Ok(0)));
    let leaves = fx.try_batch(&[b]).unwrap().unwrap();
    assert_eq!(leaves.get(0), Some(1));
}

// ── extra: unapproved asset on single shield ─────────────────────────────────

#[test]
fn shield_rejects_unapproved_asset_exact_error() {
    let fx = Fx::new();
    let it = fx.item(1, 5_000);
    fx.client().set_asset_approved(&fx.asset, &false);
    fx.assert_shield_err(&it, Error::AssetNotApproved);
}
