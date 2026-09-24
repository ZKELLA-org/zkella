//! Real-WASM vs native-Rust instruction-cost parity for every proof-verifying
//! or Merkle-mutating entrypoint (Tranche 1, Deliverable 6).
//!
//! A native-Rust cost estimate can look comfortable while the compiled WASM the
//! network actually runs is materially more expensive (transfer4 read 89% native
//! against 97% on the real WASM before the verifier was optimised). These tests
//! run each entrypoint twice, once against the native contract and once against
//! the compiled `wasm32v1-none` artefacts, and FAIL the build when the WASM cost
//! exceeds the 400M mainnet limit or drifts more than `MAX_WASM_OVER_NATIVE`
//! above the native figure. They run in CI with `cargo test --workspace` after
//! the WASM artefacts are built.
use super::*;
extern crate std;
use std::vec::Vec as StdVec;

const BUDGET: u64 = 400_000_000;
/// WASM may cost at most this much more than native before the build fails.
/// Measured gaps sit around 9-12%; 25% leaves room for compiler noise while
/// still catching a real divergence.
const MAX_WASM_OVER_NATIVE_PERCENT: u64 = 25;
/// The verifier as it was before the batched-MSM optimisation (per-input
/// `g1_mul`/`g1_add` loop), kept only as a measurement baseline.
const VERIFIER_PRE_MSM_WASM: &[u8] = include_bytes!("../../tests_data/verifier_pre_msm.wasm");

struct Ctx {
    env: Env,
    client_addr: Address,
    verifier: Address,
}

fn ctx(wasm: bool, verifier_wasm: Option<&[u8]>) -> Ctx {
    let env = Env::default();
    env.cost_estimate().budget().reset_limits(2_000_000_000, 100_000_000);
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let token = if wasm { env.register(TOKEN_WASM, ()) } else { env.register(ShieldedToken, ()) };
    let verifier = match (wasm, verifier_wasm) {
        (_, Some(bytes)) => env.register(bytes, ()),
        (true, None) => env.register(VERIFIER_WASM, ()),
        (false, None) => env.register(zkella_verifier::VerifierContract, ()),
    };
    zkella_verifier::VerifierContractClient::new(&env, &verifier).initialize(&admin);
    ShieldedTokenClient::new(&env, &token).initialize(&admin, &verifier);
    Ctx { env, client_addr: token, verifier }
}

fn client(c: &Ctx) -> ShieldedTokenClient<'_> {
    ShieldedTokenClient::new(&c.env, &c.client_addr)
}

fn cost(c: &Ctx) -> u64 {
    c.env.cost_estimate().budget().cpu_instruction_cost()
}

fn approved_asset(c: &Ctx) -> Address {
    let asset = c.env.register_stellar_asset_contract_v2(Address::generate(&c.env)).address();
    client(c).set_asset_approved(&asset, &true);
    asset
}

/// One real shield, returning (leaf_index, commitment) — used to fund pool state.
fn shield_one(c: &Ctx, asset: &Address, user: &Address, amount: i128, seed: u8) -> BytesN<32> {
    let env = &c.env;
    let rho = BytesN::from_array(env, &canon(seed));
    let rcm = BytesN::from_array(env, &canon(seed + 1));
    let mut hasher = poseidon::Poseidon2Hasher::new(env);
    let commitment = BytesN::from_array(env, &compute_commitment(env, amount, asset, &rho, &rcm, &test_pk(env), &mut hasher));
    let value_commit = BytesN::from_array(env, &[0u8; 32]);
    let proof = prove_and_register_shield(env, &c.verifier, &commitment, &value_commit, amount, asset);
    let pub_in = ShieldPublicInputs { commitment: commitment.clone(), value_commit, pub_value: amount, pub_asset_id: asset.clone() };
    client(c).shield(user, asset, &amount, &rho, &rcm, &test_pk(env), &commitment, &Bytes::from_array(env, &[0u8; 176]), &proof, &pub_in);
    commitment
}

fn shield_cost(wasm: bool) -> u64 {
    let c = ctx(wasm, None);
    let asset = approved_asset(&c);
    let user = Address::generate(&c.env);
    soroban_sdk::token::StellarAssetClient::new(&c.env, &asset).mint(&user, &1_000_000_000);
    let env = &c.env;
    let rho = BytesN::from_array(env, &canon(9));
    let rcm = BytesN::from_array(env, &canon(10));
    let mut hasher = poseidon::Poseidon2Hasher::new(env);
    let commitment = BytesN::from_array(env, &compute_commitment(env, 1_000, &asset, &rho, &rcm, &test_pk(env), &mut hasher));
    let value_commit = BytesN::from_array(env, &[0u8; 32]);
    let proof = prove_and_register_shield(env, &c.verifier, &commitment, &value_commit, 1_000, &asset);
    let pub_in = ShieldPublicInputs { commitment: commitment.clone(), value_commit, pub_value: 1_000, pub_asset_id: asset.clone() };
    env.cost_estimate().budget().reset_tracker();
    client(&c).shield(&user, &asset, &1_000i128, &rho, &rcm, &test_pk(env), &commitment, &Bytes::from_array(env, &[0u8; 176]), &proof, &pub_in);
    cost(&c)
}

fn shield_batch_cost(wasm: bool) -> u64 {
    let c = ctx(wasm, None);
    let asset = approved_asset(&c);
    let user = Address::generate(&c.env);
    soroban_sdk::token::StellarAssetClient::new(&c.env, &asset).mint(&user, &1_000_000_000);
    let env = &c.env;
    let mut hasher = poseidon::Poseidon2Hasher::new(env);
    let mut items = Vec::new(env);
    for i in 0..3u8 {
        let amount: i128 = 10_000 * (i as i128 + 1);
        let rho = BytesN::from_array(env, &canon(60 + 2 * i));
        let rcm = BytesN::from_array(env, &canon(61 + 2 * i));
        let commitment = BytesN::from_array(env, &compute_commitment(env, amount, &asset, &rho, &rcm, &test_pk(env), &mut hasher));
        let value_commit = BytesN::from_array(env, &[0u8; 32]);
        let mut value_bytes = [0u8; 32];
        value_bytes[..16].copy_from_slice(&(amount as u128).to_le_bytes());
        let (vk, proof) = test_groth16::build_valid_shield_proof(
            env,
            [commitment.clone().into(), value_commit.clone().into(), value_bytes, address_to_field_bytes(env, &asset)],
        );
        if i == 0 {
            zkella_verifier::VerifierContractClient::new(env, &c.verifier).register_verifying_key(&CircuitType::Shield.into(), &vk);
        }
        items.push_back(ShieldBatchItem {
            amount, rho, rcm, owner_pk: test_pk(env), commitment: commitment.clone(),
            encrypted_note: Bytes::from_array(env, &[0u8; 176]), shield_proof: proof,
            shield_pub: ShieldPublicInputs { commitment, value_commit, pub_value: amount, pub_asset_id: asset.clone() },
        });
    }
    env.cost_estimate().budget().reset_tracker();
    client(&c).shield_batch(&user, &asset, &items);
    cost(&c)
}

/// n = 2 (transfer) or 4 (transfer4).
fn transfer_cost(wasm: bool, n: u32, verifier_wasm: Option<&[u8]>) -> u64 {
    let c = ctx(wasm, verifier_wasm);
    let env = &c.env;
    let asset = Address::generate(env);
    client(&c).set_asset_approved(&asset, &true);
    let anchor = client(&c).merkle_root();
    let base = 181u8;
    let mut nullifiers = Vec::new(env);
    let mut outs = Vec::new(env);
    let mut zeros = Vec::new(env);
    let mut encs = Vec::new(env);
    for i in 0..n as u8 {
        nullifiers.push_back(BytesN::from_array(env, &canon(base + i)));
        outs.push_back(BytesN::from_array(env, &canon(base + 10 + i)));
        zeros.push_back(BytesN::from_array(env, &[0u8; 32]));
        encs.push_back(Bytes::from_array(env, &[0u8; 176]));
    }
    let pub_inputs = TransferPublicInputs {
        anchor: anchor.clone(), nullifiers: nullifiers.clone(), out_commitments: outs.clone(),
        in_value_commits: zeros.clone(), out_value_commits: zeros, fee: 0, asset_id: asset.clone(),
    };
    let mut public_inputs_le: StdVec<[u8; 32]> = StdVec::new();
    public_inputs_le.push(anchor.clone().into());
    for i in 0..n { public_inputs_le.push(nullifiers.get(i).unwrap().into()); }
    for i in 0..n { public_inputs_le.push(outs.get(i).unwrap().into()); }
    for _ in 0..(2 * n) { public_inputs_le.push([0u8; 32]); }
    public_inputs_le.push([0u8; 32]); // fee
    public_inputs_le.push(address_to_field_bytes(env, &asset));
    let (vk, proof) = test_groth16::build_valid_groth16_proof(env, &public_inputs_le);
    let circuit = if n == 2 { CircuitType::Transfer } else { CircuitType::Transfer4x4 };
    zkella_verifier::VerifierContractClient::new(env, &c.verifier).register_verifying_key(&circuit.into(), &vk);
    env.cost_estimate().budget().reset_tracker();
    if n == 2 {
        client(&c).transfer(&nullifiers, &outs, &encs, &proof, &pub_inputs);
    } else if verifier_wasm.is_some() {
        // The baseline verifier is expected to exceed the network limit, which
        // aborts the call; the instructions consumed up to that point are the figure.
        let cl = client(&c);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cl.transfer4(&nullifiers, &outs, &encs, &proof, &pub_inputs)
        }));
    } else {
        client(&c).transfer4(&nullifiers, &outs, &encs, &proof, &pub_inputs);
    }
    cost(&c)
}

fn unshield_cost(wasm: bool) -> u64 {
    let c = ctx(wasm, None);
    let env = &c.env;
    let asset = approved_asset(&c);
    let shielder = Address::generate(env);
    let recipient = Address::generate(env);
    soroban_sdk::token::StellarAssetClient::new(env, &asset).mint(&shielder, &1_000_000_000);
    shield_one(&c, &asset, &shielder, 1_000_000, 40);
    let anchor = client(&c).merkle_root();
    let nullifier = BytesN::from_array(env, &canon(42));
    let mut hasher = poseidon::Poseidon2Hasher::new(env);
    let recipient_hash_bytes = hasher.hash(&address_to_field_bytes(env, &recipient), &[0u8; 32]);
    let pub_value: i128 = 500_000;
    let pub_inputs = UnshieldPublicInputs {
        anchor: anchor.clone(), nullifier: nullifier.clone(), pub_value, pub_asset_id: asset.clone(),
        recipient_hash: BytesN::from_array(env, &recipient_hash_bytes),
    };
    let mut value_bytes = [0u8; 32];
    value_bytes[..16].copy_from_slice(&(pub_value as u128).to_le_bytes());
    let public_inputs_le: [[u8; 32]; 5] = [
        anchor.into(), nullifier.clone().into(), value_bytes, address_to_field_bytes(env, &asset), recipient_hash_bytes,
    ];
    let (vk, proof) = test_groth16::build_valid_groth16_proof(env, &public_inputs_le);
    zkella_verifier::VerifierContractClient::new(env, &c.verifier).register_verifying_key(&CircuitType::Unshield.into(), &vk);
    env.cost_estimate().budget().reset_tracker();
    client(&c).unshield(&nullifier, &recipient, &BytesN::from_array(env, &[0u8; 32]), &proof, &pub_inputs);
    cost(&c)
}

fn check(name: &str, native: u64, wasm: u64) {
    std::println!("COST_PARITY {name}: native={native} wasm={wasm} ({}% of budget)", wasm * 100 / BUDGET);
    assert!(wasm < BUDGET, "{name}: real WASM used {wasm} instructions, over the {BUDGET} mainnet limit");
    assert!(
        wasm * 100 <= native * (100 + MAX_WASM_OVER_NATIVE_PERCENT),
        "{name}: real WASM ({wasm}) is more than {MAX_WASM_OVER_NATIVE_PERCENT}% above the native estimate ({native}); \
         the native figure is not a trustworthy proxy any more"
    );
}

#[test]
fn cost_parity_shield() { check("shield", shield_cost(false), shield_cost(true)); }

#[test]
fn cost_parity_shield_batch() { check("shield_batch(3)", shield_batch_cost(false), shield_batch_cost(true)); }

#[test]
fn cost_parity_transfer() { check("transfer 2x2", transfer_cost(false, 2, None), transfer_cost(true, 2, None)); }

#[test]
fn cost_parity_transfer4() { check("transfer4", transfer_cost(false, 4, None), transfer_cost(true, 4, None)); }

#[test]
fn cost_parity_unshield() { check("unshield", unshield_cost(false), unshield_cost(true)); }

/// The optimisation this tranche funds: aggregating public inputs with the
/// native batched multi-scalar-multiplication host function instead of one
/// `g1_mul` + `g1_add` per input. Measured end to end on the real WASM for the
/// heaviest entrypoint (19 public inputs), against the verifier as it was
/// before the change.
#[test]
fn msm_aggregation_is_cheaper_than_the_per_input_loop_on_transfer4() {
    let before = transfer_cost(true, 4, Some(VERIFIER_PRE_MSM_WASM));
    let after = transfer_cost(true, 4, None);
    std::println!("MSM_VS_LOOP transfer4 real WASM: per-input loop verifier={before} batched-MSM verifier={after} saved={}", before.saturating_sub(after));
    assert!(after < before, "batched MSM ({after}) must cost less than the per-input loop ({before})");
    assert!(before >= BUDGET, "the per-input loop verifier was over the network limit for transfer4 (used {before}); if not, the baseline comment is wrong");
    assert!(after < BUDGET, "transfer4 with the MSM verifier must fit the {BUDGET} budget, used {after}");
}
