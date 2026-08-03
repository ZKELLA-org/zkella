//! Test-only Groth16 fixture generator.
//!
//! ct20's own shield() tests need a proof that genuinely verifies against
//! the exact public inputs those tests compute at run time (in particular
//! `pub_asset_id`, derived from a dynamically-generated test `Address` that
//! differs on every run — so no fixed hex constant can match it). Rather
//! than fake the check, this builds a real (if synthetic, not tied to
//! shield.circom's actual constraints) Groth16-shaped VK/proof pair using
//! the well-audited `ark-bn254` crate: pick beta = gamma = delta = H (the
//! standard G2 generator), choose alpha/IC freely, and set
//! `A = alpha + vk_x + C` via real curve arithmetic so that
//! `e(A,H) = e(alpha,H)*e(vk_x,H)*e(C,H)` holds by construction — the same
//! technique validated in `zkella-verifier`'s own test suite against a
//! hand-checked arkworks self-check, and separately against a real
//! shield.circom proof there. This file exercises the contract's plumbing
//! (shield() calling the verifier correctly, handling true/false), not
//! circuit-specific soundness — that's proven once, at the circuit level,
//! in zkella-verifier's `verify_accepts_real_shield_circuit_proof`.

use ark_bn254::{Fq, Fq2, Fr, G1Affine, G1Projective, G2Affine};
use ark_ec::{AffineRepr, CurveGroup};
use ark_ff::{BigInteger, PrimeField};
use ark_std::UniformRand;
use soroban_sdk::{Bytes, Env};

fn fq_be(f: &Fq) -> [u8; 32] {
    let mut out = [0u8; 32];
    let be = f.into_bigint().to_bytes_be();
    out[32 - be.len()..].copy_from_slice(&be);
    out
}

fn g1_bytes(p: &G1Affine) -> [u8; 64] {
    let mut out = [0u8; 64];
    out[0..32].copy_from_slice(&fq_be(&p.x));
    out[32..64].copy_from_slice(&fq_be(&p.y));
    out
}

fn fq2_be(f: &Fq2) -> [u8; 64] {
    let mut out = [0u8; 64];
    out[0..32].copy_from_slice(&fq_be(&f.c1));
    out[32..64].copy_from_slice(&fq_be(&f.c0));
    out
}

fn g2_bytes(p: &G2Affine) -> [u8; 128] {
    let mut out = [0u8; 128];
    out[0..64].copy_from_slice(&fq2_be(&p.x));
    out[64..128].copy_from_slice(&fq2_be(&p.y));
    out
}

/// `little_endian_bytes` -> arkworks `Fr`, reducing mod r (matching the
/// protocol-wide convention that raw 32-byte fields are canonicalized before
/// use, e.g. `ct20::poseidon::Fr::from_bytes`).
fn fr_from_le_bytes(le: &[u8; 32]) -> Fr {
    let mut be = *le;
    be.reverse();
    Fr::from_be_bytes_mod_order(&be)
}

/// Builds a self-consistent (VK, proof) pair for an arbitrary number of
/// public inputs (little-endian, matching every `CircuitType`'s public-input
/// convention — 4 for Shield, 11 for Transfer, 5 for Unshield). Returns
/// (vk_bytes, proof_bytes) ready to register with a `VerifierContract` and
/// pass to `shield()`/`transfer()`/`unshield()`.
pub fn build_valid_groth16_proof(env: &Env, public_inputs_le: &[[u8; 32]]) -> (Bytes, Bytes) {
    let mut rng = ark_std::test_rng();

    let g1 = G1Affine::generator();
    let h = G2Affine::generator(); // beta = gamma = delta = H

    let n = public_inputs_le.len();
    let alpha_s = Fr::rand(&mut rng);
    let ic_s: ark_std::vec::Vec<Fr> = (0..n + 1).map(|_| Fr::rand(&mut rng)).collect();
    let c_s = Fr::rand(&mut rng);

    let alpha: G1Projective = g1 * alpha_s;
    let ic: ark_std::vec::Vec<G1Projective> = ic_s.iter().map(|s| g1 * *s).collect();

    let mut vk_x: G1Projective = ic[0];
    for i in 0..n {
        let xi = fr_from_le_bytes(&public_inputs_le[i]);
        vk_x += ic[i + 1] * xi;
    }

    let c: G1Projective = g1 * c_s;
    let a: G1Projective = alpha + vk_x + c;

    let mut vk_bytes_arr = ark_std::vec::Vec::new();
    vk_bytes_arr.extend_from_slice(&g1_bytes(&alpha.into_affine()));
    vk_bytes_arr.extend_from_slice(&g2_bytes(&h)); // beta
    vk_bytes_arr.extend_from_slice(&g2_bytes(&h)); // gamma
    vk_bytes_arr.extend_from_slice(&g2_bytes(&h)); // delta
    for point in ic.iter() {
        vk_bytes_arr.extend_from_slice(&g1_bytes(&point.into_affine()));
    }

    let mut proof_bytes_arr = ark_std::vec::Vec::new();
    proof_bytes_arr.extend_from_slice(&g1_bytes(&a.into_affine()));
    proof_bytes_arr.extend_from_slice(&g2_bytes(&h)); // B = H
    proof_bytes_arr.extend_from_slice(&g1_bytes(&c.into_affine()));

    (
        Bytes::from_slice(env, &vk_bytes_arr),
        Bytes::from_slice(env, &proof_bytes_arr),
    )
}

/// Takes a proof previously returned by `build_valid_shield_proof` and
/// corrupts its `C` component (by adding the G1 generator to it) so the
/// result does NOT satisfy the pairing equation for the same VK/public
/// inputs — for negative tests.
///
/// Deliberately *not* built by drawing fresh randomness and reconstructing a
/// whole new (alpha, IC, A) tuple the way `build_valid_shield_proof` does:
/// `ark_std::test_rng()` is a fixed-seed RNG (reproducible by design), so two
/// independent calls to it — one in each function — draw the *same*
/// sequence, which silently produced an "invalid" proof identical to the
/// valid one the first time this was written. Corrupting the already-built
/// proof in place has no such risk.
pub fn corrupt_proof(env: &Env, proof_bytes: &Bytes) -> Bytes {
    let g1 = G1Affine::generator();

    let bytes: [u8; 256] = proof_bytes.clone().try_into().expect("proof must be 256 bytes");
    let a = g1_from_bytes(&bytes[0..64]);
    let b = &bytes[64..192];
    let c = g1_from_bytes(&bytes[192..256]);

    let corrupted_c: G1Projective = c + g1;

    let mut out = ark_std::vec::Vec::new();
    out.extend_from_slice(&g1_bytes(&a));
    out.extend_from_slice(b);
    out.extend_from_slice(&g1_bytes(&corrupted_c.into_affine()));
    Bytes::from_slice(env, &out)
}

fn g1_from_bytes(b: &[u8]) -> G1Affine {
    let x = Fq::from_be_bytes_mod_order(&b[0..32]);
    let y = Fq::from_be_bytes_mod_order(&b[32..64]);
    G1Affine::new(x, y)
}
