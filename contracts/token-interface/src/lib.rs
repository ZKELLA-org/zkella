#![no_std]

//! Cross-contract interface for `contracts/token`, with no `#[contract]`/
//! `#[contractimpl]` of its own — mirrors `contracts/verifier-interface`'s
//! pattern and exists for the identical reason documented there: depending
//! on `zkella-token` directly would pull its actual `#[contractimpl]`
//! (`initialize`, `shield`, `unshield`, ...) into the caller's compilation
//! graph, and Soroban contract exports are unconditional — a caller that
//! also exports a function with the same name (e.g. `contracts/swap`'s own
//! `initialize`) hits a WASM linker error: `duplicate symbol: initialize`.
//!
//! `contracts/swap` depends on this crate to cross-call `token::unshield`
//! (pulling a spent note's public value into escrow at `commit_swap` time,
//! reusing the already-real, already-audited unshield proof path as the
//! swap's note-ownership proof) and `token::shield` (re-shielding the
//! escrowed output as a new note at claim time).
//!
//! `ShieldPublicInputs`/`UnshieldPublicInputs`/`Error` mirror
//! `contracts/token/src/types.rs`'s definitions field-for-field / variant-
//! for-variant (same `#[repr]`, same discriminants) rather than being
//! re-exported from there, for the same reason `zkella-verifier` now
//! defines `CircuitType`/`Error` locally instead of re-exporting them from
//! `verifier-interface`: empirically, a type's `contractspecv0` metadata
//! only reliably survives WASM linking when it's defined in the same crate
//! that exports functions using it. These are wire-compatible with token's
//! own copies (XDR encodes structs/enums by field order and discriminant,
//! not Rust nominal identity), so cross-contract calls decode correctly on
//! both sides regardless.

use soroban_sdk::{contractclient, contracterror, contracttype, Address, Bytes, BytesN, Env};

#[contracttype]
#[derive(Clone)]
pub struct ShieldPublicInputs {
    pub commitment:   BytesN<32>,
    pub value_commit: BytesN<32>,
    pub pub_value:    i128,
    pub pub_asset_id: Address,
}

#[contracttype]
#[derive(Clone)]
pub struct UnshieldPublicInputs {
    pub anchor:         BytesN<32>,
    pub nullifier:      BytesN<32>,
    pub pub_value:      i128,
    pub pub_asset_id:   Address,
    pub recipient_hash: BytesN<32>,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized   = 1,
    NotInitialized        = 2,
    Paused                = 3,
    InvalidProof          = 4,
    InvalidAnchor         = 5,
    NullifierSpent        = 6,
    CommitmentMismatch    = 7,
    AssetMismatch         = 8,
    AmountMismatch        = 9,
    Unauthorized          = 10,
    MerkleTreeFull        = 11,
    NotImplemented        = 12,
    InvalidNote           = 13,
    DuplicateCommitment   = 14,
    InvalidInputCount     = 15,
    RecipientMismatch     = 16,
    DuplicateInputInCall  = 17,
}

/// Mirrors `zkella_token::ShieldedToken`'s public interface (the subset
/// `contracts/swap` needs). Generates `TokenClient` — a lightweight
/// cross-contract caller that never links the implementing contract's code.
#[contractclient(name = "TokenClient")]
pub trait Token {
    #[allow(clippy::too_many_arguments)]
    fn shield(
        env:            Env,
        from:           Address,
        asset:          Address,
        amount:         i128,
        rho:            BytesN<32>,
        rcm:            BytesN<32>,
        commitment:     BytesN<32>,
        encrypted_note: Bytes,
        shield_proof:   Bytes,
        shield_pub:     ShieldPublicInputs,
    ) -> Result<u32, Error>;

    fn unshield(
        env:         Env,
        nullifier:   BytesN<32>,
        to:          Address,
        binding_tag: BytesN<32>,
        proof:       Bytes,
        pub_inputs:  UnshieldPublicInputs,
    ) -> Result<(), Error>;
}
