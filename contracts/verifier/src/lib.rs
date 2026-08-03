#![no_std]

//! Groth16/BN254 verifying-key registry, shared across ZKELLA's circuits.
//!
//! Stores one verifying key per [`CircuitType`] and exposes [`VerifierContract::verify`],
//! a generic Groth16 pairing check against Soroban's native BN254 host functions
//! (`bn254_g1_add`, `bn254_g1_mul`, `bn254_multi_pairing_check`, protocol 25+).
//! Keeping this in its own contract — rather than embedding the VK in `ct20`'s
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
//! (`ct20`'s commitments, nullifiers, `Fr::to_bytes`/`from_bytes` in
//! `ct20::poseidon`), not the host's native big-endian point encoding. `verify`
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
// `ct20`/`governance`/`compliance` only `use` (not re-export) these types
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

// `ct20`/`governance`/`compliance` test modules construct proofs and
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
}

// ── Storage ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
#[soroban_sdk::contracttype]
pub enum StorageKey {
    Admin,
    VerifyingKey(CircuitType),
}

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
    pub fn update_verifying_key(env: Env, circuit: CircuitType, new_vk: Bytes) -> Result<(), Error> {
        Self::require_admin(&env)?;
        let key = StorageKey::VerifyingKey(circuit);
        if !env.storage().instance().has(&key) {
            return Err(Error::VkNotRegistered);
        }
        Self::validate_vk_shape(&new_vk)?;
        env.storage().instance().set(&key, &new_vk);
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
    pub fn verify(
        env: Env,
        circuit: CircuitType,
        public_inputs: Vec<BytesN<32>>,
        proof: Bytes,
    ) -> Result<bool, Error> {
        let vk_bytes: Bytes = env
            .storage()
            .instance()
            .get(&StorageKey::VerifyingKey(circuit))
            .ok_or(Error::VkNotRegistered)?;

        if proof.len() != PROOF_LEN {
            return Err(Error::InvalidProofLength);
        }

        let (alpha_g1, beta_g2, gamma_g2, delta_g2, ic) = Self::parse_vk(&env, &vk_bytes)?;
        if ic.len() != public_inputs.len() + 1 {
            return Err(Error::PublicInputCountMismatch);
        }

        let (a, b, c) = Self::parse_proof(&env, &proof);
        let bn254 = env.crypto().bn254();

        // vk_x = IC[0] + Σ x_i · IC[i+1]
        let mut vk_x = ic.get(0).unwrap();
        for i in 0..public_inputs.len() {
            // Public inputs are little-endian (see module doc); the host's
            // U256/Bn254Fr are big-endian, so reverse before constructing.
            let mut xi_be: [u8; 32] = public_inputs.get(i).unwrap().into();
            xi_be.reverse();
            let xi_bytes = Bytes::from_array(&env, &xi_be);
            let xi_fr = Bn254Fr::from_u256(U256::from_be_bytes(&env, &xi_bytes));
            let term = bn254.g1_mul(&ic.get(i + 1).unwrap(), &xi_fr);
            vk_x = bn254.g1_add(&vk_x, &term);
        }

        // -A = A · (r - 1), the group-order negation trick (no dedicated negate host call).
        let zero = Bn254Fr::from_u256(U256::from_u32(&env, 0));
        let one = Bn254Fr::from_u256(U256::from_u32(&env, 1));
        let neg_one = bn254.fr_sub(&zero, &one);
        let neg_a = bn254.g1_mul(&a, &neg_one);

        // e(-A,B) * e(alpha,beta) * e(vk_x,gamma) * e(C,delta) == 1
        //   <=>  e(A,B) == e(alpha,beta) * e(vk_x,gamma) * e(C,delta)
        let g1_points = Vec::from_array(&env, [neg_a, alpha_g1, vk_x, c]);
        let g2_points = Vec::from_array(&env, [b, beta_g2, gamma_g2, delta_g2]);

        Ok(bn254.pairing_check(g1_points, g2_points))
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

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
    use soroban_sdk::{testutils::Address as _, Env};

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

    const SHIELD_VK_HEX: &str = "004a10ae973df76d18cc7282cb3fd87d293ac1521d0f8c3fe1b7a8bf2cc1cd3d1c1a9a34341a0bbae006c778fbfdf7e5d0ae8cac871ecf71f17f4673e5b1f56b062d17924ef71132b66fc4695c553433e7cede3848a8bda9332958709c984a0b13bc9e495aa7512ed247262baefd60f73226f7017843977bc797462aa88034970cdc3f64b84088c7343b736148da94beb5b9ed7c19d2397c25ecb783132af1292bcac674e74ffd994b4152e1347afe87ea763ffde54a274b6e0f5653ab94de91198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa2df369fb1a80fe5f43ce5815c9b3d0fef629d854609fde7a59490390c92214641c4a9e5de0ce34f02e9e7bd8fcb0a020f9b772727549769a13c1aa2a896c988e1fd2e8d20f42512629dbe9cd409682377127537e0b522bd586149822ca640a08124ccc6c94f84e623f6edd2ac87bc88df3c41d3abc05ffa297a1ef152accd75d15daa321aa936187619b84941b53be84c1b578522c21fdd325098db19d1eb3391b0f6021904d94598ee99139ffd5b19c192eed86af5ec5c3a119ca19688429c022ab8b7c9d621b1d4e9de842f1038c76ce2e5a5bed63961bc8bc7b19d386ccc21ed47d27a78edf33611e9bcdae16333a0af3f12236105be8f3e047d8b7081fa4114750d6ecf3c05dafe55c30c1814760e2bc9287e1734a74963c90fdf19afac81b4bb078cf97e0bc1d0498b93ad71ef3f41ec1a75fc509d8ab59e632b6a088c01c30e0a53f367cf515da53417ad5caff8e52d85d56e90129b0d2fa8ea086c4140f5e194b1fd8c120f112207e71426ac80447337645a162e1e1b116f746ea9b202fa04c7145c3b274229a2129bbbb3d165372d2fd876e66167286b430198317f310c240e4e7097770adf18390dccd4d934b42cf5ad47d390b6766b3ac5f522cf7";

    const SHIELD_PROOF_HEX: &str = "2d365709202e339030899a22e64988fdd93c5a823670909c9bb41d41c02368462ac8cb01984d8cdd479e9c5c41056a7985cda9052c212a48593b67ea6b840b9e2dabb21e521c292638d562b866299c4b4a937d7fc80d7a9bd2dfa7a187254b8315339d9fe5526791e67c4c060f5fefba0fb8d9de64a3014480b4402ec080bac82f7edf085a1d23dd4b543f3862511d7af632a41f4b50574adc9ee0350766a94517febfff04a6c1ab2e11d244db45d52197c75de34a0ef73532574df1a75552b70fdd0f0ac749688a753e65c92f5c3e896a58a855e61b932e121163dd1fc18920115b00d67cac672cce5de198706f7577a1ade5a6b90a1989ae2b4519bc1afa87";

    // Order: commitment, value_commit, pub_value, pub_asset_id (matches
    // shield.circom's `component main {public [...]}` list and
    // ShieldPublicInputs in ct20/src/types.rs), little-endian 32 bytes each.
    const SHIELD_PUBLIC_INPUTS_LE_HEX: [&str; 4] = [
        "34e0b1164d8115f16361db88db58197334127310d50ed897e3ca979f403b302c",
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
    // synthetic proof (covered separately in ct20's own test suite), but
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

    const TRANSFER2X2_VK_HEX: &str = "0b291fdaaa28add7553e94df40614c894ca8fb22a2b6b4ed7351d325cad7068e1242afa10511b208e98200b835350f44a0b2641bf06744f87f3960b79f6122880041e3d1d3043bbf9687e1c198b5fe1f3f597c26b7a97127b33b64938c49887e0c65ed7e66ecf358b07f11fc7eb9cb3ecb88ec0dcfb88c12938f1ef0fa330e601c122704e90921beaa1548ea5efcd702fa0689a866360fd874cec4d1f0507f7430573487cca5aaa0c8a3417a831694d86b1171e39d821f5d456f9be5a2d7457f198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa1a022a8c4c2a87d64880aa9e3d089ff748c93703db1e60adaa506dac3b219ae8103ddf891d57fbfa6f0001e2fe6447bd623fa47da6800e781270d715263ea63b10bb43b6b3289dfba438d67552dcffdc17407156ad92dd8e217a74996480492721a6abc90e29003b333699172bd49ae364719dd2b91d470801752fc3c11747ba2166be7231e969fbc3be058f12e91c077a02d8a63f570a7bbf366887ee86a15901ee4fa8c3211e7bf0825e52dc01257b072e290653aa45b5fba100ae4efb4c7a042cc4a4c3313a6d2c30f8530ae70c08488b2e3c06eb5e22044d80c9bd318683089fe41e4a63cfff931618126296f878fbb8e7d02e867f56fb75e0393c05c0ab167cf0e019935756ebbe9331dc785c802f9df453077b96e70a87b1966692443e188c27fb20669a43394269ab05fd77d890855a02bb5a5096431477e25dca5b94125a0224b1a22cdc6f2f24e2a22639b475ae7677e67c10897e798b3477949238145580e95cbb6abe2f70cec376448769b6e656696ca042ac6c3ac5270926062b06d734251c7b7458de1a6b7bdc7dd9ea8ac1c47c11eae2ea376707d0afbed38208f7da7654670501dfb0fef4c430e03977d4583da19541cdb4ed42690ad860a017cdc4f5e8589bca329b168a0d590a5fc28b23e680bd71778aa36726ee3dd77a17e139ea1ca09c0d06a71d0f50199c2066ea11aafcb259ffb01b5fa91786fadf116b0f1cde75d7ee410d995c7c96e1b6575cb6a7e1f6a96266eddb6b9cc497e81a7971dd4e2fae5a68dc090f5506f31ed65c9c1805111e9072b9ec940759e9921245d0a7cee5d344d037cbf9c349e7f595099e60114138878869ebcaa7a3074f2e7cef9f88255989981defc1e21e00d5ef484115604cadc02a9adbe727ae433228921fad056b74bd862d86e834b1f10c1548158f125d05e316b81ca364480d6d0e0b598e9cc80c07bf3f867a413e103ad6361f0bce7f818bc6a09e4cca81ff13022955b6f1b3d444d2bf2a25f9ebf4c78aebb4fcb2795764e62d70eae6002947145252dacd18a1751baa375345a267c951e0602f4a81c98400cc0f66510d47521380a2da41a6226935aef250526140b4354c0df229ad2903f94643d34270ead6175bc3fb9d2a30be19c1b58ef43200184f7379eb8ce66016e29ca8c63f78b56717e864590d2ae0f68a80d944b46647e3c775511daa928a5f5763c880ac5bd8b9243a50209770f518fed800886354d7cf63a54f87b2e88c6c579a87e43edbca09";

    const TRANSFER2X2_PROOF_HEX: &str = "300877e872a23340f1f0d75082fbbd500ae454a024ad4eb99133de3e564f6343145e0084b6bc59d67a38c333192b49c448e8f068dd1251605deadf29d41a151224fda90eb369051ebd0e52be53140f00013ec7290801c82dae18efaf8039c23d13972c83a3ea0d955765316e9e4c9c3791bd76b723c488db01328cdd3bf3c71f2a0349affd23aac02df48f95731424cd679c3cc76ba4c13e7dd038a04fa8b68a26cac1cb0d791894e73a7ef87a52a6166b666ac60f8aa4c7694a48abc7a23cb80a5343954637267d98cb3a07641407a2e0140a86d183b38e268e1f8379d870d417cbc698a329d9dc33d28717da2cb3e004c287a2d4c5b52c63b28990b53444e5";

    // Order: anchor, nullifiers[0..2], out_commitments[0..2],
    // in_value_commits[0..2], out_value_commits[0..2], fee, asset_id
    // (matches transfer_2in2out/transfer.circom's public signal list),
    // little-endian 32 bytes each.
    const TRANSFER2X2_PUBLIC_INPUTS_LE_HEX: [&str; 11] = [
        "b6977f2be1ff8fdfc0cf61be1c876be6433eb0f18610103e17b24104d7713d30", // ANCHOR
        "4d8868c8b074d176055d827cfd335b5e2f811a060938c6ed26ed191ccb80e207", // NF0
        "3836e7d2d0c9902fe1a488b0b4cfe190b08f347144d1df7ec978ac8259b38f06", // NF1
        "49030caed2d8b0e5a610150752867b60844c3e7bb3b147391cc1316d65e33327", // OUTCM0
        "768b135894509989d244f21ff91e7d554c0a778434f6ede80fa67f8df38fe12a", // OUTCM1
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
    // exercised against ct20's synthetic arkworks-based test proofs (see
    // `contracts/ct20/src/test_groth16.rs`), which prove the *contract*
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

    const TRANSFER4X4_VK_HEX: &str = "0b291fdaaa28add7553e94df40614c894ca8fb22a2b6b4ed7351d325cad7068e1242afa10511b208e98200b835350f44a0b2641bf06744f87f3960b79f6122880041e3d1d3043bbf9687e1c198b5fe1f3f597c26b7a97127b33b64938c49887e0c65ed7e66ecf358b07f11fc7eb9cb3ecb88ec0dcfb88c12938f1ef0fa330e601c122704e90921beaa1548ea5efcd702fa0689a866360fd874cec4d1f0507f7430573487cca5aaa0c8a3417a831694d86b1171e39d821f5d456f9be5a2d7457f198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa137999d40c16d37bf27ee0d36e82922f18e46a610d13ca839051b4440086bfd209c4a649c167208b06511e56d2187e8c9b8cfae13a556dac40516d44568dce8904cf3281661110ca8d93210795a100a25611338bec8111a03136d96b208b3d9d1826fc82326da7dd3023c9b662e8989adac75b0509802107f0b04631bb3351c726cd1ceca944058c98d289f619e72ba5d34be160dc5d5026d488f2415cfd3df62e890cc23496756b71c69cea80802757bc8a1893951c3ca858ff68eaa07cb6862870531ad44f46197ed77a641762adf0471c4bbcafbfbbb784911e6be2f7e1520fc61efc9db6f4762f13177e5d463f194d80a85028f7f6e0fd286ad32a5a376b0e65557004d448dea6e2d0ec49660233f5374ddeb88b32398fd980a04f4832521910eedecb7132da7d9bd25b691e5df1db30017e272c9d9e462ffb76707030d9172103beed7dc7f4973dcb91b9e00fbe97555de2fdf45b0d9340c7fb7c482439291ed8f92fb8b8fd27cd06fd6456b2c6bde910cf4f1244cc0576024d5bfe906c073495b5e4cbdfeaa0252e855cfe966b2beaee299d99529beb9a444c914b59ac1081addb836222970db90fa723c1a592e7a681b030586ae7f9e2a1d5f4bde6e01073fcb83dda2bb8c7cf5e823710e6693382881a3b8d109c3f351cd802118c2828fd2bf3342955823f3fe7912ed45d76fb589ae04de0b98367e4c9e582a9435f0570584d613651f08746f543b30a9084d80e1c304659e5008d0c4d6cebeac9802630d5733bc6a7c8710eb0b4d83ca892daed6144166d1ad4e6b7527374151541178b9b091c2dcd8a25dbc1000f375093bc1765dfb99b168facdd0a5ad7be7aba26ef48f9ac7968a57f4a9fdafa64bbb777cbec8d70eae056fbe7a09f0e72f1d01793d64076f5ede38f87d6edaf9239c1fd3f4ffa546ee23ddfde9f836810d28517ab0ae906e8d6a92d4cceea6f5c5dd528d809b5a0d02bfbbc560f1f7a53c4b5099398c7f8ebcfbb06145f1bc074574693e02aa228f2499cecb2194f69477eb92d29da218088dd95d6cb2743fed1d1c2542b90b03eb5edbf9b1e0c830a9cf0ba29cf8aa0fa12dfa791ebd87778c9a06a555575a9f2c7d600467730eefc250ff92c14399ecd67aec0a2d8811f86468fa9efdb0d8ddd88a901688c18f5d1058e0802b8a6b89e186580b06b5515b4deac9aae11012e1b5493b1a84008ccec367c831c55170e4e7ef2bd12c76d2630c4be1c0dfb90d4bdade8017b40dc5ac61f7b580cd3923499482013df6613f1c01e88b67618a0b1175f4e835bbae3eaf12b8ec718f089cdf3fb290903ea7601b7c57ebca87859db5accf3dc830135f9aa719f0606ca7f25ece6b5335dcd8e463310d8b2e27ba3786eb3b9d56dafcda88602b3c31e2f2ec864503e3a1cdc3ebebc554b2bf537f2eeb84efa4952daba522f61e2f826ff6f9e0985d18e2122c56010daf38cac09b1a3aaff888361b7418fcc55f1071976ae99a979f039ef594bf81a409b8aa6a41721d4ea2ef11f9ecdfb348105b02ed8161cc6397d6c157bc4cd9bb684b95648c9dbc260f2837819f5aeaf73464229e7b6f94b414da3291466883261d653947f78a9aaaa5c91a57b6db60ebdb41c20ede9c1034947705b23c6e403d3d8bf4cbc9f61bd1a0fde76dd62e35815dbec2dc28b9c22f4b69d66784fd809068d05db94f5b089e8f96c277646fbc9a13b601864ce1363310be9959bf291adf8e526254308139d2991b524a55ecd612df280191d73187b27350e62a443bcc2a8643609d0faa4628058f54e4a33288924c1221c56a375fda4398ed0201c946204e32bdc24a817e5ce152988a6f21bbcb5947117539478c00d722c61f8fc6e3a99efbda73041162101928664cbf84308f0de08076e2cce3ee1c81bd9daa32a47d74f0014a760a0bb94c270fe3308e2b235adf22dbe779b2644371de358e4745dc233ab6b1a21949050a727e6a93a1032b051bc";

    const TRANSFER4X4_PROOF_HEX: &str = "28d25daf2b5672a2534a76687af1328a9363af1ddbae196b488d63673d37da0d14d7c933ab99e73b4083ad5e407677523093d4697fc42f1c6bcd701970ea443d0529a96382d9932382723886a4c0575a8784eb1c09ba8ff963fce46132e3d70211df582df9f62324d3b6b8b2629ddbd71918d15effb32f9598d69febd4ef391224c05cd7739924a37908ac609b8f6aa9d7795f126f3e2684e0443a50bf1a09e30cd1d2b6e70985af2fe20998cb30b8a8c3a73bcd83eac0d3ffaa15daab809d142efb876d56bbebe25986d468496c1a7fec71b2109ce2889ac91f77fde98c9e010c0b7f6c1b1aa1442d7920694e751c8556e5ff344293fb360e61b6dd7db605c9";

    // Order: anchor, nullifiers[0..4], out_commitments[0..4],
    // in_value_commits[0..4], out_value_commits[0..4], fee, asset_id
    // (matches transfer_4in4out/transfer.circom's public signal list),
    // little-endian 32 bytes each.
    const TRANSFER4X4_PUBLIC_INPUTS_LE_HEX: [&str; 19] = [
        "c789c180a20104a61db818b1e52fbe8eb70cd26c09c56aa754d87f8e546cfa2a", // ANCHOR
        "4d8868c8b074d176055d827cfd335b5e2f811a060938c6ed26ed191ccb80e207", // NF0
        "3836e7d2d0c9902fe1a488b0b4cfe190b08f347144d1df7ec978ac8259b38f06", // NF1
        "6bb2bcb4fde4ab5052e1b0ef4e1e35132fd29f01252b5d475c579d1d53c1642e", // NF2
        "448ccf7ea6b39e6749a56b7426e306fcd781174bff9bb91388d8d6847d30c216", // NF3
        "254f6b7c63513ab10c6d86e721f17551a3c4a02fd1de04caf28d72f1ae333e16", // OUTCM0
        "af96bde9ca2e081277467225c6d5481aa0b2f0408f41e425efb602444aa06115", // OUTCM1
        "aff082f9c974da7f3eb18365bdc48556ddddde53c67d97b207864f7d4d5ccc29", // OUTCM2
        "21deb6e52c070711d8507b8203c377abce72be3dce3a3ff245ac9df1c8268223", // OUTCM3
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

    const SWAP_FAIRNESS_VK_HEX: &str = "004a10ae973df76d18cc7282cb3fd87d293ac1521d0f8c3fe1b7a8bf2cc1cd3d1c1a9a34341a0bbae006c778fbfdf7e5d0ae8cac871ecf71f17f4673e5b1f56b062d17924ef71132b66fc4695c553433e7cede3848a8bda9332958709c984a0b13bc9e495aa7512ed247262baefd60f73226f7017843977bc797462aa88034970cdc3f64b84088c7343b736148da94beb5b9ed7c19d2397c25ecb783132af1292bcac674e74ffd994b4152e1347afe87ea763ffde54a274b6e0f5653ab94de91198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa0095dfc079f39d999750afaa0abb0bbd7f7c8e6b640862571e186bb2dc70fb711ceac25c7003d2c5dffbf3621fb70851e93a21b3d1451be1ba40a6f2b540caf401aaf3049ad3b2e09e3c5f8ba1c395c152a771d22dded9824788ba8f4e789f0c1469c7772f4cacfdd349b742dd08b82c4ff61fe79db0ef62d430c4747e72acad232b16a9a87ef833af929b141d653f4cf75e25a7acc5f82038920775da5b25c107c54c3b4de79e24e381fecd73697500ecc515bde9087a646ea587148da9be4c1c106506a3716278e01b6ea3418b28fd0e72d2e2c2afca42b56dd35b7cb7dc54064d201429d2f82924fce8c9dfab7609c6b2c25e76db6e34ba8df791157b9c620f2e99b93253009a2c454e105e56a025235ac0906592d09f4494bebbf506e2512f58062de8f9489e21478636db5ebc7067dcbaac136a7d66e1dfdce2ff21e7070cbd822e73618e200c91f77f45ae84b169f2ac0dabc9bd579a14fc99b2a8da1c0bf22a4f02ec50dd0d4e0ac4135b5246f0dd608487d3c0f32f06f3b8b12dbaf30220f749ecd7fe1cfeedd2b5d365334090f4f09e9fe6226b5831bb1444b197cd10523a59f11bb13034e04b4b33bb3fe2b718846090548734806ee1567e75612c12b7c0b32943465d1237ce017a3c9748840514c534f3d27660f7d9d6311c7b5027aee02d57eecd73de269e22ba815e80a543391b3e08cd85fc28ec6a86d04af9";

    const SWAP_FAIRNESS_PROOF_HEX: &str = "14e956c61d8540d19a1f4d60dc7ddd1cfdfbdd7ebe1288ecc43b9cc4f61867962617371959044e8559791906318b14eb501a1bffcea8bb7b87a1c4f72b0329681ec8e9cbef298757f2e9bc8c42eb5f576c690a9bd20942a56247214a7acda6c80baa7a5adefb470793ed9ebc4de60957d23f5c2716cb724b653002322df5f9991b226f859953c59272d8698afc0e6942de54080264d99d2ba66e12d0f3e3b3d8127e218acb5257d35f327190ece18c5937a51815ef9ae31d4ac1942e1378eb6f0581d26bc95f7749b85c49cb28b5261cd119050527f4693147ef487fa5baacef18237bc2c24e52d2890ccd7a1629c8375df993f1c1f38529a87ccd9e4ab262e7";

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

    const UNSHIELD_VK_HEX: &str = "0b291fdaaa28add7553e94df40614c894ca8fb22a2b6b4ed7351d325cad7068e1242afa10511b208e98200b835350f44a0b2641bf06744f87f3960b79f6122880041e3d1d3043bbf9687e1c198b5fe1f3f597c26b7a97127b33b64938c49887e0c65ed7e66ecf358b07f11fc7eb9cb3ecb88ec0dcfb88c12938f1ef0fa330e601c122704e90921beaa1548ea5efcd702fa0689a866360fd874cec4d1f0507f7430573487cca5aaa0c8a3417a831694d86b1171e39d821f5d456f9be5a2d7457f198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa2d9cca5351120b63aa991da6bec4b807d543867c2ebc51da8ee2eba181a0d478040e060e1fedac5c6567b436890917c9f21093c0c6db108e446371a6dab2d4a419d2a654617d5bfb4f9aa088c7cfc33780d00aba13dc5e59af91ace654fcab320defa3acf67254c86dc6627f1a28a1624aa685129c26e697f3b1368c146998921488716f57136a9c6e0aea900c443e8bf4851a3b4a9de095b2bbe6ee45a0928c04fcc15f28278f536deca9a58745100a454426ad38a1ef86532dbc9150ab3c5721d875b836271817bf90fa35fc178f4e34a755dc95fb801df7917ac67871669609b798ca5c6e03e3be8e3f39305f451a4e7b6605aab3718b26a468506657b21a1fe02728ff258d1f58b1c8dec33ba170a5f4ac4adbf50ad72b4378cb8e17ee000d48c9210286aed1e336f8d33b25f02c1c91ef2681c8d70cbab0dd69aa2e0cfb1ffba34d56dd06b0a47830cf0d5687b18771651409aeed5b3530ad124ac656891fd63eef08b7d24f442835884a08a6db7d5f8cc4336a06c4b658e35b7e698f070aa3095674f8d47ff59423e7563055bff31554bab4e1a4c31fbb4ad433ba34fc1ce0f8189093bcab3693812d9c92cae91cafb077622e004b81d46e784de4bac02416127a41885c66ff7d8a98af5b52042b2f84d8883ee2593a5195a376ec2600241347b2dcb3c525784408da8d2c6012ac0ab456c68f0c5efd25e30edfcf0d3c";

    const UNSHIELD_PROOF_HEX: &str = "0b98d2e8e4d3ed5c236deff9434f5e8e51523806efd3c31969ed2342d1a2a11a17599b42085920008a36a1d36756abcb3feca74838284e4b318eb94fbb7039071310a9439c13f4fbfdc28ebc72bf32e1b09bed4a2c6186ff2f264a125e219abf2cc93c6ca91dd49a7e21d86c4696d1af62ddf9ba6ea491fdeb20beaca93f323121274fc006e90d0fa05c3e004f877fc4b71ca7773382a31db6b161fbfc6a20f62b6b68e9eed20e906f286a99c2482999056377cb79b9a12260473eb7d62fed070aa19630eed42756d60b1c3c6dd6b3e36a19ffd1cd9d57ba211c0359136add371d0491473b8176fb4b78368cd9f8854d60c5dbdc7819d168527b1f95be4390f4";

    // Order: anchor, nullifier, pub_value, pub_asset_id, recipient_hash
    // (matches unshield/unshield.circom's public signal list),
    // little-endian 32 bytes each.
    const UNSHIELD_PUBLIC_INPUTS_LE_HEX: [&str; 5] = [
        "17618668a8b4ec213f956a90c17f10a496fba87093233bcb4db389ee756b1409", // ANCHOR
        "bb59c47fd18dfa22d7e51c1e2dafc346b60b6a60e39639e15ceb43f3bbe90609", // NULLIFIER
        "90d0030000000000000000000000000000000000000000000000000000000000", // PUBVALUE
        "cd81010000000000000000000000000000000000000000000000000000000000", // PUBASSETID
        "2a00000000000000000000000000000000000000000000000000000000000000", // RECIPIENTHASH
    ];
}
