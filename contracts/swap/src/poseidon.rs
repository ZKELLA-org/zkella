//! Minimal Poseidon2 hashing for `swap`'s own use: computing
//! `recipient_hash = Poseidon2(address_field(this_contract), 0)` to match
//! what `ct20::unshield` independently computes for its `RecipientMismatch`
//! check when `to = this_contract`'s address.
//!
//! Copied verbatim from `contracts/ct20/src/poseidon.rs`'s `Fr`
//! (canonicalization) and `Poseidon2Hasher` (native-host-backed hashing) —
//! not re-derived — because any deviation in the field-reduction algorithm
//! would silently produce a *different* hash than ct20's own, breaking the
//! recipient-hash check with no useful error message. Not extracted into a
//! shared library crate (a cleaner option, deferred rather than skipped)
//! because `contracts/ct20` doesn't currently expose `poseidon` outside its
//! own crate, and reorganizing that heavily-tested module was judged higher
//! risk than a small, precise duplication for this specific need.

// BN254 scalar field prime (little-endian u64 limbs)
const R: [u64; 4] = [
    0x43e1f593f0000001,
    0x2833e84879b97091,
    0xb85045b68181585d,
    0x30644e72e131a029,
];

#[derive(Clone, Copy, PartialEq, Debug)]
struct Fr([u64; 4]);

impl Fr {
    fn from_bytes(bytes: &[u8; 32]) -> Fr {
        let mut limbs = [0u64; 4];
        for i in 0..4 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&bytes[i * 8..(i + 1) * 8]);
            limbs[i] = u64::from_le_bytes(b);
        }
        // External input may be anywhere in [0, 2^256); reduce fully.
        let mut f = Fr(limbs);
        while f.geq_r() { f = f.sub_r(); }
        f
    }

    fn to_bytes(self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for i in 0..4 {
            out[i * 8..(i + 1) * 8].copy_from_slice(&self.0[i].to_le_bytes());
        }
        out
    }

    fn geq_r(self) -> bool {
        for i in (0..4).rev() {
            if self.0[i] > R[i] { return true; }
            if self.0[i] < R[i] { return false; }
        }
        true
    }

    fn sub_r(self) -> Fr {
        let mut borrow: i128 = 0;
        let mut limbs = [0u64; 4];
        for i in 0..4 {
            let diff = self.0[i] as i128 - R[i] as i128 - borrow;
            if diff < 0 { limbs[i] = (diff + (1i128 << 64)) as u64; borrow = 1; }
            else { limbs[i] = diff as u64; borrow = 0; }
        }
        Fr(limbs)
    }
}

pub struct Poseidon2Hasher<'a> {
    env: &'a soroban_sdk::Env,
    sponge: soroban_poseidon::PoseidonSponge<3, soroban_sdk::crypto::bn254::Bn254Fr>,
}

impl<'a> Poseidon2Hasher<'a> {
    pub fn new(env: &'a soroban_sdk::Env) -> Self {
        Self {
            env,
            sponge: soroban_poseidon::PoseidonSponge::new(env),
        }
    }

    /// Hash two 32-byte little-endian values, returning 32 little-endian bytes.
    pub fn hash(&mut self, a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
        use soroban_sdk::{Bytes, Vec, U256};

        let mut a_be = Fr::from_bytes(a).to_bytes();
        a_be.reverse();
        let mut b_be = Fr::from_bytes(b).to_bytes();
        b_be.reverse();

        let a_u256 = U256::from_be_bytes(self.env, &Bytes::from_array(self.env, &a_be));
        let b_u256 = U256::from_be_bytes(self.env, &Bytes::from_array(self.env, &b_be));

        let inputs = Vec::from_array(self.env, [a_u256, b_u256]);
        let out_u256 = self.sponge.compute_hash(&inputs);

        let out_be_bytes: Bytes = out_u256.to_be_bytes();
        let mut out: [u8; 32] = out_be_bytes.try_into().expect("poseidon output is 32 bytes");
        out.reverse();
        out
    }
}
