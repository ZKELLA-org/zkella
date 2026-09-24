#![no_std]

//! Groth16/BN254 verifying-key registry, shared across ZKELLA's circuits.
//!
//! Stores one verifying key per [`CircuitType`] and exposes [`VerifierContract::verify`],
//! a generic Groth16 pairing check against Soroban's native BN254 host functions
//! (`bn254_g1_add`, `bn254_g1_mul`, `bn254_g1_msm`, `bn254_multi_pairing_check`,
//! protocol 25+). The public-input linear combination (`vk_x`) uses `bn254_g1_msm`,
//! a single batched multi-scalar-multiplication call, rather than one `g1_mul` +
//! `g1_add` pair per input — the dominant real-WASM cost in this function for any
//! circuit with more than a couple of public inputs.
//! Keeping this in its own contract — rather than embedding the VK in `token`'s
//! instance storage — lets the verifying key be rotated (via `governance`'s
//! timelock) without redeploying the token contract, and keeps VK-management
//! privileges scoped separately from token-admin powers.
//!
//! ## Wire format
//!
//! `verifying_key: Bytes` = `alpha_g1(64) || beta_g2(128) || gamma_g2(128) || delta_g2(128) || IC[0](64) || IC[1](64) || ...`
//! where `IC` has one entry per public input plus one (`IC[0]` is the constant term).
//! `proof: Bytes` = `A(64) || B(128) || C(64)`.
//! All points use the host's native encoding: G1 = 64 bytes `be(X)||be(Y)`,
//! G2 = 128 bytes `be(X)||be(Y)` with each coordinate an Fp2 element `be(c1)||be(c0)`.
//! Public inputs are field elements as 32-byte **little-endian** `BytesN<32>` —
//! matching the convention every other 32-byte field value uses across ZKELLA
//! (`token`'s commitments, nullifiers, `Fr::to_bytes`/`from_bytes` in
//! `token::poseidon`), not the host's native big-endian point encoding. `verify`
//! reverses byte order internally before constructing each `U256`, the same
//! way `Poseidon2Hasher::hash` does, so callers never need to flip bytes
//! themselves.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype,
    crypto::bn254::{Bn254Fr, Bn254G1Affine, Bn254G2Affine},
    Address, Bytes, BytesN, Env, U256, Vec,
};

// `CircuitType`/`Error` are declared here directly rather than re-exported
// from `zkella-verifier-interface` (which defines the *same-shaped* types for
// its `VerifierClient` trait). Empirically, a `pub use` of a foreign crate's
// `#[contracttype]` here caused `stellar contract info interface` (and thus
// `stellar contract invoke`'s arg parsing) to fail with "Missing Entry
// CircuitType": the WASM linker only reliably keeps a `#[contracttype]`'s
// `contractspecv0` metadata blob when the type is defined in the same crate
// that exports functions using it — cross-crate retention depends on
// incidental codegen-unit partitioning (the same class of fragility
// documented in `verifier-interface`'s own doc comment for the unrelated
// duplicate-export bug), not on IC-defined `use` vs `pub use` semantics.
// `token`/`governance`/`compliance` only `use` (not re-export) these types
// internally to call out via `VerifierClient`, which is why they didn't hit
// this. The two definitions are `#[repr(u32)]` with identical variants, so
// they're wire-compatible: XDR encodes/decodes by discriminant, not by
// Rust's nominal type identity.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CircuitType {
    Shield = 0,
    Transfer = 1,
    Unshield = 2,
    NonMembership = 3,
    Transfer4x4 = 4,
    SwapFairness = 5,
}

/// One proof within a `verify_batch` call: the same `(public_inputs, proof)`
/// pair `verify` takes, minus `circuit`, which `verify_batch` fixes once for
/// the whole batch since every item is checked against the same VK.
#[contracttype]
#[derive(Clone)]
pub struct BatchProofItem {
    pub public_inputs: Vec<BytesN<32>>,
    pub proof:         Bytes,
}

// `token`/`governance`/`compliance` test modules construct proofs and
// register VKs against this crate's own generated `VerifierContractClient`
// (dev-dependency, for a real verifier in-process) while their *production*
// code is typed against `zkella_verifier_interface::CircuitType`. This
// conversion bridges the two nominal types at those test call sites — it's
// exact and lossless since both are `#[repr(u32)]` with identical variants.
// Not used by any `#[contractimpl]` function signature, so it doesn't
// reintroduce the spec-retention problem the type duplication above fixes.
impl From<zkella_verifier_interface::CircuitType> for CircuitType {
    fn from(c: zkella_verifier_interface::CircuitType) -> Self {
        match c {
            zkella_verifier_interface::CircuitType::Shield => CircuitType::Shield,
            zkella_verifier_interface::CircuitType::Transfer => CircuitType::Transfer,
            zkella_verifier_interface::CircuitType::Unshield => CircuitType::Unshield,
            zkella_verifier_interface::CircuitType::NonMembership => CircuitType::NonMembership,
            zkella_verifier_interface::CircuitType::Transfer4x4 => CircuitType::Transfer4x4,
            zkella_verifier_interface::CircuitType::SwapFairness => CircuitType::SwapFairness,
        }
    }
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized           = 1,
    Unauthorized             = 2,
    VkAlreadyRegistered      = 3,
    VkNotRegistered          = 4,
    InvalidVkLength          = 5,
    InvalidProofLength       = 6,
    PublicInputCountMismatch = 7,
    EmptyBatch               = 8,
    NonCanonicalInput        = 9,
}

// ── Storage ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
#[soroban_sdk::contracttype]
pub enum StorageKey {
    Admin,
    VerifyingKey(CircuitType),
    /// The key `VerifyingKey(circuit)` held immediately before the most
    /// recent `update_verifying_key` rotation, retained until
    /// `PreviousVkExpiry(circuit)` — see `update_verifying_key`'s doc comment.
    PreviousVerifyingKey(CircuitType),
    /// Ledger sequence after which `PreviousVerifyingKey(circuit)` is no
    /// longer honored by `verify`.
    PreviousVkExpiry(CircuitType),
}

/// How long a just-replaced verifying key stays valid for verification
/// alongside its replacement, in ledgers (~5s each). This is deliberately
/// short relative to `contracts/governance`'s 7-day rotation timelock — it
/// exists only to cover proofs already generated against the outgoing key
/// but not yet submitted at the moment rotation executes, not to make the
/// old key a standing, long-lived alternative.
const VK_RETENTION_WINDOW_LEDGERS: u32 = 17_280; // ~1 day

/// BN254 scalar-field modulus r, big-endian. Public inputs must be strictly
/// below it: the host reduces `x` and `x + r` to the same field element, so
/// accepting both encodings lets a caller that keys storage on raw input bytes
/// (nullifiers, commitments) be aliased into spending one note several times.
const FR_MODULUS_BE: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

// ── Wire-format constants ────────────────────────────────────────────────────

const G1_LEN: u32 = 64;
const G2_LEN: u32 = 128;
/// alpha_g1 + beta_g2 + gamma_g2 + delta_g2, before the variable-length IC array.
const VK_FIXED_LEN: u32 = G1_LEN + G2_LEN * 3;
/// A (G1) + B (G2) + C (G1).
const PROOF_LEN: u32 = G1_LEN * 2 + G2_LEN;

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct VerifierContract;

#[contractimpl]
impl VerifierContract {
    /// Initialize the contract. Can only be called once.
    /// `admin` is typically the `governance` contract's own address, so VK
    /// rotation flows through governance's timelock rather than a bare key.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&StorageKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&StorageKey::Admin, &admin);
    }

    /// Registers a verifying key for `circuit`. Fails if one is already registered
    /// — use `update_verifying_key` to rotate.
    pub fn register_verifying_key(env: Env, circuit: CircuitType, vk: Bytes) -> Result<(), Error> {
        Self::require_admin(&env)?;
        let key = StorageKey::VerifyingKey(circuit);
        if env.storage().instance().has(&key) {
            return Err(Error::VkAlreadyRegistered);
        }
        Self::validate_vk_shape(&vk)?;
        env.storage().instance().set(&key, &vk);
        Ok(())
    }

    /// Replaces the verifying key for `circuit`. Soundness-critical: a wrong VK
    /// makes the affected circuit accept forged proofs. Callers should gate this
    /// behind a timelock (see `contracts/governance`), not call it directly in
    /// steady state.
    ///
    /// The outgoing key is retained (see `PreviousVerifyingKey`) for
    /// `VK_RETENTION_WINDOW_LEDGERS` after this call: without it, a proof
    /// generated against the pre-rotation key, but not yet submitted at the
    /// moment this executes, would simply fail the instant the new key takes
    /// effect, even though the proof itself is genuine. `verify` checks the
    /// new key first — it always verifies immediately — and only falls back
    /// to the retained old key if the new key rejects the proof.
    pub fn update_verifying_key(env: Env, circuit: CircuitType, new_vk: Bytes) -> Result<(), Error> {
        Self::require_admin(&env)?;
        let key = StorageKey::VerifyingKey(circuit);
        let old_vk: Bytes = env
            .storage()
            .instance()
            .get(&key)
            .ok_or(Error::VkNotRegistered)?;
        Self::validate_vk_shape(&new_vk)?;

        let expiry = env.ledger().sequence().saturating_add(VK_RETENTION_WINDOW_LEDGERS);
        env.storage().instance().set(&StorageKey::PreviousVerifyingKey(circuit), &old_vk);
        env.storage().instance().set(&StorageKey::PreviousVkExpiry(circuit), &expiry);

        env.storage().instance().set(&key, &new_vk);
        Ok(())
    }


    /// Immediately drops the retained previous key for `circuit`. Use this
    /// when a rotation was made because the outgoing key (or its circuit) was
    /// compromised, so it must not stay acceptable for the retention window.
    pub fn revoke_previous_vk(env: Env, circuit: CircuitType) -> Result<(), Error> {
        Self::require_admin(&env)?;
        env.storage().instance().remove(&StorageKey::PreviousVerifyingKey(circuit));
        env.storage().instance().remove(&StorageKey::PreviousVkExpiry(circuit));
        Ok(())
    }

    pub fn get_verifying_key(env: Env, circuit: CircuitType) -> Result<Bytes, Error> {
        env.storage()
            .instance()
            .get(&StorageKey::VerifyingKey(circuit))
            .ok_or(Error::VkNotRegistered)
    }

    /// Verifies a Groth16 proof for `circuit` against `public_inputs`.
    /// Returns `Ok(true)`/`Ok(false)` for a well-formed proof that
    /// cryptographically checks out or not; returns `Err` only for malformed
    /// input (wrong lengths, missing VK) that indicates a caller bug rather
    /// than an invalid proof.
    ///
    /// Tries the current verifying key first — it always verifies
    /// immediately after a rotation. If that check doesn't succeed (either a
    /// clean `Ok(false)`, or a shape mismatch because the proof was built
    /// against a different-arity VK), and a just-replaced previous key is
    /// still within its retention window, retries against that key before
    /// giving up. See `update_verifying_key`'s doc comment for why.
    pub fn verify(
        env: Env,
        circuit: CircuitType,
        public_inputs: Vec<BytesN<32>>,
        proof: Bytes,
    ) -> Result<bool, Error> {
        if proof.len() != PROOF_LEN {
            return Err(Error::InvalidProofLength);
        }

        let vk_bytes: Bytes = env
            .storage()
            .instance()
            .get(&StorageKey::VerifyingKey(circuit))
            .ok_or(Error::VkNotRegistered)?;

        let current_result = Self::verify_against_vk(&env, &vk_bytes, &public_inputs, &proof);
        if let Ok(true) = current_result {
            return Ok(true);
        }

        let expiry: u32 = env
            .storage()
            .instance()
            .get(&StorageKey::PreviousVkExpiry(circuit))
            .unwrap_or(0);
        if env.ledger().sequence() > expiry {
            return current_result;
        }
        let prev_vk_bytes: Option<Bytes> = env
            .storage()
            .instance()
            .get(&StorageKey::PreviousVerifyingKey(circuit));
        match prev_vk_bytes {
            Some(prev_vk_bytes) => Self::verify_against_vk(&env, &prev_vk_bytes, &public_inputs, &proof),
            None => current_result,
        }
    }

    /// The actual Groth16 pairing check against one specific VK — factored
    /// out of `verify` so it can be tried against both the current key and,
    /// within its retention window, the just-replaced previous one.
    fn verify_against_vk(
        env: &Env,
        vk_bytes: &Bytes,
        public_inputs: &Vec<BytesN<32>>,
        proof: &Bytes,
    ) -> Result<bool, Error> {
        let (alpha_g1, beta_g2, gamma_g2, delta_g2, ic) = Self::parse_vk(env, vk_bytes)?;
        if ic.len() != public_inputs.len() + 1 {
            return Err(Error::PublicInputCountMismatch);
        }

        let (a, b, c) = Self::parse_proof(env, proof);
        let bn254 = env.crypto().bn254();
        let one = Bn254Fr::from_u256(U256::from_u32(env, 1));

        // vk_x = IC[0] + Σ x_i · IC[i+1], computed as a single batched
        // multi-scalar-multiplication (one `bn254_g1_msm` host call for all
        // N+1 terms, IC[0]'s implicit "· 1" included) rather than N separate
        // g1_mul/g1_add pairs. This is the dominant cost in this function for
        // any circuit with more than a couple of public inputs — transfer4x4's
        // 19 inputs previously meant 19 g1_mul + 19 g1_add calls here alone.
        let mut msm_points = Vec::new(env);
        let mut msm_scalars = Vec::new(env);
        msm_points.push_back(ic.get(0).unwrap());
        msm_scalars.push_back(one.clone());
        for i in 0..public_inputs.len() {
            // Public inputs are little-endian (see module doc) and must be canonical.
            let xi_fr = Self::public_input_to_fr(env, &public_inputs.get(i).unwrap())?;
            msm_points.push_back(ic.get(i + 1).unwrap());
            msm_scalars.push_back(xi_fr);
        }
        let vk_x = bn254.g1_msm(msm_points, msm_scalars);

        // -A = A · (r - 1), the group-order negation trick (no dedicated negate host call).
        let zero = Bn254Fr::from_u256(U256::from_u32(env, 0));
        let neg_one = bn254.fr_sub(&zero, &one);
        let neg_a = bn254.g1_mul(&a, &neg_one);

        // e(-A,B) * e(alpha,beta) * e(vk_x,gamma) * e(C,delta) == 1
        //   <=>  e(A,B) == e(alpha,beta) * e(vk_x,gamma) * e(C,delta)
        let g1_points = Vec::from_array(env, [neg_a, alpha_g1, vk_x, c]);
        let g2_points = Vec::from_array(env, [b, beta_g2, gamma_g2, delta_g2]);

        Ok(bn254.pairing_check(g1_points, g2_points))
    }

    /// Verifies every item in `items` against `circuit`'s current verifying
    /// key with a single combined pairing check, real batching rather than
    /// calling `verify` once per item. Useful during Tranche 3's
    /// redeployment step, when many proofs might need re-checking at once.
    /// All items must be for the currently-registered VK; this does not
    /// extend `verify`'s retention-window fallback, to keep the batching
    /// itself straightforward to reason about.
    ///
    /// Standard Groth16 batch verification: rather than checking each
    /// proof's own `e(-A,B)·e(α,β)·e(vk_x,γ)·e(C,δ) = 1` separately (4 pairings
    /// each, 4K total for K proofs), every equation is raised to a random
    /// power `r_j` and multiplied together. Since `α`, `β`, `γ`, `δ` are the
    /// same across all items (one VK), the `e(α,β)`, `e(vk_x,γ)`, and
    /// `e(C,δ)` terms collapse into one combined pairing each via linearity,
    /// leaving K + 3 total pairings instead of 4K. This is only sound if the
    /// `r_j` weights are unpredictable to whoever constructed the proofs —
    /// otherwise a forged proof could be crafted to cancel out against a
    /// genuine one under a known combination. Each `r_j` is therefore a
    /// Fiat-Shamir challenge: SHA-256 over a digest of the whole batch (circuit,
    /// every item's public inputs and proof) and the item's index, so no part
    /// of any item can be chosen after the challenges are known.
    pub fn verify_batch(
        env: Env,
        circuit: CircuitType,
        items: Vec<BatchProofItem>,
    ) -> Result<bool, Error> {
        if items.is_empty() {
            return Err(Error::EmptyBatch);
        }

        let vk_bytes: Bytes = env
            .storage()
            .instance()
            .get(&StorageKey::VerifyingKey(circuit))
            .ok_or(Error::VkNotRegistered)?;
        let (alpha_g1, beta_g2, gamma_g2, delta_g2, ic) = Self::parse_vk(&env, &vk_bytes)?;
        let bn254 = env.crypto().bn254();
        let zero = Bn254Fr::from_u256(U256::from_u32(&env, 0));
        let one = Bn254Fr::from_u256(U256::from_u32(&env, 1));

        // Transcript binds everything an adversary controls. Deriving r_j from
        // the proof alone would let public inputs be chosen after r_j is known
        // and cancelled across items.
        let mut transcript = Bytes::new(&env);
        transcript.extend_from_array(&(circuit as u32).to_be_bytes());
        transcript.extend_from_array(&items.len().to_be_bytes());
        for item in items.iter() {
            transcript.extend_from_array(&item.public_inputs.len().to_be_bytes());
            for input in item.public_inputs.iter() {
                transcript.extend_from_array(&input.to_array());
            }
            transcript.append(&item.proof);
        }
        let transcript_digest: [u8; 32] = env.crypto().sha256(&transcript).to_array();

        let mut g1_points = Vec::new(&env); // per-item -r_j·A_j terms, then the 3 combined terms
        let mut g2_points = Vec::new(&env); // matching B_j terms, then beta/gamma/delta
        let mut alpha_weight_sum = zero.clone();
        let mut vkx_points = Vec::new(&env);
        let mut vkx_scalars = Vec::new(&env);
        let mut c_points = Vec::new(&env);
        let mut c_scalars = Vec::new(&env);

        for (idx, item) in items.iter().enumerate() {
            if item.proof.len() != PROOF_LEN {
                return Err(Error::InvalidProofLength);
            }
            if ic.len() != item.public_inputs.len() + 1 {
                return Err(Error::PublicInputCountMismatch);
            }

            let (a, b, c) = Self::parse_proof(&env, &item.proof);

            // Fiat-Shamir challenge over the whole batch transcript (circuit,
            // every item's public inputs and proof), then this item's index.
            let mut challenge_input = Bytes::from_array(&env, &transcript_digest);
            challenge_input.extend_from_array(&(idx as u32).to_be_bytes());
            let digest: Bytes = env.crypto().sha256(&challenge_input).into();
            let r_j = Bn254Fr::from_u256(U256::from_be_bytes(&env, &digest));

            // vk_x_j = IC[0] + Σ x_i·IC[i+1] for this item, same MSM as verify_against_vk.
            let mut msm_points = Vec::new(&env);
            let mut msm_scalars = Vec::new(&env);
            msm_points.push_back(ic.get(0).unwrap());
            msm_scalars.push_back(one.clone());
            for i in 0..item.public_inputs.len() {
                let xi_fr = Self::public_input_to_fr(&env, &item.public_inputs.get(i).unwrap())?;
                msm_points.push_back(ic.get(i + 1).unwrap());
                msm_scalars.push_back(xi_fr);
            }
            let vk_x_j = bn254.g1_msm(msm_points, msm_scalars);

            let neg_r_j = bn254.fr_sub(&zero, &r_j);
            let neg_rj_a = bn254.g1_mul(&a, &neg_r_j);
            g1_points.push_back(neg_rj_a);
            g2_points.push_back(b);

            vkx_points.push_back(vk_x_j);
            vkx_scalars.push_back(r_j.clone());
            c_points.push_back(c);
            c_scalars.push_back(r_j.clone());
            alpha_weight_sum = bn254.fr_add(&alpha_weight_sum, &r_j);
        }

        let combined_vk_x = bn254.g1_msm(vkx_points, vkx_scalars);
        let combined_c = bn254.g1_msm(c_points, c_scalars);
        let combined_alpha = bn254.g1_mul(&alpha_g1, &alpha_weight_sum);

        g1_points.push_back(combined_alpha);
        g2_points.push_back(beta_g2);
        g1_points.push_back(combined_vk_x);
        g2_points.push_back(gamma_g2);
        g1_points.push_back(combined_c);
        g2_points.push_back(delta_g2);

        Ok(bn254.pairing_check(g1_points, g2_points))
    }

    // ── Internal helpers ──────────────────────────────────────────────────────


    /// Converts a little-endian public input to a field element, rejecting any
    /// encoding that is not already reduced (>= r).
    fn public_input_to_fr(env: &Env, input: &BytesN<32>) -> Result<Bn254Fr, Error> {
        let mut be: [u8; 32] = input.clone().into();
        be.reverse();
        // Big-endian byte arrays of equal length compare lexicographically.
        if be >= FR_MODULUS_BE {
            return Err(Error::NonCanonicalInput);
        }
        let bytes = Bytes::from_array(env, &be);
        Ok(Bn254Fr::from_u256(U256::from_be_bytes(env, &bytes)))
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

    fn validate_vk_shape(vk: &Bytes) -> Result<(), Error> {
        let len = vk.len();
        if len < VK_FIXED_LEN {
            return Err(Error::InvalidVkLength);
        }
        let ic_bytes = len - VK_FIXED_LEN;
        // Must hold at least IC[0] (the constant term) plus a whole number of G1 points.
        if ic_bytes == 0 || ic_bytes % G1_LEN != 0 {
            return Err(Error::InvalidVkLength);
        }
        Ok(())
    }

    fn parse_vk(
        env: &Env,
        vk: &Bytes,
    ) -> Result<(Bn254G1Affine, Bn254G2Affine, Bn254G2Affine, Bn254G2Affine, Vec<Bn254G1Affine>), Error> {
        let mut offset = 0u32;

        let alpha_g1 = Bn254G1Affine::from_bytes(
            vk.slice(offset..offset + G1_LEN).try_into().map_err(|_| Error::InvalidVkLength)?,
        );
        offset += G1_LEN;

        let beta_g2 = Bn254G2Affine::from_bytes(
            vk.slice(offset..offset + G2_LEN).try_into().map_err(|_| Error::InvalidVkLength)?,
        );
        offset += G2_LEN;

        let gamma_g2 = Bn254G2Affine::from_bytes(
            vk.slice(offset..offset + G2_LEN).try_into().map_err(|_| Error::InvalidVkLength)?,
        );
        offset += G2_LEN;

        let delta_g2 = Bn254G2Affine::from_bytes(
            vk.slice(offset..offset + G2_LEN).try_into().map_err(|_| Error::InvalidVkLength)?,
        );
        offset += G2_LEN;

        let ic_count = (vk.len() - offset) / G1_LEN;
        let mut ic = Vec::new(env);
        for _ in 0..ic_count {
            let point = Bn254G1Affine::from_bytes(
                vk.slice(offset..offset + G1_LEN).try_into().map_err(|_| Error::InvalidVkLength)?,
            );
            ic.push_back(point);
            offset += G1_LEN;
        }

        Ok((alpha_g1, beta_g2, gamma_g2, delta_g2, ic))
    }

    fn parse_proof(env: &Env, proof: &Bytes) -> (Bn254G1Affine, Bn254G2Affine, Bn254G1Affine) {
        let _ = env;
        let a = Bn254G1Affine::from_bytes(proof.slice(0..G1_LEN).try_into().unwrap());
        let b = Bn254G2Affine::from_bytes(proof.slice(G1_LEN..G1_LEN + G2_LEN).try_into().unwrap());
        let c = Bn254G1Affine::from_bytes(
            proof.slice(G1_LEN + G2_LEN..PROOF_LEN).try_into().unwrap(),
        );
        (a, b, c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::{Address as _, Ledger}, Env};

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        let admin = Address::generate(&env);
        let verifier = env.register_contract(None, VerifierContract);
        (env, admin, verifier)
    }

    #[test]
    fn initialize_sets_admin() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);
    }

    #[test]
    #[should_panic(expected = "already initialized")]
    fn initialize_cannot_be_called_twice() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);
        client.initialize(&admin);
    }

    #[test]
    fn rejects_malformed_vk_length() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let bad_vk = Bytes::from_array(&env, &[0u8; 10]);
        let result = client.try_register_verifying_key(&CircuitType::Shield, &bad_vk);
        assert!(result.is_err());
    }

    #[test]
    fn verify_rejects_wrong_proof_length() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        // 1 public input => IC has 2 entries (64*2=128) + VK_FIXED_LEN.
        let vk_len = VK_FIXED_LEN + G1_LEN * 2;
        let mut vk = Bytes::new(&env);
        for _ in 0..vk_len {
            vk.push_back(0u8);
        }
        client.register_verifying_key(&CircuitType::Shield, &vk);

        let bad_proof = Bytes::from_array(&env, &[0u8; 10]);
        let inputs = Vec::from_array(&env, [BytesN::from_array(&env, &[0u8; 32])]);
        let result = client.try_verify(&CircuitType::Shield, &inputs, &bad_proof);
        assert!(result.is_err());
    }

    // ── Real Groth16 correctness check ───────────────────────────────────────
    //
    // These hex vectors are NOT a real circuit's VK/proof — they're a synthetic
    // but genuinely non-degenerate Groth16-shaped tuple, generated with the
    // arkworks `ark-bn254` crate (not hand-typed, to remove transcription risk)
    // by picking beta = gamma = delta = H (the standard BN254 G2 generator) and
    // setting A = alpha + vk_x + C via real curve arithmetic, so that
    // e(A,H) = e(alpha,H)*e(vk_x,H)*e(C,H) holds by construction. Arkworks'
    // own pairing implementation confirmed lhs == rhs before these bytes were
    // captured. This proves the verifier's parsing, vk_x/MSM computation, the
    // A-negation trick, and the `pairing_check` wiring are all correct against
    // a real (if synthetic) instance of the equation Groth16 verification
    // relies on — not just that malformed input is rejected.

    fn hex_nibble(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => panic!("bad hex digit"),
        }
    }

    fn hex_push(s: &str, out: &mut Bytes) {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            out.push_back((hex_nibble(b[i]) << 4) | hex_nibble(b[i + 1]));
            i += 2;
        }
    }

    const ALPHA_G1: &str = "2c73fd312a9c3b5c2ab57c5fc12b4a1ad08b245a86ecb1744bb672da676a9b230a9f46d4388aa89ec81ef2bfc538996d9d2c0d85d0ed6a56e4655b2ba0443de7";
    const H_G2: &str = "198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa";
    const IC0: &str = "041ce74518c3b01010d18e4b0cea31d91f37c86ac5ef48a012a2402c8f6db2832d8c135bd7cb2f2a17c0e71cb91bcd3f63b817cda17a9ebcae0e768070b09022";
    const IC1: &str = "0339cdbdc22fd121c5c6157db59cc640624f15848bb5d04a57eacf0b86ecd3cd1b45491e5b92d1391e9bbc1cf7863a6a3b71d225075dc3cb99ce3c00d285febc";
    const PROOF_A: &str = "1f7649b113442a13d4baa0d453d954abd08bae93d1419dd46b251c7f65261be9285d6b9fbc0920f49d7ce68c7b821ca2a9cdd1c4dc805c922264528b8275df7e";
    const PROOF_C: &str = "0e10beca2bdb8de59dbdbbd99fc855fddc36e8fe41b4bbf8e2788ea2dac94c64132e3e311b71472ab42de89f180e9107f0a7be4429a4b7dd13073e2bd21ee484";
    const PROOF_C_BAD: &str = "16356072ef6aaf0bd6c0c360baed32029f7fa281f6b531119dcb3f54fd315c0f143ec6deb3fd3bd3f1675abf83ec01cb7d051af488a14377673a15980077b854";
    // Little-endian (verify()'s public-input convention); the arkworks
    // generator script above emits big-endian field elements, so this is
    // that same x_s value with byte order reversed.
    const PUBLIC_X: &str = "f57b5ea9f955a8d108dfc092926d315843372a5bd79e3d4206a51c6e2e74b502";

    fn build_vk(env: &Env) -> Bytes {
        let mut vk = Bytes::new(env);
        hex_push(ALPHA_G1, &mut vk);
        hex_push(H_G2, &mut vk); // beta
        hex_push(H_G2, &mut vk); // gamma
        hex_push(H_G2, &mut vk); // delta
        hex_push(IC0, &mut vk);
        hex_push(IC1, &mut vk);
        vk
    }

    fn build_proof(env: &Env, c_hex: &str) -> Bytes {
        let mut proof = Bytes::new(env);
        hex_push(PROOF_A, &mut proof);
        hex_push(H_G2, &mut proof); // B = H
        hex_push(c_hex, &mut proof);
        proof
    }

    #[test]
    fn verify_accepts_genuine_groth16_relation() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let vk = build_vk(&env);
        client.register_verifying_key(&CircuitType::Shield, &vk);

        let proof = build_proof(&env, PROOF_C);
        let mut x_bytes = Bytes::new(&env);
        hex_push(PUBLIC_X, &mut x_bytes);
        let x: BytesN<32> = x_bytes.try_into().unwrap();
        let inputs = Vec::from_array(&env, [x]);

        let ok = client.verify(&CircuitType::Shield, &inputs, &proof);
        assert!(ok, "genuine Groth16-shaped proof must verify");
    }

    #[test]
    fn verify_rejects_tampered_proof() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let vk = build_vk(&env);
        client.register_verifying_key(&CircuitType::Shield, &vk);

        // Same A, B, VK, and public input, but C swapped for an unrelated valid
        // curve point: A no longer equals alpha + vk_x + C, so the pairing
        // check must fail even though every point is individually well-formed.
        let proof = build_proof(&env, PROOF_C_BAD);
        let mut x_bytes = Bytes::new(&env);
        hex_push(PUBLIC_X, &mut x_bytes);
        let x: BytesN<32> = x_bytes.try_into().unwrap();
        let inputs = Vec::from_array(&env, [x]);

        let ok = client.verify(&CircuitType::Shield, &inputs, &proof);
        assert!(!ok, "tampered proof must not verify");
    }

    #[test]
    fn verify_rejects_wrong_public_input() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let vk = build_vk(&env);
        client.register_verifying_key(&CircuitType::Shield, &vk);

        let proof = build_proof(&env, PROOF_C);
        // Wrong public input (all zero instead of the real x): vk_x changes,
        // so A no longer equals alpha + vk_x' + C.
        let x = BytesN::from_array(&env, &[0u8; 32]);
        let inputs = Vec::from_array(&env, [x]);

        let ok = client.verify(&CircuitType::Shield, &inputs, &proof);
        assert!(!ok, "wrong public input must not verify");
    }

    // ── Real shield.circom Groth16 proof ─────────────────────────────────────
    //
    // Unlike the synthetic tuple above (arkworks-constructed, not tied to any
    // actual circuit), this test uses a genuine end-to-end artifact:
    //   1. circuits/shield/shield.circom compiled with circom 2.2.3,
    //   2. a dev Groth16 trusted setup (Powers of Tau, bn128, 2^13) generated
    //      locally with snarkjs — not reused from any external ceremony,
    //   3. a witness generated from the SDK's own cross-validated test vector
    //      (circuits/shield/shield_test_vectors.json, v2_shield_500stroops:
    //      value=500, rho=3, rcm=4, asset field matching the SDK's addressToField
    //      encoding of CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC),
    //   4. a real proof generated by `snarkjs groth16 prove`, independently
    //      confirmed valid by `snarkjs groth16 verify` before being converted
    //      to this contract's wire format by
    //      circuits/shield/build/convert_to_wire_format.py.
    //
    // This is the strongest test in this module: it proves the on-chain
    // verifier, using Soroban's real native BN254 host functions, accepts a
    // proof produced by the actual compiled shield circuit — not just a
    // hand-constructed pairing-equation instance.
    #[test]
    fn verify_accepts_real_shield_circuit_proof() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let mut vk = Bytes::new(&env);
        hex_push(SHIELD_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::Shield, &vk);

        let mut proof = Bytes::new(&env);
        hex_push(SHIELD_PROOF_HEX, &mut proof);

        let mut inputs = Vec::new(&env);
        for input_hex in SHIELD_PUBLIC_INPUTS_LE_HEX {
            let mut b = Bytes::new(&env);
            hex_push(input_hex, &mut b);
            inputs.push_back(b.try_into().unwrap());
        }

        let ok = client.verify(&CircuitType::Shield, &inputs, &proof);
        assert!(ok, "real shield.circom proof must verify");
    }

    #[test]
    fn verify_rejects_real_shield_circuit_proof_with_wrong_public_input() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let mut vk = Bytes::new(&env);
        hex_push(SHIELD_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::Shield, &vk);

        let mut proof = Bytes::new(&env);
        hex_push(SHIELD_PROOF_HEX, &mut proof);

        // Same real proof, but claim pub_value = 501 instead of 500.
        let mut inputs = Vec::new(&env);
        for (i, input_hex) in SHIELD_PUBLIC_INPUTS_LE_HEX.iter().enumerate() {
            let mut b = Bytes::new(&env);
            if i == 2 {
                hex_push("f501000000000000000000000000000000000000000000000000000000000000", &mut b);
            } else {
                hex_push(input_hex, &mut b);
            }
            inputs.push_back(b.try_into().unwrap());
        }

        let ok = client.verify(&CircuitType::Shield, &inputs, &proof);
        assert!(!ok, "real proof must not verify against a tampered public input");
    }

    const SHIELD_VK_HEX: &str = "004a10ae973df76d18cc7282cb3fd87d293ac1521d0f8c3fe1b7a8bf2cc1cd3d1c1a9a34341a0bbae006c778fbfdf7e5d0ae8cac871ecf71f17f4673e5b1f56b062d17924ef71132b66fc4695c553433e7cede3848a8bda9332958709c984a0b13bc9e495aa7512ed247262baefd60f73226f7017843977bc797462aa88034970cdc3f64b84088c7343b736148da94beb5b9ed7c19d2397c25ecb783132af1292bcac674e74ffd994b4152e1347afe87ea763ffde54a274b6e0f5653ab94de91198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa06e7e22546bfcda39cc15848d8e4a41251db89eff6fa9c9452bf2807489b70971c260eee5728432e75b3394dedf096dccaceab6f37b9796f4be6721b5cd713bd1227dcce30f5763480b5eac1e2559d33a06bf084f7c384a4afc6e39b631a6222057fda0b99afccb0dc00f26ac4ef82741d910f5a2d9cb5fe4acb3d0a9c158c5607b18a6ccb9b32f0672aca99481dffe428cff645603fca1bde5574e72fa6f5d80f57f770fdd11420efcaf2695edd2c6cbba69bb7916a152509c97707b4f900f905ce89c96bd6b54d1238b2354188a5c977fae12a2c8bca7417d83e77886bcfa41e8ba1c97e5e88bec74e98ebcad73d79a74bc39dc0bd0fced5ce4a875bbcacf02c9ba37d6555f2fcfa71a91be16824000dfdb37ab858d626b9e8874f3ab58c5418a06e2d491746dbdaf92e9b7a765ca89c6eac487ac53c2e59707b6421a744e7268eca039336571751ce3735ee30868b0d55e56d701ce86736fe837f041c834225d5ab71f8c23576cb4345708dc8c165d4feb6e18164655c363eccdf53034553058181165f38851ddd4a63579e6d7fa538ba5cd317694f3acc7e9df82bddd33301892b5bf46d45777bd2ed927749b3f3c14a478a60a3fb21effdf01f0a3981c3";

    const SHIELD_PROOF_HEX: &str = "170114f4f6e774ab611faa23f81f8277e6766b2c7e8550b0a49ec726fef50f952fcdb86a6ba62d892da137bb81e8970fd63d96dad8549e5fc6afc915490098ef18caa8939b242d200b4a11c1e31a63bbe455de405daa56c906b93116cd8958022349205a0fb2b54ae871b6ffbcd9fca7a685c4dcdc794edd61c24fc5eedc586e0c11b26503487daad56666b302175f9d27ec41ee4aaa1dfdcfe6118feb784af40511cd7fa69976cd8f2afa58c955be0ef7df32201b7352d7b3288797026c247e2bda401ea604de212be4f4b97b16a9e025e7e843023c58ee6d1de7eaab23440626c2fc779ca98db92acf0ae5d9c205ee470bcef1ed905831465c3a7f5739eada";

    // Order: commitment, value_commit, pub_value, pub_asset_id (matches
    // shield.circom's `component main {public [...]}` list and
    // ShieldPublicInputs in token/src/types.rs), little-endian 32 bytes each.
    const SHIELD_PUBLIC_INPUTS_LE_HEX: [&str; 4] = [
        "fcb8cc071cd8261e2250cb43775ad177b4f73c39850d7d85d8cc1e5a5381f807",
        "2b0a0ab5d86942b81c38e99c402c056398fb75a23605e1b082b4ac584af6b118",
        "f401000000000000000000000000000000000000000000000000000000000000",
        "d5928b929a857847c81679ac631fe6ff8fa4a5b60c71fbd4ba616580ce340601",
    ];

    // ── Real transfer_2in2out.circom Groth16 proof ───────────────────────────
    //
    // Same rigor as the real shield.circom test above: circom 2.2.3 compiled
    // circuit, a local dev Powers-of-Tau (bn128, 2^16 — this circuit's
    // ~43K constraints need it, unlike shield's much smaller one), a witness
    // built from two real input notes with independently-verified Merkle
    // paths into the same anchor (both input notes sibling-adjacent at leaf
    // indices 0/1 of an otherwise-empty depth-32 tree), value-conserving
    // outputs (600000+400000 in, 700000+300000 out, fee 0), and a proof
    // confirmed valid by `snarkjs groth16 verify` before conversion to this
    // contract's wire format. This is the direct regression test for the
    // audit-round-2 finding: it specifically exercises the fixed
    // `MerkleProof` (boolean-constrained `index[i]`) and the new
    // nullifier/output-commitment distinctness constraints in
    // transfer_2in2out/transfer.circom, with a genuine two-distinct-note
    // witness — not just a proof that the *contract* rejects a duplicated
    // synthetic proof (covered separately in token's own test suite), but
    // proof the *circuit itself* now compiles and produces valid witnesses
    // under the fixed constraint set.
    #[test]
    fn verify_accepts_real_transfer_2in2out_circuit_proof() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let mut vk = Bytes::new(&env);
        hex_push(TRANSFER2X2_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::Transfer, &vk);

        let mut proof = Bytes::new(&env);
        hex_push(TRANSFER2X2_PROOF_HEX, &mut proof);

        let mut inputs = Vec::new(&env);
        for input_hex in TRANSFER2X2_PUBLIC_INPUTS_LE_HEX {
            let mut b = Bytes::new(&env);
            hex_push(input_hex, &mut b);
            inputs.push_back(b.try_into().unwrap());
        }

        let ok = client.verify(&CircuitType::Transfer, &inputs, &proof);
        assert!(ok, "real transfer_2in2out.circom proof must verify");
    }

    #[test]
    fn verify_rejects_real_transfer_2in2out_circuit_proof_with_wrong_public_input() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let mut vk = Bytes::new(&env);
        hex_push(TRANSFER2X2_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::Transfer, &vk);

        let mut proof = Bytes::new(&env);
        hex_push(TRANSFER2X2_PROOF_HEX, &mut proof);

        // Same real proof, but claim fee = 1 instead of 0 (index 9).
        let mut inputs = Vec::new(&env);
        for (i, input_hex) in TRANSFER2X2_PUBLIC_INPUTS_LE_HEX.iter().enumerate() {
            let mut b = Bytes::new(&env);
            if i == 9 {
                hex_push("0100000000000000000000000000000000000000000000000000000000000000", &mut b);
            } else {
                hex_push(input_hex, &mut b);
            }
            inputs.push_back(b.try_into().unwrap());
        }

        let ok = client.verify(&CircuitType::Transfer, &inputs, &proof);
        assert!(!ok, "real proof must not verify against a tampered public input");
    }

    const TRANSFER2X2_VK_HEX: &str = "0b291fdaaa28add7553e94df40614c894ca8fb22a2b6b4ed7351d325cad7068e1242afa10511b208e98200b835350f44a0b2641bf06744f87f3960b79f6122880041e3d1d3043bbf9687e1c198b5fe1f3f597c26b7a97127b33b64938c49887e0c65ed7e66ecf358b07f11fc7eb9cb3ecb88ec0dcfb88c12938f1ef0fa330e601c122704e90921beaa1548ea5efcd702fa0689a866360fd874cec4d1f0507f7430573487cca5aaa0c8a3417a831694d86b1171e39d821f5d456f9be5a2d7457f198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa1effac1381d80d2415c69e56f0a628a4dc55f13b090eb3bd86de552ae9fdd68209d75da5ec727bcc8b76590d657c4b5237829ba299c3322a0e6a2b86d585550329b3b2689301966d574b1aa5445816ccdbdd0158743673efd38dbe8489552eca0f48ba739892a9729c21a0708f990a0e7d35d14969e19e5e36a793171ae93020273ab9561cac80aead7e6b6f053cc8fcf4b9446595dd3a3e1f9a444931bce03b1ffcdd49f31f6c80ccd8ffb85aa0ed72b45b0dc616becf8fdb167c2456fca43c060493e0a3b49513753c2d641bb23986e16ed42d2f8c4daa01764a0ca990b3f8157873b3f23167c5e0e7622e1b000197274b0083842bb55d2afbe3c9bea2ba610c2ee8a9f2f18a16688c010fc53f630e04ee576e53cf9729b08318f9bcd9fa5a1814b9e48f1d2a0e4c915d211c82568569ffdb61e94c8716970f22826f8220621b0d7df9e1ba7f82567dd747745625bc2759f95a0ece2421c7dec1682f5323ca28087709c7ba83b6a8d50e0fd195c1bb8e0337a5a11b2c27d3ae49ce66ccf1db09dda3b5e7df8eae2db5b8f693173719cabf64a4ae91d631fab91f2a4d6fd3dc062035efc02a059c13e283812a6febfffdd0ee682a8461d0f9f143fd7bd568b60d702be3daabf1cfd9c90c6cb9b02774d1e40db7a91196866b27a48956c7e32315f9975a0e05fa76e50cff8fbb704e7ab44e2a940bbd83688934a27766fbb7e50a377b4de6e265263a317a3e0894bc58bfa0f13223366f75d931e36cfbc1bfea2e8503a342af59ef67dd679e1ce42fdaf8ced860963b931282028129a6679ccc1f8f5828b13838959e90627bc80afcb89bb72ea25d1b2ce9b32458ea414ce4682fa5ba9b4cc2b85d574ce8cfc3d1a6437dcf269bae92498af48cba31df2739a202785747518aa6baebd7b9d060766161c2c7b3d7cefea33fae02783221d75ad71ae880161a58d9eaa65d929aef9d7a3ddca1788cf4c2981a141df7dedfa1016c137102928554207f56b1fca8871358d0dbc1a1f18ac532081d72f4c4a552e5f510ebd92e091535d619f2fcff060d1fed1899d44fa57d3178a882ea5481d9d80a1061a5bbc9834d107926c40c5149b3a8932e4f61c60e0c5b3a78a99212268bfc1cd747b7973e73cdd8d8824ac0e6472418b7a3c9666407402f34f4c63de0c467086000275e337fe4ec0b6d22f852a46ed480939d611f9e9e15c1d6e03dfb2bcd0f5884cf91fe4b50b826f502ec0d8936984a7819bec268f2678c2154149c8f52";

    const TRANSFER2X2_PROOF_HEX: &str = "2fbe7b8f55c3f1653606bcff3e96cd736c0482bb3c7620986523ab586921c938171c7917e33830460601fc4b2158cca5340d152c64eac68d8bbadbeaf5ba86d11c29bc6a3224ea93e33d39dcf3bd65aabf7f5d1277d17372125f838e53824fdf02587a6b3437cdc40690a6c4fa6d8d0d634736f9a74cebb17fe63c781757175f1b503278c3e99cdcf3da58a5e787911a05cd060d31aa4e0b777edf0a10395abe1f001998f88a31a0af3cc95890c601fcfc6c4f73f6254d4c10e5f4ff700a1f15233a185f875bee3ed05188af6fa6d47c5159a774c9703e922d95c9659741d611141f536e639216db09e391caa428b491d8760d4c7cbbae4c75bfe425efd81829";

    // Order: anchor, nullifiers[0..2], out_commitments[0..2],
    // in_value_commits[0..2], out_value_commits[0..2], fee, asset_id
    // (matches transfer_2in2out/transfer.circom's public signal list),
    // little-endian 32 bytes each.
    const TRANSFER2X2_PUBLIC_INPUTS_LE_HEX: [&str; 11] = [
        "5eacddf54c17bb9aee65b72bf9c5b64bfc8d87b785d2e160d072d95c14df2004", // ANCHOR
        "4d8868c8b074d176055d827cfd335b5e2f811a060938c6ed26ed191ccb80e207", // NF0
        "3836e7d2d0c9902fe1a488b0b4cfe190b08f347144d1df7ec978ac8259b38f06", // NF1
        "9c599bd0cd9a7e17459988299ef52e8fb34df9c81eab2536f07422fc60cc591b", // OUTCM0
        "5c9b38d8eab1c4826d7efa7edaca9dd28133063a29af6e788d7629f6af200c0e", // OUTCM1
        "c0e8993966a165503afa2c0bf36c7027554086cdfd42576860efe61126eccf17", // INVC0
        "ac00bbc2abc4158b312b88fe617ea54d47b2c0efdbfcbb1a0a414efc8cc07001", // INVC1
        "61a689b735be1fa57a76b57f35820a8ef40ea3fa9c34c3499a82eec1c5f41516", // OUTVC0
        "f9ad173388a30002a504d1bf17773e57980c9604c63655bc02bdc4ec2e7f5117", // OUTVC1
        "0000000000000000000000000000000000000000000000000000000000000000", // FEE
        "3930000000000000000000000000000000000000000000000000000000000000", // ASSETID
    ];

    // ── Real transfer_4in4out/transfer.circom Groth16 proof ─────────────────
    //
    // Same rigor as the transfer_2in2out test above, extended to 4 real
    // input notes (leaf indices 0-3 of an otherwise-empty depth-32 tree,
    // pairwise-combined then chained through empty-subtree roots) and 4
    // fresh output notes. Until now `CircuitType::Transfer4x4` was only
    // exercised against token's synthetic arkworks-based test proofs (see
    // `contracts/token/src/test_groth16.rs`), which prove the *contract*
    // plumbing but not that this specific compiled circuit's constraints
    // are satisfiable/sound — this closes that gap the same way shield,
    // transfer_2in2out, and unshield already were.
    #[test]
    fn verify_accepts_real_transfer_4in4out_circuit_proof() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let mut vk = Bytes::new(&env);
        hex_push(TRANSFER4X4_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::Transfer4x4, &vk);

        let mut proof = Bytes::new(&env);
        hex_push(TRANSFER4X4_PROOF_HEX, &mut proof);

        let mut inputs = Vec::new(&env);
        for input_hex in TRANSFER4X4_PUBLIC_INPUTS_LE_HEX {
            let mut b = Bytes::new(&env);
            hex_push(input_hex, &mut b);
            inputs.push_back(b.try_into().unwrap());
        }

        let ok = client.verify(&CircuitType::Transfer4x4, &inputs, &proof);
        assert!(ok, "real transfer_4in4out.circom proof must verify");
    }

    #[test]
    fn verify_rejects_real_transfer_4in4out_circuit_proof_with_wrong_public_input() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let mut vk = Bytes::new(&env);
        hex_push(TRANSFER4X4_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::Transfer4x4, &vk);

        let mut proof = Bytes::new(&env);
        hex_push(TRANSFER4X4_PROOF_HEX, &mut proof);

        // Same real proof, but claim fee = 0 instead of 1000 (index 17).
        let mut inputs = Vec::new(&env);
        for (i, input_hex) in TRANSFER4X4_PUBLIC_INPUTS_LE_HEX.iter().enumerate() {
            let mut b = Bytes::new(&env);
            if i == 17 {
                hex_push("0000000000000000000000000000000000000000000000000000000000000000", &mut b);
            } else {
                hex_push(input_hex, &mut b);
            }
            inputs.push_back(b.try_into().unwrap());
        }

        let ok = client.verify(&CircuitType::Transfer4x4, &inputs, &proof);
        assert!(!ok, "real proof must not verify against a tampered public input");
    }

    const TRANSFER4X4_VK_HEX: &str = "0b291fdaaa28add7553e94df40614c894ca8fb22a2b6b4ed7351d325cad7068e1242afa10511b208e98200b835350f44a0b2641bf06744f87f3960b79f6122880041e3d1d3043bbf9687e1c198b5fe1f3f597c26b7a97127b33b64938c49887e0c65ed7e66ecf358b07f11fc7eb9cb3ecb88ec0dcfb88c12938f1ef0fa330e601c122704e90921beaa1548ea5efcd702fa0689a866360fd874cec4d1f0507f7430573487cca5aaa0c8a3417a831694d86b1171e39d821f5d456f9be5a2d7457f198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa2b397a7f41e76babc831c209931b3941ed08f8a6806df0d598176baa6e7510a40ef2052047a4a9c442b58ee948dd8ab76194703feb1376bc2688ba6e9abe623a0f7abf8cba42e213aa72f7e6f41b4a1b50302e2945e7a6c41652b562fcb2e5e127eaa557eba00f4db1f8b3b1b422c2502d7c84823a0a758622da9994cb5d98021b951e3bcf343ff1b794d695d3344ff9221f653b79d044ab2ed3d546b0c6dc041499cb8656c6927c567c450e84aaa1f5b4b7d97dec56f5317dda852a6a39d57b1e5d5eeaee14cc098f0d75182d5f54f653f59f112a91383802f77a5d73de34f125d12ff34a12da37d64c7eb24fdf39c7cf807a8ab24a5be9ef9ce1a533cbb034093415d6065330b03011576274344cf34ebb1f3ae1191542c3a243880c2c2db52cc0db7f9b9bbdce8114f8ffdbbc21e415e7a74fd8820a0d1a9fe5040732be54086240a1a9c7f1ebebcee88b14bc70fc8109ae1c480823e4781f135db1171268097d61758a7ea8c57898011e1776a09946b6a583b264446121b8c6626c69699520bb1d46bd8bd19c3311314fa90fc413a00a3bc2ceb5b9f27e2a3358684602f205419519fbbf7cd2d28db295e16a06a4435dfbe90daf48f4839d504b42cb82a405d1ef42872a7ee40bc54ccb819cfb63cde6c2b62a33734778bfc07892ca639f02114bd4d190249c995eea2aceb44b9e704c56b30e14162779ccd22b022ae9681885f71476f9dda8112136fd849f0ad5dd847a989eff1c3976a824364ee2cc23288af940cec268afdc4a48207cc9f578c33ce5584966eb753529567fbe88f222013eb0ab3657d8b39cde00ed235d10aeb1aef0dab3afe51f53746d618b4866fb1cfeeb61f4e8d2c30cc84f6d412fd90571503a74c6d368248ca13d30a0bbadd0253ec8f486d618ad2d2b29ac31893b057f2737c169104a1927806d455bec8daf10def57de819d0d37493c1ed85926e6a6dbc2757c097d5f4b2d36d35850e59d22c577bf3ef9b92c553f5895bd0991dbb36684b8050e22f9a59e403c51a8393982a820e478c96433b3848dafdaf93c10585a56094a4dda83a94178681effc3ac21546ebd3741134e9bccba63789345d03f06dd01f65479e4a92d13261e3cbe50f0ba336ae0275005dba0fb2f17cde58f62746863dace1e05e336297723f30e43906dd703082c82b50360228123f2caa69a3d2364871a6399a244f054bd960024b0e7874a5eb80a777bac3eb83247c6ad6ba907734a5156c1c61e9e378f72f75ac21aae28fae4136623b0548a3c3179f3d6cd73442a3e0be525bf711a19ee0119d1edc9cf9453f6830590dd542cbb89b6d39ebda05ccda0e6f10fefd6ff3b2da2121d5688042993c5a2dea20ec4790c73e3d9da59eab78d18998e4adccc53e01921e1b72901b838fb65a3bd40faf1ab9b101e4cb682af4f7859db9bd8dc993f20d0cbdde687a4016cc90e1ae56eae14fe6b36fcc48196fadb213d997f0ca79cad907d6094435792beb97c97df9155be99d186b8559f22dbef38241fc485a98c8d61fa298ac9c2eef00911db8fa02799221ada937fde85f69c389fcec5298539066103e3bb6d41a42c33f7bdbf175f44651b40fe0d75dd849b61c63ed4cf81806fb254bb250250e41bbebadc8b033086736d679579f0e1ef51b686e9c2476262e762c3b77df71fe33c728168b6877b7b2e230421833795d218420b6f8fb7c13234f0e10fcbee4763b9e6650983ce0d16796afcabbf96f49fbbe8cd0dc7a0bdd8998248b158f26be061806570501bebfed6ae728cae1f108ac3b4460d6d3f07498f1261655cf98dba151c2b76ac49cefeaa17b19006fa5bc1b51467601b1dddcb0b826e98656989b7577fd9eead04648ec38fc1703dfe8d0cdfec7662199c774d63c13cc48319b533f46ae1d6b65721a12c479885dbe0a43ab7784bc7c95ef03345809bd0f85a4feca1c4aa64c403365f98f7361ae87bcad2bbe8d34fd9ab1275729";

    const TRANSFER4X4_PROOF_HEX: &str = "2e38e75b5cc798f6376c9165e62c9ac193ebcbfa4458ac52e2a99b35fe0955621df804f31eb57132ad6280a0d7a2bbe70a9b8c0234d2994892c526b918ef2e380c4cf3cc63cf546aa4b3083ce21366c809c131032fdf4b52bec781e0b2fcfe5929d986127fdacd88fb26222a5d9add22c4c6d1d7d4052b92afa8200e75e2d12206dd030d3d44065cc9dfe9d74e31db8cb0ffc221bd8d78661532e4e1c5dd88e11f6900c901aa89480b9d856de1cae1834a9b960e3e65cc2e2bb8114469cc1e9f203a8244072b9350493f165b23559c902abdfbe6454e622ad1c399f3676971ac10e8238377c959b041389b5f5deba319e0648564f4d4674de7451608bddb1124";

    // Order: anchor, nullifiers[0..4], out_commitments[0..4],
    // in_value_commits[0..4], out_value_commits[0..4], fee, asset_id
    // (matches transfer_4in4out/transfer.circom's public signal list),
    // little-endian 32 bytes each.
    const TRANSFER4X4_PUBLIC_INPUTS_LE_HEX: [&str; 19] = [
        "147584905c24b201296f7d3a9a8eeb35b186b1218de05dc9fdd29148fdfce71d", // ANCHOR
        "4d8868c8b074d176055d827cfd335b5e2f811a060938c6ed26ed191ccb80e207", // NF0
        "3836e7d2d0c9902fe1a488b0b4cfe190b08f347144d1df7ec978ac8259b38f06", // NF1
        "6bb2bcb4fde4ab5052e1b0ef4e1e35132fd29f01252b5d475c579d1d53c1642e", // NF2
        "448ccf7ea6b39e6749a56b7426e306fcd781174bff9bb91388d8d6847d30c216", // NF3
        "42d237682b0b73e57f5ee6e6aab2fdf293064962c640d93f605c0a83a70d5202", // OUTCM0
        "009cd1699e8fe79b772b46e679e0c8344872742640493126e2ae02c73f903d2b", // OUTCM1
        "0edfc00fb606304a7846dc6df386b2a321b6d09ceb075ba56097991830537f15", // OUTCM2
        "ddf1d1a50c5092e32c74baaa2a321897695cd0653845db07acdcb6686bbd6504", // OUTCM3
        "7d44a357ea0366e2404d07ab846882869d94640645b32c89fda24f9785a61624", // INVC0
        "26f76c57c525c191a8c937bc85f5de084ade8248554a6ff53bf1c6a5e83edc21", // INVC1
        "b002b6c5ccb8007b79b51d5bf7c2884a7d1b9dc2f639097f7cec02823c55f104", // INVC2
        "e5e69bbdaf3cd95902a01016fbdcfec44d2d67bd209eee8053892c3039cadb1a", // INVC3
        "44f345daccb43789709bc3111781d71bd0e4650d4e6fef220f197212c9f8d82f", // OUTVC0
        "a7e59d2a27c022e8f11fdf897ec241477624f803755370f2462c4a61ae8bd50c", // OUTVC1
        "c914a6a02c0c012500cc180c368d54cbf84fc3eea20f804e0635a92d37b8d314", // OUTVC2
        "f5832b40e1c88a8b1b7bbd63e7474e0411431987422ffa24eda60d9d2d110924", // OUTVC3
        "e803000000000000000000000000000000000000000000000000000000000000", // FEE (1000)
        "3930000000000000000000000000000000000000000000000000000000000000", // ASSETID (12345)
    ];

    // ── Real swap/swap_fairness.circom Groth16 proof ─────────────────────────
    //
    // Regression test for the audit finding that `min_amount_out` was never
    // bound to `intent_commitment` (only `amount_in`/`max_slippage_bps` were),
    // letting a prover supply an arbitrarily low `min_amount_out` at reveal
    // time regardless of what slippage tolerance was actually committed to —
    // defeating the swap's front-running protection the moment
    // `contracts/swap::reveal_and_claim` started verifying this proof for
    // real. Fixed in `swap_fairness.circom` by deriving `min_amount_out` as
    // `floor(amount_in * (10000 - max_slippage_bps) / 10000)` in-circuit.
    // This proof uses a legitimately-derived `min_amount_out`; the
    // "forged" test below uses the exact attack value (0) the finding
    // described and confirms it's rejected.
    #[test]
    fn verify_accepts_real_swap_fairness_circuit_proof() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let mut vk = Bytes::new(&env);
        hex_push(SWAP_FAIRNESS_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::SwapFairness, &vk);

        let mut proof = Bytes::new(&env);
        hex_push(SWAP_FAIRNESS_PROOF_HEX, &mut proof);

        let mut inputs = Vec::new(&env);
        for input_hex in SWAP_FAIRNESS_PUBLIC_INPUTS_LE_HEX {
            let mut b = Bytes::new(&env);
            hex_push(input_hex, &mut b);
            inputs.push_back(b.try_into().unwrap());
        }

        let ok = client.verify(&CircuitType::SwapFairness, &inputs, &proof);
        assert!(ok, "real swap_fairness.circom proof with correctly-derived min_amount_out must verify");
    }

    #[test]
    fn verify_rejects_real_swap_fairness_circuit_proof_with_forged_min_amount_out() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let mut vk = Bytes::new(&env);
        hex_push(SWAP_FAIRNESS_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::SwapFairness, &vk);

        let mut proof = Bytes::new(&env);
        hex_push(SWAP_FAIRNESS_PROOF_HEX, &mut proof);

        // Same real proof, but claim min_amount_out = 0 instead of the real
        // 975000 (index 4) — exactly the attack the finding described. The
        // proof was generated against a witness with the real value, so a
        // tampered public input here must fail the pairing check (this is a
        // different, complementary check from the witness-generation-time
        // rejection already confirmed when building this test's fixtures:
        // circom's own witness calculator refuses to build a witness at all
        // for a forged min_amount_out, since the in-circuit constraint now
        // catches it before a proof can even be produced for one).
        let mut inputs = Vec::new(&env);
        for (i, input_hex) in SWAP_FAIRNESS_PUBLIC_INPUTS_LE_HEX.iter().enumerate() {
            let mut b = Bytes::new(&env);
            if i == 4 {
                hex_push("0000000000000000000000000000000000000000000000000000000000000000", &mut b);
            } else {
                hex_push(input_hex, &mut b);
            }
            inputs.push_back(b.try_into().unwrap());
        }

        let ok = client.verify(&CircuitType::SwapFairness, &inputs, &proof);
        assert!(!ok, "real proof must not verify against a forged min_amount_out");
    }

    const SWAP_FAIRNESS_VK_HEX: &str = "004a10ae973df76d18cc7282cb3fd87d293ac1521d0f8c3fe1b7a8bf2cc1cd3d1c1a9a34341a0bbae006c778fbfdf7e5d0ae8cac871ecf71f17f4673e5b1f56b062d17924ef71132b66fc4695c553433e7cede3848a8bda9332958709c984a0b13bc9e495aa7512ed247262baefd60f73226f7017843977bc797462aa88034970cdc3f64b84088c7343b736148da94beb5b9ed7c19d2397c25ecb783132af1292bcac674e74ffd994b4152e1347afe87ea763ffde54a274b6e0f5653ab94de91198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa0019c0c90910f83e18e7d744ad362f81d5c882f9ec94556e92729f9c85cf1ea2156027c50b35e7747feeed12b31a9ec244949729a6f7c10fa9d801f774ad35e80ad43904da733b498cdfbd3fdf0a4ddafe471973885241a8656ee6e340d2d4b21799ad345557a6e73e3291a5205e46066c61511b0fdc5b12bee33a9fb0c1e7370c1daaee301f30c80ccfb72d3ccfa6c103f9b90efb048bc9f8813f44e06351d40b272bbc3ece2cf32874d377dd3c7a150dbab40adf6c2004ec431d36d84fb7f604b22c4c322753459d75c903c9178dd3c7204f0ec09cb0f81c40d2cdb8ef6c1e0af5246493de566608beb1ea1f41d186d298e0384cbe7bef5d94ba46db1a95561c1ac38c8ecfad040707f99e57d3fb1940edf5b44e732d2dc740bf1e328b14d3099a44a21db61d0a73c0d065fa72924932673b910b85b9fac2779bda8acb7ba51be2368fbe7dcedfb8e0fb0e2318df05795b3cf77eda524790aebf236c3a3aad0924e28b0cd7eec9c1467afbd1186ddd7b43d3fa5ce85b5b0d957a399a48118d174070bbb8c7f80beb451b8d47865d456656db2f9fc377e5cfde2af4a879720716393ebe785dbccbe1f20c367f19c904c505fec6200639c59c03d32e5ae0f8a41637934178424131de727a304334c2db4f6fc68cda6d04528d7620a48d376e651120126e0c274a79ecdc92860d55c657368ec276c2b0fa1c43d3efc22b52733b";

    const SWAP_FAIRNESS_PROOF_HEX: &str = "17d585e740cdf41cf8f822ed140dbecd074791783f7ce454ea5eb595eec61095141851a80195169e0dc8f044eff01b9b9b8338c5615057866ce92f3df46c64b707346e839a6ec0f4427a6463a44f1aeecad944c677c682ba5eeb16673b2871570f3a9739d85eff1a6872cba47b63f4f67578d4c0960a124ac32fd3213a099dda1a8400c4b70e505ceef65757a6fa433b5d1858240e7940be68abc590a100c54f1041d8154b03fb5b74e51c51ebd6afc687d69f58f56575cdb07472e9bfdf8b3d0bc643c67a3bd548ab2017f9e62715f0488c5ef1e0912c1b3120760c0691de720d76f6cb8246b67c7338d0c13e08dcae0eed639da5ba9fbdd1fe28bcddcff76b";

    // Order: intent_commitment, asset_in, asset_out, amount_out, min_amount_out
    // (matches swap_fairness.circom's public signal list), little-endian 32 bytes each.
    const SWAP_FAIRNESS_PUBLIC_INPUTS_LE_HEX: [&str; 5] = [
        "2144195246ec906201992a0f891b606068775a5138dc7e9ac820eb6b5ffaa100", // INTENT_COMMITMENT
        "6f00000000000000000000000000000000000000000000000000000000000000", // ASSET_IN (111)
        "de00000000000000000000000000000000000000000000000000000000000000", // ASSET_OUT (222)
        "20f40e0000000000000000000000000000000000000000000000000000000000", // AMOUNT_OUT (980000)
        "98e00e0000000000000000000000000000000000000000000000000000000000", // MIN_AMOUNT_OUT (975000)
    ];

    // ── Real unshield.circom Groth16 proof ───────────────────────────────────
    //
    // Same rigor as the transfer_2in2out test above: a genuine single-note
    // witness (one real leaf at index 0 of an otherwise-empty depth-32 tree,
    // its Merkle path independently verified), proof confirmed valid by
    // `snarkjs groth16 verify` before conversion. This also exercises the
    // fixed `MerkleProof` gadget, from the unshield side.
    #[test]
    fn verify_accepts_real_unshield_circuit_proof() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let mut vk = Bytes::new(&env);
        hex_push(UNSHIELD_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::Unshield, &vk);

        let mut proof = Bytes::new(&env);
        hex_push(UNSHIELD_PROOF_HEX, &mut proof);

        let mut inputs = Vec::new(&env);
        for input_hex in UNSHIELD_PUBLIC_INPUTS_LE_HEX {
            let mut b = Bytes::new(&env);
            hex_push(input_hex, &mut b);
            inputs.push_back(b.try_into().unwrap());
        }

        let ok = client.verify(&CircuitType::Unshield, &inputs, &proof);
        assert!(ok, "real unshield.circom proof must verify");
    }

    #[test]
    fn verify_rejects_real_unshield_circuit_proof_with_wrong_public_input() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let mut vk = Bytes::new(&env);
        hex_push(UNSHIELD_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::Unshield, &vk);

        let mut proof = Bytes::new(&env);
        hex_push(UNSHIELD_PROOF_HEX, &mut proof);

        // Same real proof, but claim pub_value = 250001 instead of 250000 (index 2).
        let mut inputs = Vec::new(&env);
        for (i, input_hex) in UNSHIELD_PUBLIC_INPUTS_LE_HEX.iter().enumerate() {
            let mut b = Bytes::new(&env);
            if i == 2 {
                hex_push("91d0030000000000000000000000000000000000000000000000000000000000", &mut b);
            } else {
                hex_push(input_hex, &mut b);
            }
            inputs.push_back(b.try_into().unwrap());
        }

        let ok = client.verify(&CircuitType::Unshield, &inputs, &proof);
        assert!(!ok, "real proof must not verify against a tampered public input");
    }

    const UNSHIELD_VK_HEX: &str = "0b291fdaaa28add7553e94df40614c894ca8fb22a2b6b4ed7351d325cad7068e1242afa10511b208e98200b835350f44a0b2641bf06744f87f3960b79f6122880041e3d1d3043bbf9687e1c198b5fe1f3f597c26b7a97127b33b64938c49887e0c65ed7e66ecf358b07f11fc7eb9cb3ecb88ec0dcfb88c12938f1ef0fa330e601c122704e90921beaa1548ea5efcd702fa0689a866360fd874cec4d1f0507f7430573487cca5aaa0c8a3417a831694d86b1171e39d821f5d456f9be5a2d7457f198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa028dbc6d50946ff9437d5313c639902caa6ea3f0d9ee6e29bdd62b4753dee4e91190f122364363a5752b4bc8fd7aec8ec0c236eaf099e64dbb7fb60bcb05455320fcb6b768bb1d9b35ac7613310b44f94656ed5abb2760866e202634aacc882a041cdd332c6d893997436e0a9bbd4807af2cb2ab2cf125f216adce9bdc7cfa4202c369c7679f429a103a052efcab3313203a6f6b8e82c6a2ce19add77bdfc3e8206d8b89632770915133cab480bf130759db86f2569b681d4b3432cbaca9c7f22e9e9f8523925e7868673857aba734c2ca50aa02c8deb44c40792da4032affe61bc0d8c3662587ef9035e2e83f03f735af725c1700e9d36744baf92951f2cb800bad3a059a1dae4575ab11a0d943bf41304aeeca86e460aeee3857d070052af60d527d70793f946f63336b23c896d4779382858f1c52355b6768084bebc3fef22658432b1db72185e7c4501680bb94641cf70a8af2e861f19eabc5f958729f1526cbb504cc08d49409c630800e14c3b61069cf408172b807f5f362ded2d2b865170adf28043ac603ea0d3ecec766069f8d303ee75703a7cca32b494301b173ec2f798d1d6be3e3cbf1ba6298214801e727edab845a3d3a01b52ef86508e1e82e2e59431282e194d04c8a0da95a7e3c9a3c4bb897e0527ee1ced4f2c11051b56c11e28c1f1545895bd7538ad71bc99a429b32ca95df1b118c24f2537a6efb9be8";

    const UNSHIELD_PROOF_HEX: &str = "12df8a60cfe4b8345cc56a5b00d65c7c455678526daefbcf9db86fc8ee841a9e2e234bb7a4b606d5ca6566daa8a1a1895b39ad1505d8ccc44fe089a7ed49fc99229feef33db3d57c064053bc0e99a7128f36f7fd80f5578660ab76868dbfa5f814eb9576fd0219e143e20ecba4d073def09ae9f01b26093bd0210a92e7c038eb1219827645ff0d0fef18b759d3efd47ff8da25abd5fa47f688e771398655a9e518458c8973c4d15400772a08df1abfe6f1dc732093aaaf09d556c16bc9d9de7c1ea57b7b134b14bc911907b22170190fcbfdcc31084bff2ec9e8f15eb3f9457d243283289af9809d1e7d5a001a7016af6fe6f7646a86ded24ed5a0ae162b36e5";

    // Order: anchor, nullifier, pub_value, pub_asset_id, recipient_hash
    // (matches unshield/unshield.circom's public signal list),
    // little-endian 32 bytes each.
    const UNSHIELD_PUBLIC_INPUTS_LE_HEX: [&str; 5] = [
        "1f0d6b5ede9f49a76e976c141ff30b9e7f5fd57ef77db70c1851d787209d9f28", // ANCHOR
        "bb59c47fd18dfa22d7e51c1e2dafc346b60b6a60e39639e15ceb43f3bbe90609", // NULLIFIER
        "90d0030000000000000000000000000000000000000000000000000000000000", // PUBVALUE
        "cd81010000000000000000000000000000000000000000000000000000000000", // PUBASSETID
        "2a00000000000000000000000000000000000000000000000000000000000000", // RECIPIENTHASH
    ];

    // ── Deliverable 3: VK retention window ────────────────────────────────

    #[test]
    fn update_verifying_key_retains_old_key_within_window_then_expires() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        // Two genuinely distinct, independently-valid (VK, proof) pairs —
        // borrowing the real unshield and real swap_fairness fixtures purely
        // as two unrelated valid Groth16 instances, not for their real
        // circuit semantics, to exercise key rotation mechanics. Both share
        // the same 5-public-input arity, matching how a real rotation always
        // replaces a VK with another for the *same* circuit (same arity) —
        // unlike a shield(4)/transfer2x2(11) pair, which would make `verify`
        // return a `PublicInputCountMismatch` `Err` against the wrong key
        // rather than a clean `Ok(false)`, a case that can't arise from a
        // real same-circuit rotation and would otherwise make this test
        // exercise a scenario `update_verifying_key` was never meant to
        // handle.
        let mut vk_a = Bytes::new(&env);
        hex_push(UNSHIELD_VK_HEX, &mut vk_a);
        let mut proof_a = Bytes::new(&env);
        hex_push(UNSHIELD_PROOF_HEX, &mut proof_a);
        let mut inputs_a = Vec::new(&env);
        for input_hex in UNSHIELD_PUBLIC_INPUTS_LE_HEX {
            let mut b = Bytes::new(&env);
            hex_push(input_hex, &mut b);
            inputs_a.push_back(b.try_into().unwrap());
        }

        let mut vk_b = Bytes::new(&env);
        hex_push(SWAP_FAIRNESS_VK_HEX, &mut vk_b);
        let mut proof_b = Bytes::new(&env);
        hex_push(SWAP_FAIRNESS_PROOF_HEX, &mut proof_b);
        let mut inputs_b = Vec::new(&env);
        for input_hex in SWAP_FAIRNESS_PUBLIC_INPUTS_LE_HEX {
            let mut b = Bytes::new(&env);
            hex_push(input_hex, &mut b);
            inputs_b.push_back(b.try_into().unwrap());
        }

        client.register_verifying_key(&CircuitType::Shield, &vk_a);
        assert!(client.verify(&CircuitType::Shield, &inputs_a, &proof_a), "proof A must verify against key A before any rotation");

        // Rotate to key B. The new key must verify immediately...
        client.update_verifying_key(&CircuitType::Shield, &vk_b);
        assert!(client.verify(&CircuitType::Shield, &inputs_b, &proof_b), "proof B must verify against key B immediately after rotation");
        // ...and proof A, built against the now-retired key A, must still
        // verify too, within the retention window.
        assert!(client.verify(&CircuitType::Shield, &inputs_a, &proof_a), "proof A must still verify against retained key A within the retention window");

        // Advance past the retention window.
        env.ledger().with_mut(|li| {
            li.sequence_number += VK_RETENTION_WINDOW_LEDGERS + 1;
        });

        assert!(!client.verify(&CircuitType::Shield, &inputs_a, &proof_a), "proof A must no longer verify once the retention window has expired");
        assert!(client.verify(&CircuitType::Shield, &inputs_b, &proof_b), "proof B, the current key, must still verify after the old key expires");
    }

    // Audit regression: a public input x and x + r are the same field
    // element. The verifier must not accept the second encoding, or any
    // caller keying storage on raw input bytes (nullifiers) can be aliased.
    #[test]
    fn verify_rejects_non_canonical_public_input_encoding() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);
        let mut vk = Bytes::new(&env);
        hex_push(SHIELD_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::Shield, &vk);
        let mut proof = Bytes::new(&env);
        hex_push(SHIELD_PROOF_HEX, &mut proof);

        let mut inputs = Vec::new(&env);
        for (i, input_hex) in SHIELD_PUBLIC_INPUTS_LE_HEX.iter().enumerate() {
            let mut b = Bytes::new(&env);
            if i == 2 {
                hex_push("f50100f093f5e1439170b97948e833285d588181b64550b829a031e1724e6430", &mut b); // 500 + r
            } else {
                hex_push(input_hex, &mut b);
            }
            inputs.push_back(b.try_into().unwrap());
        }
        let res = client.try_verify(&CircuitType::Shield, &inputs, &proof);
        assert!(
            !matches!(res, Ok(Ok(true))),
            "verify accepted the non-canonical alias of a public input"
        );
    }

    // ── Deliverable 3: batch verification ─────────────────────────────────

    #[test]
    fn verify_batch_accepts_multiple_valid_proofs_in_one_combined_check() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let mut vk = Bytes::new(&env);
        hex_push(SHIELD_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::Shield, &vk);

        let mut proof = Bytes::new(&env);
        hex_push(SHIELD_PROOF_HEX, &mut proof);
        let mut inputs = Vec::new(&env);
        for input_hex in SHIELD_PUBLIC_INPUTS_LE_HEX {
            let mut b = Bytes::new(&env);
            hex_push(input_hex, &mut b);
            inputs.push_back(b.try_into().unwrap());
        }

        // The same genuine proof, submitted three times in one batch — a
        // real, independent test of the combined-pairing math (three -r_j·A_j
        // terms plus the three collapsed alpha/vk_x/C terms), not just a
        // single-item pass-through.
        let items = Vec::from_array(&env, [
            BatchProofItem { public_inputs: inputs.clone(), proof: proof.clone() },
            BatchProofItem { public_inputs: inputs.clone(), proof: proof.clone() },
            BatchProofItem { public_inputs: inputs.clone(), proof: proof.clone() },
        ]);
        let ok = client.verify_batch(&CircuitType::Shield, &items);
        assert!(ok, "a batch of three genuine, identical valid proofs must verify");
    }

    #[test]
    fn verify_batch_rejects_when_any_item_is_tampered() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);

        let mut vk = Bytes::new(&env);
        hex_push(SHIELD_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::Shield, &vk);

        let mut proof = Bytes::new(&env);
        hex_push(SHIELD_PROOF_HEX, &mut proof);
        let mut inputs = Vec::new(&env);
        for input_hex in SHIELD_PUBLIC_INPUTS_LE_HEX {
            let mut b = Bytes::new(&env);
            hex_push(input_hex, &mut b);
            inputs.push_back(b.try_into().unwrap());
        }

        // Same real proof, but the second item claims a tampered public
        // input (pub_value off by one, same tamper the single-proof
        // `verify_rejects_real_shield_circuit_proof_with_wrong_public_input`
        // test above uses) — the batch as a whole must reject, not silently
        // accept because two of the three items are individually genuine.
        let mut tampered_inputs = Vec::new(&env);
        for (i, input_hex) in SHIELD_PUBLIC_INPUTS_LE_HEX.iter().enumerate() {
            let mut b = Bytes::new(&env);
            if i == 2 {
                hex_push("f501000000000000000000000000000000000000000000000000000000000000", &mut b);
            } else {
                hex_push(input_hex, &mut b);
            }
            tampered_inputs.push_back(b.try_into().unwrap());
        }

        let items = Vec::from_array(&env, [
            BatchProofItem { public_inputs: inputs.clone(), proof: proof.clone() },
            BatchProofItem { public_inputs: tampered_inputs, proof: proof.clone() },
        ]);
        let ok = client.verify_batch(&CircuitType::Shield, &items);
        assert!(!ok, "a batch containing one item with a tampered public input must not verify");
    }

    #[test]
    fn verify_batch_rejects_empty_batch() {
        let (env, admin, verifier) = setup();
        env.mock_all_auths();
        let client = VerifierContractClient::new(&env, &verifier);
        client.initialize(&admin);
        let mut vk = Bytes::new(&env);
        hex_push(SHIELD_VK_HEX, &mut vk);
        client.register_verifying_key(&CircuitType::Shield, &vk);

        let empty: Vec<BatchProofItem> = Vec::new(&env);
        let result = client.try_verify_batch(&CircuitType::Shield, &empty);
        assert_eq!(result, Err(Ok(Error::EmptyBatch)));
    }
}
