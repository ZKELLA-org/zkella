#![no_std]

//! Cross-contract interface for `contracts/verifier`, with no `#[contract]`/
//! `#[contractimpl]` of its own.
//!
//! Contracts that need to *call* the verifier (`ct20`, `governance`,
//! `compliance`) should depend on this crate, not on `zkella-verifier`
//! directly. Depending on `zkella-verifier` directly pulls its actual
//! `#[contractimpl]` — including its WASM export directives for `initialize`,
//! `verify`, etc. — into the *caller's* compilation graph. Because Soroban
//! contract exports are unconditional (not subject to normal dead-code
//! elimination — they're kept alive specifically because they're marked
//! externally visible), a caller that also happens to export a function with
//! the same name (e.g. its own `initialize`) hits a WASM linker error:
//! `duplicate symbol: initialize`. This was found empirically: `compliance`
//! hit it depending on `zkella-verifier` directly, while `ct20` and
//! `governance` happened not to under the codegen-unit partitioning in place
//! at the time — a difference that is not something to build on, since nothing
//! guarantees it survives a toolchain update or unrelated code changes. A
//! plain interface crate (only `#[contracttype]`/`#[contractclient]`, no
//! `#[contract]`) can never trigger this, deterministically, because it has no
//! WASM exports of its own to collide with anything.
//!
//! `zkella-verifier` itself depends on this crate for [`CircuitType`] and
//! [`Error`] rather than defining its own, so there is exactly one definition
//! of each — not two independently-maintained copies that could drift out of
//! XDR-compatible sync with each other.

use soroban_sdk::{contractclient, contracterror, contracttype, Bytes, BytesN, Env, Vec};

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CircuitType {
    Shield = 0,
    Transfer = 1,
    Unshield = 2,
    NonMembership = 3,
    /// `circuits/transfer_4in4out/transfer.circom` — same public-input shape
    /// as `Transfer` (anchor, nullifiers, out_commitments, in_value_commits,
    /// out_value_commits, fee, asset_id), just 4 slots instead of 2.
    Transfer4x4 = 4,
    /// `circuits/swap/swap_fairness.circom` — binds a revealed (amount_out,
    /// min_amount_out) pair to the swap's original `intent_commitment`
    /// without revealing amount_in/max_slippage_bps/intent_nonce. Covers only
    /// `contracts/swap::reveal_and_claim`'s fairness check; `commit_swap`'s
    /// nullifier-ownership proof has no circuit yet (see that contract's doc
    /// comment).
    SwapFairness = 5,
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

/// Mirrors `zkella_verifier::VerifierContract`'s public interface exactly.
/// Generates `VerifierClient` — a lightweight cross-contract caller that
/// never links the implementing contract's code.
#[contractclient(name = "VerifierClient")]
pub trait Verifier {
    fn register_verifying_key(e: Env, circuit: CircuitType, vk: Bytes) -> Result<(), Error>;
    fn update_verifying_key(e: Env, circuit: CircuitType, new_vk: Bytes) -> Result<(), Error>;
    fn get_verifying_key(e: Env, circuit: CircuitType) -> Result<Bytes, Error>;
    fn verify(e: Env, circuit: CircuitType, public_inputs: Vec<BytesN<32>>, proof: Bytes) -> Result<bool, Error>;
}
