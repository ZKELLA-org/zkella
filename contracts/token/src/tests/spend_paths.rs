//! Error-branch tests for transfer, transfer4 and unshield (the last uncovered lines in the token crate).
use super::*;
extern crate std;

use soroban_sdk::testutils::Address as _;

fn zero(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

fn notes(env: &Env, n: u32) -> Vec<Bytes> {
    let mut v = Vec::new(env);
    for _ in 0..n {
        v.push_back(Bytes::from_array(env, &[0u8; 176]));
    }
    v
}

/// Builds transfer public inputs for `n` slots with distinct canonical nullifiers/commitments
/// derived from `seed` (nullifiers seed+i, commitments seed+10+i).
fn transfer_pi(env: &Env, client: &ShieldedTokenClient, asset: &Address, n: u32, seed: u8) -> TransferPublicInputs {
    let mut nfs = Vec::new(env);
    let mut cms = Vec::new(env);
    let mut z = Vec::new(env);
    for i in 0..n {
        nfs.push_back(BytesN::from_array(env, &canon(seed + i as u8)));
        cms.push_back(BytesN::from_array(env, &canon(seed + 10 + i as u8)));
        z.push_back(zero(env));
    }
    TransferPublicInputs {
        anchor: client.merkle_root(),
        nullifiers: nfs,
        out_commitments: cms,
        in_value_commits: z.clone(),
        out_value_commits: z,
        fee: 0,
        asset_id: asset.clone(),
    }
}

/// Registers the VK, or replaces it if one is already registered for the circuit.
fn set_vk(env: &Env, verifier: &Address, circuit: CircuitType, vk: &Bytes) {
    let c = zkella_verifier::VerifierContractClient::new(env, verifier);
    if c.try_register_verifying_key(&circuit.into(), vk).is_err() {
        c.update_verifying_key(&circuit.into(), vk);
    }
}

/// Builds a valid proof for `pi` (fee must be >= 0) and registers its VK for `circuit`.
fn transfer_proof(env: &Env, verifier: &Address, circuit: CircuitType, pi: &TransferPublicInputs) -> Bytes {
    let mut le: std::vec::Vec<[u8; 32]> = std::vec::Vec::new();
    le.push(pi.anchor.clone().into());
    for v in [&pi.nullifiers, &pi.out_commitments, &pi.in_value_commits, &pi.out_value_commits] {
        for x in v.iter() {
            le.push(x.into());
        }
    }
    let mut fee = [0u8; 32];
    fee[..16].copy_from_slice(&(pi.fee.max(0) as u128).to_le_bytes());
    le.push(fee);
    le.push(address_to_field_bytes(env, &pi.asset_id));
    let (vk, proof) = test_groth16::build_valid_groth16_proof(env, &le);
    set_vk(env, verifier, circuit, &vk);
    proof
}

struct Env0 {
    env: Env,
    token: Address,
    verifier: Address,
    asset: Address,
}

fn fixture() -> Env0 {
    let (env, admin, token, verifier) = setup();
    let client = ShieldedTokenClient::new(&env, &token);
    client.initialize(&admin, &verifier);
    let asset = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    client.set_asset_approved(&asset, &true);
    Env0 { env, token, verifier, asset }
}

impl Env0 {
    fn client(&self) -> ShieldedTokenClient<'_> {
        ShieldedTokenClient::new(&self.env, &self.token)
    }

    /// Real shield of `amount`; returns the note commitment.
    fn shield(&self, seed: u8, amount: i128) -> BytesN<32> {
        let env = &self.env;
        let shielder = Address::generate(env);
        soroban_sdk::token::StellarAssetClient::new(env, &self.asset).mint(&shielder, &amount);
        let mut h = poseidon::Poseidon2Hasher::new(env);
        let rho = BytesN::from_array(env, &canon(seed));
        let rcm = BytesN::from_array(env, &canon(seed + 1));
        let cm = BytesN::from_array(env, &compute_commitment(env, amount, &self.asset, &rho, &rcm, &test_pk(env), &mut h));
        let vc = zero(env);
        let pi = ShieldPublicInputs { commitment: cm.clone(), value_commit: vc.clone(), pub_value: amount, pub_asset_id: self.asset.clone() };
        let proof = prove_and_register_shield(env, &self.verifier, &cm, &vc, amount, &self.asset);
        self.client().shield(&shielder, &self.asset, &amount, &rho, &rcm, &test_pk(env), &cm, &Bytes::from_array(env, &[0u8; 176]), &proof, &pi);
        cm
    }

    fn snap(&self, nfs: &Vec<BytesN<32>>) -> (u32, BytesN<32>, i128, std::vec::Vec<bool>) {
        let c = self.client();
        (c.leaf_count(), c.merkle_root(), c.shielded_supply(&self.asset), nfs.iter().map(|n| c.is_spent(&n)).collect())
    }

    /// Asserts a transfer/transfer4 call fails with `err` and leaves all state untouched.
    fn assert_transfer_err(&self, four: bool, nfs: &Vec<BytesN<32>>, cms: &Vec<BytesN<32>>, enc: &Vec<Bytes>, proof: &Bytes, pi: &TransferPublicInputs, err: Error) {
        let before = self.snap(&pi.nullifiers);
        let c = self.client();
        let res = if four { c.try_transfer4(nfs, cms, enc, proof, pi) } else { c.try_transfer(nfs, cms, enc, proof, pi) };
        assert_eq!(res, Err(Ok(err)));
        assert_eq!(self.snap(&pi.nullifiers), before);
        assert!(before.3.iter().all(|s| !s));
    }

    fn unshield_pi(&self, to: &Address, tag: &BytesN<32>, nullifier: &BytesN<32>, pub_value: i128) -> UnshieldPublicInputs {
        let mut h = poseidon::Poseidon2Hasher::new(&self.env);
        let tag_b: [u8; 32] = tag.clone().into();
        let rh = h.hash(&address_to_field_bytes(&self.env, to), &tag_b);
        UnshieldPublicInputs {
            anchor: self.client().merkle_root(),
            nullifier: nullifier.clone(),
            pub_value,
            pub_asset_id: self.asset.clone(),
            recipient_hash: BytesN::from_array(&self.env, &rh),
        }
    }

    /// Valid proof for `pi` (pub_value clamped to >= 0 for encoding); registers the Unshield VK.
    fn unshield_proof(&self, pi: &UnshieldPublicInputs) -> Bytes {
        let mut vb = [0u8; 32];
        vb[..16].copy_from_slice(&(pi.pub_value.max(0) as u128).to_le_bytes());
        let le: [[u8; 32]; 5] = [
            pi.anchor.clone().into(),
            pi.nullifier.clone().into(),
            vb,
            address_to_field_bytes(&self.env, &pi.pub_asset_id),
            pi.recipient_hash.clone().into(),
        ];
        let (vk, proof) = test_groth16::build_valid_groth16_proof(&self.env, &le);
        set_vk(&self.env, &self.verifier, CircuitType::Unshield, &vk);
        proof
    }

    fn usnap(&self, nf: &BytesN<32>, to: &Address) -> (u32, BytesN<32>, i128, bool, i128, i128) {
        let c = self.client();
        let sa = soroban_sdk::token::StellarAssetClient::new(&self.env, &self.asset);
        (c.leaf_count(), c.merkle_root(), c.shielded_supply(&self.asset), c.is_spent(nf), sa.balance(to), sa.balance(&self.token))
    }

    fn assert_unshield_err(&self, nf: &BytesN<32>, to: &Address, tag: &BytesN<32>, proof: &Bytes, pi: &UnshieldPublicInputs, err: Error) {
        let before = self.usnap(nf, to);
        let res = self.client().try_unshield(nf, to, tag, proof, pi);
        assert_eq!(res, Err(Ok(err)));
        assert_eq!(self.usnap(nf, to), before);
    }
}

// ───────────────────────── transfer (2-in-2-out) ─────────────────────────

#[test]
fn transfer_rejects_negative_fee() {
    let fx = fixture();
    let mut pi = transfer_pi(&fx.env, &fx.client(), &fx.asset, 2, 50);
    let proof = transfer_proof(&fx.env, &fx.verifier, CircuitType::Transfer, &pi);
    pi.fee = -1;
    fx.assert_transfer_err(false, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 2), &proof, &pi, Error::AmountMismatch);
}

#[test]
fn transfer_rejects_wrong_arity() {
    let fx = fixture();
    let pi = transfer_pi(&fx.env, &fx.client(), &fx.asset, 2, 60);
    let proof = transfer_proof(&fx.env, &fx.verifier, CircuitType::Transfer, &pi);
    let one_nf = Vec::from_array(&fx.env, [pi.nullifiers.get(0).unwrap()]);
    let one_cm = Vec::from_array(&fx.env, [pi.out_commitments.get(0).unwrap()]);
    let three_nf = Vec::from_array(&fx.env, [pi.nullifiers.get(0).unwrap(), pi.nullifiers.get(1).unwrap(), zero(&fx.env)]);
    let e = Error::InvalidInputCount;
    fx.assert_transfer_err(false, &one_nf, &pi.out_commitments, &notes(&fx.env, 2), &proof, &pi, e);
    fx.assert_transfer_err(false, &three_nf, &pi.out_commitments, &notes(&fx.env, 2), &proof, &pi, e);
    fx.assert_transfer_err(false, &pi.nullifiers, &one_cm, &notes(&fx.env, 2), &proof, &pi, e);
    fx.assert_transfer_err(false, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 1), &proof, &pi, e);
    fx.assert_transfer_err(false, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 3), &proof, &pi, e);

    // Mismatched pub_inputs vec lengths, each field in turn.
    let mut p = pi.clone();
    p.nullifiers = one_nf.clone();
    fx.assert_transfer_err(false, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 2), &proof, &p, e);
    let mut p = pi.clone();
    p.out_commitments = one_cm.clone();
    fx.assert_transfer_err(false, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 2), &proof, &p, e);
    let mut p = pi.clone();
    p.in_value_commits = Vec::from_array(&fx.env, [zero(&fx.env)]);
    fx.assert_transfer_err(false, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 2), &proof, &p, e);
    let mut p = pi.clone();
    p.out_value_commits = Vec::from_array(&fx.env, [zero(&fx.env), zero(&fx.env), zero(&fx.env)]);
    fx.assert_transfer_err(false, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 2), &proof, &p, e);
}

#[test]
fn transfer_rejects_nullifier_arg_differing_from_pub_inputs() {
    let fx = fixture();
    let pi = transfer_pi(&fx.env, &fx.client(), &fx.asset, 2, 70);
    let proof = transfer_proof(&fx.env, &fx.verifier, CircuitType::Transfer, &pi);
    for slot in 0..2u32 {
        let mut nfs = pi.nullifiers.clone();
        nfs.set(slot, BytesN::from_array(&fx.env, &canon(200)));
        fx.assert_transfer_err(false, &nfs, &pi.out_commitments, &notes(&fx.env, 2), &proof, &pi, Error::CommitmentMismatch);
    }
}

#[test]
fn transfer_rejects_commitments_arg_differing_from_pub_inputs() {
    let fx = fixture();
    let pi = transfer_pi(&fx.env, &fx.client(), &fx.asset, 2, 80);
    let proof = transfer_proof(&fx.env, &fx.verifier, CircuitType::Transfer, &pi);
    for slot in 0..2u32 {
        let mut cms = pi.out_commitments.clone();
        cms.set(slot, BytesN::from_array(&fx.env, &canon(201)));
        fx.assert_transfer_err(false, &pi.nullifiers, &cms, &notes(&fx.env, 2), &proof, &pi, Error::CommitmentMismatch);
    }
}

#[test]
fn transfer_rejects_output_commitment_already_in_tree() {
    let fx = fixture();
    let existing = fx.shield(90, 1_000);
    let mut pi = transfer_pi(&fx.env, &fx.client(), &fx.asset, 2, 100);
    pi.out_commitments.set(1, existing);
    let proof = transfer_proof(&fx.env, &fx.verifier, CircuitType::Transfer, &pi);
    assert_eq!(fx.client().leaf_count(), 1);
    fx.assert_transfer_err(false, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 2), &proof, &pi, Error::DuplicateCommitment);
}

#[test]
fn transfer_rejects_invalid_proof() {
    let fx = fixture();
    let pi = transfer_pi(&fx.env, &fx.client(), &fx.asset, 2, 110);
    let proof = transfer_proof(&fx.env, &fx.verifier, CircuitType::Transfer, &pi);
    let bad = test_groth16::corrupt_proof(&fx.env, &proof);
    fx.assert_transfer_err(false, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 2), &bad, &pi, Error::InvalidProof);

    // A proof valid for different public inputs must also fail.
    let mut other = pi.clone();
    other.fee = 5;
    let other_proof = transfer_proof(&fx.env, &fx.verifier, CircuitType::Transfer, &other);
    fx.assert_transfer_err(false, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 2), &other_proof, &pi, Error::InvalidProof);
}

#[test]
fn transfer_rejects_when_tree_is_full() {
    let fx = fixture();
    let pi = transfer_pi(&fx.env, &fx.client(), &fx.asset, 2, 120);
    let proof = transfer_proof(&fx.env, &fx.verifier, CircuitType::Transfer, &pi);
    fx.env.as_contract(&fx.token, || {
        fx.env.storage().instance().set(&StorageKey::NextLeafIndex, &merkle::MAX_LEAVES);
    });
    fx.assert_transfer_err(false, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 2), &proof, &pi, Error::MerkleTreeFull);
    for cm in pi.out_commitments.iter() {
        assert!(!fx.env.as_contract(&fx.token, || fx.env.storage().persistent().has(&StorageKey::CommitmentSeen(cm.clone()))));
    }
}

// ───────────────────────── transfer4 (4-in-4-out) ─────────────────────────

#[test]
fn transfer4_rejects_negative_fee() {
    let fx = fixture();
    let mut pi = transfer_pi(&fx.env, &fx.client(), &fx.asset, 4, 130);
    let proof = transfer_proof(&fx.env, &fx.verifier, CircuitType::Transfer4x4, &pi);
    pi.fee = -1;
    fx.assert_transfer_err(true, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 4), &proof, &pi, Error::AmountMismatch);
}

#[test]
fn transfer4_rejects_invalid_proof() {
    let fx = fixture();
    let pi = transfer_pi(&fx.env, &fx.client(), &fx.asset, 4, 140);
    let proof = transfer_proof(&fx.env, &fx.verifier, CircuitType::Transfer4x4, &pi);
    let bad = test_groth16::corrupt_proof(&fx.env, &proof);
    fx.assert_transfer_err(true, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 4), &bad, &pi, Error::InvalidProof);
}

#[test]
fn transfer4_rejects_wrong_arity() {
    let fx = fixture();
    let pi = transfer_pi(&fx.env, &fx.client(), &fx.asset, 4, 150);
    let proof = transfer_proof(&fx.env, &fx.verifier, CircuitType::Transfer4x4, &pi);
    let three_nf = Vec::from_array(&fx.env, [pi.nullifiers.get(0).unwrap(), pi.nullifiers.get(1).unwrap(), pi.nullifiers.get(2).unwrap()]);
    fx.assert_transfer_err(true, &three_nf, &pi.out_commitments, &notes(&fx.env, 4), &proof, &pi, Error::InvalidInputCount);
    fx.assert_transfer_err(true, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 3), &proof, &pi, Error::InvalidInputCount);
    let mut p = pi.clone();
    p.nullifiers = three_nf;
    fx.assert_transfer_err(true, &pi.nullifiers, &pi.out_commitments, &notes(&fx.env, 4), &proof, &p, Error::InvalidInputCount);
    // 2-slot inputs into transfer4.
    let p2 = transfer_pi(&fx.env, &fx.client(), &fx.asset, 2, 160);
    fx.assert_transfer_err(true, &p2.nullifiers, &p2.out_commitments, &notes(&fx.env, 2), &proof, &p2, Error::InvalidInputCount);
}

// ───────────────────────────── unshield ─────────────────────────────

#[test]
fn unshield_rejects_pub_inputs_nullifier_mismatch() {
    let fx = fixture();
    fx.shield(170, 1_000_000);
    let to = Address::generate(&fx.env);
    let tag = zero(&fx.env);
    let nf = BytesN::from_array(&fx.env, &canon(180));
    let pi = fx.unshield_pi(&to, &tag, &nf, 1_000);
    let proof = fx.unshield_proof(&pi);
    let other = BytesN::from_array(&fx.env, &canon(181));
    fx.assert_unshield_err(&other, &to, &tag, &proof, &pi, Error::CommitmentMismatch);
}

#[test]
fn unshield_rejects_wrong_recipient_and_binding_tag_exactly() {
    let fx = fixture();
    fx.shield(170, 1_000_000);
    let to = Address::generate(&fx.env);
    let tag = zero(&fx.env);
    let nf = BytesN::from_array(&fx.env, &canon(182));
    let pi = fx.unshield_pi(&to, &tag, &nf, 1_000);
    let proof = fx.unshield_proof(&pi);
    let wrong_to = Address::generate(&fx.env);
    fx.assert_unshield_err(&nf, &wrong_to, &tag, &proof, &pi, Error::RecipientMismatch);
    let wrong_tag = BytesN::from_array(&fx.env, &canon(7));
    fx.assert_unshield_err(&nf, &to, &wrong_tag, &proof, &pi, Error::RecipientMismatch);
    // Corrupted recipient_hash in pub_inputs.
    let mut bad = pi.clone();
    bad.recipient_hash = BytesN::from_array(&fx.env, &canon(8));
    fx.assert_unshield_err(&nf, &to, &tag, &proof, &bad, Error::RecipientMismatch);
}

#[test]
fn unshield_rejects_unknown_anchor() {
    let fx = fixture();
    fx.shield(170, 1_000_000);
    let to = Address::generate(&fx.env);
    let tag = zero(&fx.env);
    let nf = BytesN::from_array(&fx.env, &canon(183));
    let mut pi = fx.unshield_pi(&to, &tag, &nf, 1_000);
    pi.anchor = BytesN::from_array(&fx.env, &canon(99));
    let proof = fx.unshield_proof(&pi);
    fx.assert_unshield_err(&nf, &to, &tag, &proof, &pi, Error::InvalidAnchor);
}

#[test]
fn unshield_rejects_already_spent_nullifier() {
    let fx = fixture();
    fx.shield(170, 1_000_000);
    let to = Address::generate(&fx.env);
    let tag = zero(&fx.env);
    let nf = BytesN::from_array(&fx.env, &canon(184));
    let pi = fx.unshield_pi(&to, &tag, &nf, 1_000);
    let proof = fx.unshield_proof(&pi);
    fx.client().unshield(&nf, &to, &tag, &proof, &pi);
    assert!(fx.client().is_spent(&nf));
    fx.assert_unshield_err(&nf, &to, &tag, &proof, &pi, Error::NullifierSpent);
    assert_eq!(fx.client().shielded_supply(&fx.asset), 1_000_000 - 1_000);
}

#[test]
fn unshield_rejects_non_positive_value() {
    let fx = fixture();
    fx.shield(170, 1_000_000);
    let to = Address::generate(&fx.env);
    let tag = zero(&fx.env);
    let nf = BytesN::from_array(&fx.env, &canon(185));
    for v in [0i128, -1, i128::MIN] {
        let pi = fx.unshield_pi(&to, &tag, &nf, v);
        let proof = fx.unshield_proof(&pi);
        fx.assert_unshield_err(&nf, &to, &tag, &proof, &pi, Error::AmountMismatch);
    }
}

#[test]
fn unshield_rejects_invalid_proof() {
    let fx = fixture();
    fx.shield(170, 1_000_000);
    let to = Address::generate(&fx.env);
    let tag = zero(&fx.env);
    let nf = BytesN::from_array(&fx.env, &canon(186));
    let pi = fx.unshield_pi(&to, &tag, &nf, 1_000);
    let proof = fx.unshield_proof(&pi);
    let bad = test_groth16::corrupt_proof(&fx.env, &proof);
    fx.assert_unshield_err(&nf, &to, &tag, &bad, &pi, Error::InvalidProof);
    // Proof valid for a different amount must fail against these public inputs.
    let mut other = pi.clone();
    other.pub_value = 2_000;
    let other_proof = fx.unshield_proof(&other);
    fx.assert_unshield_err(&nf, &to, &tag, &other_proof, &pi, Error::InvalidProof);
}
