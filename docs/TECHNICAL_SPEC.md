# ZKELLA Protocol — Technical Specification

**Version:** 0.1.0  
**Status:** Draft  
**Network:** Stellar Soroban (Protocol 25+)

**Implementation maturity:** this specification describes the target ZKELLA protocol. The current repository contains only a soft PoC implementation foundation. Existing contracts and SDK code are not final versions and must be reviewed, profiled, hardened, and improved before they are considered production-ready.

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [Cryptographic Primitives](#2-cryptographic-primitives)
3. [Data Structures](#3-data-structures)
4. [Key Management](#4-key-management)
5. [Circuit Specifications](#5-circuit-specifications)
6. [Smart Contract Interfaces](#6-smart-contract-interfaces)
7. [Protocol Flows](#7-protocol-flows)
8. [Persistent State Manager](#8-persistent-state-manager)
9. [Shielded Swap Primitive](#9-shielded-swap-primitive)
10. [Viewing Key and Compliance Layer](#10-viewing-key-and-compliance-layer)
11. [Developer SDK](#11-developer-sdk)
12. [Security Analysis](#12-security-analysis)
13. [Performance and Resource Budget](#13-performance-and-resource-budget)
14. [Deployment Plan](#14-deployment-plan)

---

## 1. Introduction

### 1.1 Scope

This document specifies the cryptographic protocols, circuit designs, Soroban contract interfaces, state management architecture, and SDK APIs for the ZKELLA Protocol. It is intended for implementors, auditors, and integrators.

The interfaces and code references below should be read as target design plus current implementation anchors. They do not imply that the existing contracts are final. Current code is useful for early validation and review, but the remaining roadmap must complete proof verification, missing flows, resource optimization, and security hardening.

### 1.2 Design Goals

| Goal | Mechanism |
|---|---|
| Balance confidentiality | Pedersen commitments over BN254 |
| Transfer validity without disclosure | Groth16 zk-SNARKs verified on Soroban |
| Compliance-ready selective disclosure | Hierarchical viewing keys |
| Double-spend prevention | On-chain nullifier set |
| Multi-asset support | Asset ID bound into note commitment |
| Liveness beyond 7-day RPC window | Decentralized note indexer |
| Composable with Stellar DEX | Commit-reveal shielded swap |

### 1.3 Non-Goals

- Full privacy of execution logic (not a private smart contract VM)
- Hiding the transaction graph at the network layer (not Tor-equivalent)
- Mandatory privacy (opt-in shielding only)
- Protocol-level changes to Stellar core

### 1.4 Notation

- `||` — byte concatenation
- `H(x)` — Poseidon2 hash (Soroban native)
- `cm` — note commitment
- `nf` — nullifier
- `sk` — spending key
- `vk` — viewing key
- `nk` — nullifier key
- `G` — BN254 G1 generator point
- `F_p` — BN254 scalar field (`p = 21888242871839275222246405745257275088548364400416034343698204186575808495617`)
- `π` — Groth16 proof `(A ∈ G1, B ∈ G2, C ∈ G1)`

---

## 2. Cryptographic Primitives

### 2.1 Elliptic Curve — BN254

BN254 (alt_bn128) is available as native Soroban host functions since Protocol 25.

```
p = 21888242871839275222246405745257275088548364400416034343698204186575808495617
r = 21888242871839275222246405745257275088696311157297823662689037894645226208583
G1 generator: (1, 2)
G2 generator: defined over F_p²
```

Host functions consumed per operation:

| Operation | Soroban Host Function | Approximate Cost |
|---|---|---|
| G1 point addition | `bn254_g1_add` | Low |
| G1 scalar multiplication | `bn254_g1_mul` | Medium |
| Multi-pairing check | `bn254_multi_pairing_check` | High (but native, not WASM) |

### 2.2 Hash Function — Poseidon2

Poseidon2 over BN254 scalar field, width-3 (2 inputs + 1 capacity element), 8 full rounds + 56 partial rounds.

```
H_pos2 : F_p × F_p → F_p
```

Used for: note commitments, nullifiers, Merkle tree internal nodes, key derivation.

Poseidon2 is available as a native Soroban host function since Protocol 25. Do not substitute SHA-256 inside ZK circuits — the constraint cost is ~20,000× higher.

### 2.3 Pedersen Commitment

Value commitment for range proof binding:

```
cv = rcv * G + value * H_v
```

Where:
- `G`, `H_v` — independent BN254 G1 generators (nothing-up-my-sleeve points from hash-to-curve)
- `rcv` — random blinding factor ∈ F_p
- `value` — token amount ∈ [0, 2^64)

Homomorphic property: `cv_in1 + cv_in2 = cv_out1 + cv_out2` (value conservation verifiable without revealing values).

**Current implementation note:** the circuits and contracts as implemented today (`circuits/common/value_commit.circom`'s `ValueCommit` template, `sdk/src/notes/builder.ts`'s `computeValueCommit`) bind `value` and `rcv` via a Poseidon2 hash (`cv = Poseidon2(value, rcv)`), not the real BN254 G1 scalar-multiplication-and-add construction described above. Balance conservation itself is still soundly enforced — the transfer circuit directly constrains `Σ in_value === Σ out_value + fee` over the private `value` signals inside the R1CS, which doesn't depend on the commitment scheme — but the homomorphic *external* check (`cv_in1 + cv_in2 = cv_out1 + cv_out2`, verifiable by a third party without a proof) is not available with a Poseidon-based `value_commit`, since Poseidon isn't additively homomorphic. Real Pedersen-over-G1 arithmetic is a real, open simplification to close (see `docs/CIRCUIT_SPEC.md`'s note on the same point), not a documentation lag.

### 2.4 Note Encryption

Symmetric encryption of note plaintext for transmission to recipient.

**Key agreement:** ECDH over BN254 G1
```
ephemeral_sk   ←  random F_p
ephemeral_pk   =  ephemeral_sk * G
shared_secret  =  ephemeral_sk * recipient_transmission_pk
```

**Encryption:** ChaCha20-Poly1305 (256-bit key derived from shared secret via BLAKE2b-256)
```
encryption_key = BLAKE2b-256(shared_secret || ephemeral_pk)
ciphertext     = ChaCha20-Poly1305.Encrypt(encryption_key, nonce=0, plaintext=note_plaintext)
```

**Note plaintext format (128 bytes, matching `sdk/src/notes/encrypt.ts`):**
```
value       : u64     (8 bytes)
asset_id    : [u8;56]  (56 bytes) — UTF-8 Stellar StrKey address (C.../G...), zero-padded — not a raw 32-byte field element; the field encoding used inside circuits/contracts is derived from this StrKey, not transmitted directly
rho         : [u8;32]  (32 bytes) — nullifier seed, random
rcm         : [u8;32]  (32 bytes) — commitment randomness, random field element (a full BN254 Fr element, not 16 bytes — an earlier draft of this spec under-sized it)
```

**Transmitted note (ciphertext bundle, 176 bytes, `ENCRYPTED_NOTE_LEN` on-chain):**
```
ephemeral_pk  : [u8;32]  — compressed G1 point
ciphertext    : [u8;144] — 128-byte plaintext + 16-byte Poly1305 MAC
```

### 2.5 Groth16 Proof System

Proof: `π = (A, B, C)` where `A, C ∈ BN254 G1`, `B ∈ BN254 G2`

Verification equation:
```
e(A, B) = e(α, β) · e(vk_x, γ) · e(C, δ)
```
Where `vk_x = Σ (public_input[i] * vk_IC[i])` over public inputs.

On Soroban this is a single `bn254_multi_pairing_check` call with 4 pairs — feasible within ledger resource limits.

---

## 3. Data Structures

### 3.1 Note

The fundamental unit of private balance.

```rust
struct Note {
    value:    u64,        // token amount (i128 on-chain in ct20; u64 in the transmitted plaintext)
    asset_id: BytesN<32>, // field-element encoding of the SEP-41 contract address (see §2.4 for the StrKey-vs-field-element distinction)
    rho:      BytesN<32>, // nullifier seed (random, unique per note)
    rcm:      BytesN<32>, // randomness for commitment (a full field element, not 16 bytes)
}
```

A note is considered **spent** when its nullifier appears in the on-chain nullifier set.

### 3.2 Note Commitment

```
cm = H_pos2(
       H_pos2(value_field, asset_id_field),
       H_pos2(rho_field, rcm_field)
     )
```

Where `*_field` denotes the field element representation (little-endian 32-byte → F_p).

The commitment is a 32-byte field element stored as a Merkle tree leaf.

### 3.3 Nullifier

```
nf = H_pos2(nk, rho)
```

Where `nk` is the nullifier key derived from the spending key (see §4). The nullifier reveals nothing about the note value, asset, or owner. It is unique per note because `rho` is unique per note.

### 3.4 Merkle Tree

**Type:** Binary incremental Merkle tree  
**Depth:** 32 (capacity: 2^32 ≈ 4 billion notes)  
**Hash:** Poseidon2  
**Empty leaf:** `H_pos2(0, 0)`  
**Internal node:** `H_pos2(left_child, right_child)`

```
root
├── H(H(cm0, cm1), H(cm2, cm3))
│   ├── H(cm0, cm1)
│   │   ├── cm0  ← leaf 0
│   │   └── cm1  ← leaf 1
│   └── H(cm2, cm3)
│       ├── cm2  ← leaf 2
│       └── cm3  ← leaf 3
...
```

Merkle path for leaf at index `i`: the 32 sibling nodes from leaf to root.

### 3.5 Public Inputs (Transfer Circuit)

```rust
struct TransferPublicInputs {
    anchor:            BytesN<32>,      // Merkle root at proof generation time
    nullifiers:        Vec<BytesN<32>>, // one per input note
    out_commitments:   Vec<BytesN<32>>, // one per output note
    in_value_commits:  Vec<BytesN<32>>, // Poseidon-based value bindings for the balance check (see §2.3's note on Pedersen vs. the current Poseidon-based simplification)
    out_value_commits: Vec<BytesN<32>>,
    fee:               i128,            // transaction fee in stroops — matches ct20's i128 amount type, not u64
    asset_id:          Address,         // SEP-41 contract address; must be consistent across all notes in one call
}
```

### 3.6 Ledger Storage Layout (Soroban)

```
StorageKey::MerkleRoot           → BytesN<32>
StorageKey::MerkleLeaf(index)    → BytesN<32>
StorageKey::NextLeafIndex        → u32
StorageKey::Nullifier(nf)        → bool
StorageKey::VerifyingKey         → Bytes  (serialized Groth16 VK)
StorageKey::AssetBalance(asset)  → i128   (total shielded supply per asset)
StorageKey::Paused               → bool
```

All storage uses Soroban `instance` storage for the contract metadata and `persistent` storage for Merkle leaves and nullifiers (requires rent payment; clients must extend TTL).

---

## 4. Key Management

### 4.1 Key Hierarchy

Real derivation, matching `sdk/src/keys/keys.ts`'s `fromSeed` exactly (domain-separated BLAKE2b, each reduced mod the BN254 scalar field order `r`, not the Poseidon-based construction an earlier draft of this spec described):

```
seed (32 bytes, random — no BIP-39 mnemonic support today)
│
└── spending_key (sk)
     = BLAKE2b-256(seed || "zkella_spend_v1") mod r
     │
     ├── nullifier_key (nk)
     │    = BLAKE2b-256(sk || "zkella_nullifier_v1") mod r
     │    [used to compute nullifiers, must stay secret]
     │
     ├── viewing_key (vk)
     │    = BLAKE2b-256(sk || "zkella_viewing_v1") mod r
     │    [can decrypt incoming notes, cannot spend, shareable with auditors]
     │
     └── transmission_key (tk)
          = vk * G   (BN254 G1 scalar mult, compressed 32 bytes)
          [public, used by senders to encrypt notes to this recipient]
```

`tk` is derived from `vk`, not `sk` — deliberately, so a viewing-key-only holder (an auditor who never has `sk`) can still be reached via the same ECDH relation notes are encrypted under, without needing the full spending key. An earlier implementation derived `tk` from `sk` directly and separately set `tk` to `vk`'s own raw bytes, which leaked the viewing key itself to anyone who saw a shielded address; `vk * G` is a one-way function of `vk`, closing that leak.

### 4.2 Address

A ZKELLA shielded address encodes a diversified transmission key, matching `ZKELLAKeys.deriveAddress`:

```
diversifier   =  BLAKE2b-11(sk || diversifier_index)
g_d           =  hash_to_curve_G1(diversifier)   (real BN254 G1 point, try-and-increment)
pk_d          =  vk * g_d                        (BN254 G1 scalar mult — by vk, not sk)

address = "zkella1" || Base58(version_byte || diversifier || pk_d)
```

Multiple addresses can be generated from one spending key (by `diversifier_index`). All resolve to the same viewing key — the diversified address model from Zcash Sapling. `g_d` doubles as the Diffie-Hellman base point a sender must use when encrypting to this specific diversified address (see `sdk/src/notes/encrypt.ts`'s `basePoint` parameter), so decryption still works via `vk`/`ephemeralPk` alone without the recipient needing to disclose which diversifier a given note used.

**Known gap:** the current encoding has **no checksum** — a typo'd address is silently a different, wrong address rather than a decode error. A SHA-256d 4-byte checksum (real Base58Check, as this section originally specified) is planned but not yet implemented; always verify shielded addresses out-of-band before sending funds.

### 4.3 Viewing Key Export Format

```json
{
  "version": 1,
  "network": "stellar_mainnet",
  "viewing_key": "<hex-encoded vk>",
  "transmission_key": "<hex-encoded tk>",
  "birthday_ledger": 12345678
}
```

`birthday_ledger` tells the indexer where to start scanning — avoids full history sync.

### 4.4 Auditor Proof-of-Compliance

An account holder can generate a ZK proof that their address does not appear in a published sanctions list (e.g. OFAC SDN list published as a Merkle tree):

```
proof: "I know a spending key sk such that:
  1. tk_commitment = Poseidon2(sk, 0)  (I control this address — a Poseidon-based
      binding today, not yet the real BN254 scalar-mult tk = sk * G; same
      simplification noted for ValueCommit in §2.3/CIRCUIT_SPEC.md)
  2. address ∉ sanctions_merkle_tree  (sorted Merkle non-membership proof)
  3. sanctions_root = <public value>  (against latest published root)"
```

Public inputs: `[sanctions_root, tk_commitment]`
Circuit: Poseidon-based Merkle non-membership proof + key-ownership binding (`circuits/compliance/non_membership.circom`, real and implemented — see `docs/CIRCUIT_SPEC.md` §7)
Proof size: 256 bytes (Groth16, uncompressed BN254 wire format — see §13.1)

---

## 5. Circuit Specifications

### 5.1 Transfer Circuit — 2-input / 2-output

**File:** `circuits/transfer_2in2out/transfer.circom`

**Private inputs:**
```
// Input notes (×2)
in_value[2]           : field
in_asset_id[2]        : field
in_rho[2]             : field
in_rcm[2]             : field
in_path[2][32]        : field  // Merkle auth path (32 siblings)
in_path_index[2][32]  : field  // 0 or 1 at each level

// Spending authority
nk                    : field  // nullifier key

// Output notes (×2)
out_value[2]          : field
out_asset_id[2]       : field
out_rho[2]            : field
out_rcm[2]            : field

// Value commitment randomness
rcv_in[2]             : field
rcv_out[2]            : field
```

**Public inputs:**
```
anchor                : field  // Merkle root
nullifiers[2]         : field
out_commitments[2]    : field
in_value_commits[2]   : field  // Pedersen commitments
out_value_commits[2]  : field
fee                   : field
asset_id              : field  // all notes must share same asset
```

**Constraints (approximate):**

| Constraint group | Gates |
|---|---|
| Input note commitment check (×2) | ~800 |
| Merkle path verification (×2 × 32 levels) | ~4,200 |
| Nullifier derivation (×2) | ~300 |
| Output commitment construction (×2) | ~800 |
| Value commitment (Pedersen) (×4) | ~1,200 |
| Balance check: Σin = Σout + fee | ~50 |
| Range proofs: values ∈ [0, 2^64) (×4) | ~8,000 |
| Asset consistency | ~100 |
| **Total** | **~15,450** |

Proving time estimate: ~1.5–2.5 seconds on a modern browser (snarkjs WASM, Groth16).

**Circuit logic (pseudocode):**
```
for i in 0..2:
  // verify input note commitment exists in tree
  computed_cm[i] = Poseidon2(Poseidon2(in_value[i], in_asset_id[i]),
                              Poseidon2(in_rho[i], in_rcm[i]))
  computed_root[i] = MerkleProof(computed_cm[i], in_path[i], in_path_index[i])
  computed_root[i] === anchor

  // derive nullifier
  computed_nf[i] = Poseidon2(nk, in_rho[i])
  computed_nf[i] === nullifiers[i]

  // value commitment
  in_value_commits[i] === PedersenCommit(in_value[i], rcv_in[i])

for i in 0..2:
  // construct output commitment
  computed_out_cm[i] = Poseidon2(Poseidon2(out_value[i], out_asset_id[i]),
                                  Poseidon2(out_rho[i], out_rcm[i]))
  computed_out_cm[i] === out_commitments[i]

  // output value commitment
  out_value_commits[i] === PedersenCommit(out_value[i], rcv_out[i])

  // asset consistency
  out_asset_id[i] === asset_id

// balance check (homomorphic on commitments)
in_value_commits[0] + in_value_commits[1]
  === out_value_commits[0] + out_value_commits[1] + fee * H_v

// range proofs
for each value in [in_value[0..2], out_value[0..2]]:
  value in [0, 2^64)
```

### 5.2 Transfer Circuit — 4-input / 4-output

**File:** `circuits/transfer_4in4out/transfer.circom`

Extends 2-in/2-out with 4 input and 4 output notes. Supports dust consolidation and multi-recipient payments.

Approximate gate count: ~28,000.  
Proving time estimate: ~4–6 seconds on a modern browser.

Public inputs include `nullifiers[4]` and `out_commitments[4]`.

### 5.3 Shield Circuit (public → shielded)

**File:** `circuits/shield/shield.circom`

Simpler circuit: no Merkle proof (note is not yet in the tree).

Private inputs: `value, asset_id, rho, rcm, rcv`
Public inputs: `commitment, value_commit, pub_value, pub_asset_id` — see `docs/CIRCUIT_SPEC.md` §2 for the exact template; `pub_value`/`pub_asset_id` are the revealed amount/asset the circuit constrains to equal the private `value`/`asset_id`, not additional independent signals.

Constraints: ~2,000 gates. Proving time: ~200ms.

### 5.4 Unshield Circuit (shielded → public)

**File:** `circuits/unshield/unshield.circom`

Private inputs: `value, asset_id, rho, rcm, nk, path[32], path_index[32]`
Public inputs: `anchor, nullifier, pub_value, pub_asset_id, recipient_hash` (amount and asset are revealed via `pub_value`/`pub_asset_id`; `recipient_hash = Poseidon2(address_field(to), 0)` binds the withdrawal destination — see `docs/CIRCUIT_SPEC.md` §3)

Constraints: ~6,200 gates. Proving time: ~600ms.

### 5.5 Swap Fairness Circuit

**File:** `circuits/swap/swap_fairness.circom`

Proves that a swap execution honoured the user's committed slippage tolerance, without having revealed `amount_in`/`max_slippage_bps`/`min_amount_out` at commit time. Matches `circuits/swap/swap_fairness.circom` exactly (see `docs/CIRCUIT_SPEC.md` §6).

Private inputs: `intent_nonce, amount_in, max_slippage_bps`
Public inputs: `intent_commitment, asset_in, asset_out, amount_out, min_amount_out`

```
intent_commitment === Poseidon2(
  Poseidon2(asset_in, asset_out),
  Poseidon2(amount_in * 2^32 + max_slippage_bps, intent_nonce)
)

min_amount_out === floor(amount_in * (10000 - max_slippage_bps) / 10000)
amount_out >= min_amount_out
```

`asset_in`/`asset_out` are public in this circuit (unlike `intent_nonce`/`amount_in`/`max_slippage_bps`, which stay private) because `contracts/swap::reveal_and_claim` needs to bind the proof to the specific assets already recorded in on-chain swap state before it will accept it. An earlier draft of this spec used a different `execution_price_bps` public signal that was never implemented; `min_amount_out`, derived in-circuit from the private `amount_in`/`max_slippage_bps`, is what the real circuit and contract use.

Constraints: ~3,500 gates. Proving time: ~400ms.

### 5.6 Sanctions Non-Membership Circuit

**File:** `circuits/compliance/non_membership.circom`

Proves address is not in a published sanctions Merkle tree.

Private inputs: `sk, non_membership_path[32], boundary_leaves[2]`  
Public inputs: `sanctions_root, tk_commitment`

Uses sorted Merkle tree non-membership proof: proves that the address falls between two consecutive leaves in the sorted tree (both provided as witnesses).

Constraints: ~9,000 gates.

### 5.7 Trusted Setup

All circuits use a **Groth16 trusted setup** with a circuit-specific Phase 2 ceremony on top of the universal Powers of Tau (ptau) from the Hermez/Iden3 ceremony (2^28 constraints, publicly verifiable).

Each circuit's Phase 2 (`zkey`) will be generated via a multi-party computation ceremony documented publicly. Beacon randomisation from a future Stellar ledger hash will be applied as the final contribution.

---

## 6. Smart Contract Interfaces

The interfaces in this section are the **real, current** contract surfaces (verified against `contracts/*/src/lib.rs` at the time of writing), not aspirational ones — an earlier draft of this spec described a materially different, never-built shape for the swap and governance contracts in particular; that draft is replaced below. These interfaces have been through an internal senior-auditor pass and a live-Testnet run (shield/unshield/swap), but not yet an *external* security review, so treat the behavior as real and exercised, not as finished, audited-by-a-third-party protocol logic.

### 6.1 CT-20 Token Contract

**File:** `contracts/ct20/src/lib.rs`

```rust
pub trait CT20Interface {
    fn initialize(env: Env, admin: Address, verifier: Address);

    /// Deposit a public SEP-41 token amount and receive a shielded note.
    /// Verifies a real Groth16 proof against the verifier's `Shield` circuit
    /// before inserting the note commitment into the Merkle tree.
    /// Emits: ("zkella","shield") { leaf_index, asset, commitment },
    ///        ("zkella","note")   { leaf_index, commitment, encrypted_note }
    fn shield(
        env:            Env,
        from:           Address,     // must authorize
        asset:          Address,     // SEP-41 token contract
        amount:         i128,
        rho:            BytesN<32>,
        rcm:            BytesN<32>,
        commitment:     BytesN<32>,  // note commitment
        encrypted_note: Bytes,       // 176-byte ciphertext bundle (see §2.4)
        shield_proof:   Bytes,       // Groth16 proof, 256-byte wire format (see §2.5)
        shield_pub:     ShieldPublicInputs,
    ) -> Result<u32, Error>;         // leaf index in Merkle tree

    /// Private note-to-note transfer, 2-in/2-out. Spends nullifiers, adds
    /// output commitments to the tree. `transfer4` is the same shape against
    /// the 4-in/4-out circuit.
    fn transfer(
        env:             Env,
        nullifiers:      Vec<BytesN<32>>,
        commitments:     Vec<BytesN<32>>,
        encrypted_notes: Vec<Bytes>,
        proof:           Bytes,
        pub_inputs:      TransferPublicInputs,
    ) -> Result<Vec<u32>, Error>;    // leaf indices of output commitments

    fn transfer4(
        env: Env, nullifiers: Vec<BytesN<32>>, commitments: Vec<BytesN<32>>,
        encrypted_notes: Vec<Bytes>, proof: Bytes, pub_inputs: TransferPublicInputs,
    ) -> Result<Vec<u32>, Error>;

    /// Reveal a note and withdraw to a public address.
    fn unshield(
        env:        Env,
        nullifier:  BytesN<32>,
        to:         Address,
        proof:      Bytes,
        pub_inputs: UnshieldPublicInputs,
    ) -> Result<(), Error>;

    fn merkle_root(env: Env) -> BytesN<32>;
    fn merkle_path(env: Env, leaf_index: u32) -> Vec<BytesN<32>>;
    fn leaf_count(env: Env) -> u32;
    fn is_spent(env: Env, nullifier: BytesN<32>) -> bool;
    fn shielded_supply(env: Env, asset: Address) -> i128;

    fn pause(env: Env) -> Result<(), Error>;
    fn unpause(env: Env) -> Result<(), Error>;
    fn transfer_admin(env: Env, new_admin: Address) -> Result<(), Error>;
    fn accept_admin(env: Env) -> Result<(), Error>;
}
```

**Verification logic (transfer, unshield, shield — all real, not pseudocode-only):** each calls `VerifierClient::new(&env, &verifier).verify(circuit, public_inputs, proof)`, which the `contracts/verifier` contract implements as: deserialize the wire-format proof into `(A, B, C)` over BN254, compute `vk_x = IC[0] + Σ public_input[i] · IC[i+1]` via `env.crypto().bn254().g1_msm(...)`, then a single `pairing_check([−A, α, vk_x, C], [B, β, γ, δ])` call. `−A` is computed as `g1_mul(A, r−1)` (scalar multiplication by `r−1` in a prime-order group is exact negation — no separate negate host function exists).

**Merkle tree insertion (incremental, `contracts/ct20/src/merkle.rs`):** a depth-32 tree; `merkle::insert` recomputes only the O(depth) empty-subtree roots needed for a fresh leaf (an earlier version recomputed the whole empty-hash chain from scratch on every level — O(depth²) — see `docs/POC_IMPLEMENTATION.md` for the fix and its budget impact).

### 6.2 Verifier Registry Contract

**File:** `contracts/verifier/src/lib.rs`

```rust
pub trait VerifierRegistry {
    fn initialize(env: Env, admin: Address);
    fn register_verifying_key(env: Env, circuit: CircuitType, vk: Bytes) -> Result<(), Error>; // first-time only
    fn update_verifying_key(env: Env, circuit: CircuitType, new_vk: Bytes) -> Result<(), Error>; // rotation, admin-gated
    fn get_verifying_key(env: Env, circuit: CircuitType) -> Result<Bytes, Error>;
    fn verify(env: Env, circuit: CircuitType, public_inputs: Vec<BytesN<32>>, proof: Bytes) -> bool;
}
```

`CircuitType` is `{ Shield = 0, Transfer = 1, Unshield = 2, NonMembership = 3, Transfer4x4 = 4, SwapFairness = 5 }` — one verifying key per variant, shared across `ct20`, `swap`, and `compliance`.

### 6.3 Viewing Key Registry Contract

**File:** `contracts/viewing_keys/src/lib.rs`

```rust
pub trait ViewingKeyRegistry {
    /// Register a viewing key commitment on-chain: vk_commitment = Poseidon2(vk, diversifier).
    fn register(env: Env, owner: Address, vk_commitment: BytesN<32>, birthday: u32);
    fn get_viewing_key_commitment(env: Env, owner: Address) -> Option<BytesN<32>>;
}
```

Sanctions/compliance proof publication is **not** on this contract — see §6.4. An earlier draft of this spec combined the two into one `ViewingKeyRegistry` trait; the repository implementation keeps them separate so an unrelated compliance-record store doesn't share a contract with the viewing-key registry.

### 6.4 Compliance Contract

**File:** `contracts/compliance/src/lib.rs`

```rust
pub trait Compliance {
    fn initialize(env: Env, verifier: Address);

    /// Publishes a verified sanctions-list non-membership proof for `owner`.
    /// Checked against the verifier's `NonMembership` circuit before storage
    /// — unlike an earlier design, an invalid proof is never stored.
    fn publish_compliance_proof(
        env: Env, owner: Address, proof: Bytes, pub_inputs: CompliancePublicInputs,
    ) -> Result<(), Error>;

    fn get_compliance_proof(env: Env, owner: Address) -> Option<ComplianceRecord>;
}

struct CompliancePublicInputs {
    sanctions_root: BytesN<32>,
    tk_commitment:  BytesN<32>,
}
```

### 6.5 Shielded Swap Contract

**File:** `contracts/swap/src/lib.rs`

The design here reuses `ct20`'s own already-real shield/unshield paths for the value-moving steps, rather than a separate DEX-execution model — see §9 for why this differs from earlier drafts of this spec.

```rust
pub trait ShieldedSwap {
    fn initialize(env: Env, admin: Address, verifier: Address, ct20: Address);

    /// Escrows `amount_in` of `asset_in` right now via a real `ct20::unshield`
    /// cross-call — `ownership_proof` is a genuine `unshield.circom` proof,
    /// reused as the swap's note-ownership proof (no separate ownership
    /// circuit exists or is needed).
    fn commit_swap(
        env: Env, nullifier_in: BytesN<32>, intent_commitment: BytesN<32>,
        asset_in: Address, asset_out: Address, amount_in: i128, anchor: BytesN<32>,
        refund_to: Address, ownership_proof: Bytes, expiry_ledger: u32,
    ) -> BytesN<32>; // swap_id = sha256(intent_commitment)

    /// Relayer really fronts `amount_out` of `asset_out` into escrow (a real
    /// SEP-41 transfer), in exchange for the already-escrowed `asset_in` once
    /// a valid fairness proof is revealed.
    fn execute_swap(env: Env, swap_id: BytesN<32>, amount_out: i128, relayer: Address);

    /// Verifies the fairness proof (binds the revealed `amount_out` back to
    /// the original `intent_commitment`, without `min_amount_out` having been
    /// revealed at commit time), pays the relayer the escrowed `asset_in`,
    /// and re-shields `asset_out` as a new note via a real, separate
    /// `ct20::shield` call (`shield_proof` — a distinct proof from
    /// `fairness_proof`).
    fn reveal_and_claim(
        env: Env, swap_id: BytesN<32>, out_rho: BytesN<32>, out_rcm: BytesN<32>,
        out_commitment: BytesN<32>, out_value_commit: BytesN<32>, encrypted_note: Bytes,
        fairness_proof: Bytes, fairness_pub: SwapFairnessPublicInputs, shield_proof: Bytes,
    ) -> u32; // output note leaf index

    /// Refunds `asset_in` to `refund_to` if never executed, once expired.
    fn cancel_swap(env: Env, swap_id: BytesN<32>);

    /// Unwinds an executed-but-never-claimed swap once a grace window past
    /// expiry has passed: returns the relayer's fronted `asset_out` *and*
    /// refunds the escrowed `asset_in`, in the same call.
    fn reclaim_expired_swap(env: Env, swap_id: BytesN<32>);

    fn set_relayer(env: Env, relayer: Address, approved: bool); // admin-gated, scoped to this contract
}

struct SwapFairnessPublicInputs {
    intent_commitment: BytesN<32>,
    asset_in:          Address,
    asset_out:         Address,
    amount_out:        i128,
    min_amount_out:    i128,
}
```

### 6.6 Governance Contract

**File:** `contracts/governance/src/lib.rs`

Owns timelocked verifying-key rotation on top of `contracts/verifier`, and its own admin lifecycle. `contracts/governance` is set as `contracts/verifier`'s own admin address, so its cross-contract calls into `verifier` satisfy that contract's `admin.require_auth()` implicitly.

```rust
pub trait ZKELLAGovernance {
    fn initialize(env: Env, admin: Address, verifier: Address);
    fn register_vk(env: Env, circuit: CircuitType, vk: Bytes);        // first-time registration, no timelock
    fn queue_vk_update(env: Env, circuit: CircuitType, new_vk: Bytes); // starts the 7-day timelock
    fn execute_vk_update(env: Env, circuit: CircuitType);              // after the timelock elapses
    fn cancel_vk_update(env: Env, circuit: CircuitType);
    fn transfer_admin(env: Env, new_admin: Address);                  // two-step handover
    fn accept_admin(env: Env);
}
```

Relayer authorization (`set_relayer`) lives on `contracts/swap` itself (§6.5), not on governance — an earlier draft of this spec placed it here; the repository implementation scopes relayer approval to the contract that actually uses it.

---

## 7. Protocol Flows

### 7.1 Shield Flow (Public → Shielded)

```
User                          zkella-sdk                    CT-20 Contract
 │                                │                               │
 │  shield(asset, amount)         │                               │
 ├───────────────────────────────>│                               │
 │                                │  generate note                │
 │                                │  (value, asset, rho, rcm)    │
 │                                │                               │
 │                                │  prove shield circuit         │
 │                                │  (~200ms, WASM)               │
 │                                │                               │
 │                                │  approve asset transfer       │
 │                                ├──────────────────────────────>│
 │                                │  call shield(...)             │
 │                                ├──────────────────────────────>│
 │                                │                               │ verify proof
 │                                │                               │ transfer tokens in
 │                                │                               │ insert cm into tree
 │                                │                               │ emit NoteCommitmentEvent
 │                                │<──────────────────────────────│
 │                                │  leaf_index                   │
 │  note stored in local wallet   │                               │
 │<───────────────────────────────│                               │
```

### 7.2 Private Transfer Flow

```
Sender                        zkella-sdk                    CT-20 Contract
 │                                │                               │
 │  transfer(recipient, amount)   │                               │
 ├───────────────────────────────>│                               │
 │                                │  select input notes           │
 │                                │  (from local note set)        │
 │                                │                               │
 │                                │  fetch Merkle paths           │
 │                                │  (from indexer or RPC)        │
 │                                │                               │
 │                                │  construct output notes       │
 │                                │  encrypt to recipient         │
 │                                │                               │
 │                                │  prove transfer_2in2out       │
 │                                │  (~2s, WASM Groth16)          │
 │                                │                               │
 │                                │  submit transaction           │
 │                                ├──────────────────────────────>│
 │                                │                               │ verify proof
 │                                │                               │ check anchor ∈ valid roots
 │                                │                               │ reject spent nullifiers
 │                                │                               │ mark nullifiers spent
 │                                │                               │ insert output cms
 │                                │                               │ emit events
 │  nf1, nf2 marked spent         │<──────────────────────────────│
 │<───────────────────────────────│                               │

Recipient                     zkella-sdk                    Note Indexer
 │                                │                               │
 │  (background sync)             │                               │
 │                                │  fetch new encrypted notes    │
 │                                ├──────────────────────────────>│
 │                                │<──────────────────────────────│
 │                                │  try decrypt with vk          │
 │                                │  (one note decrypts ✓)        │
 │                                │  verify cm matches decrypted  │
 │  new note added to wallet      │                               │
 │<───────────────────────────────│                               │
```

### 7.3 Unshield Flow (Shielded → Public)

```
User                          zkella-sdk                    CT-20 Contract
 │                                │                               │
 │  unshield(note, recipient)     │                               │
 ├───────────────────────────────>│                               │
 │                                │  fetch Merkle path            │
 │                                │  prove unshield circuit       │
 │                                │  (~600ms)                     │
 │                                │                               │
 │                                │  call unshield(...)           │
 │                                ├──────────────────────────────>│
 │                                │                               │ verify proof
 │                                │                               │ mark nullifier spent
 │                                │                               │ transfer tokens to recipient
 │                                │                               │ emit UnshieldEvent
 │  tokens received publicly      │<──────────────────────────────│
 │<───────────────────────────────│                               │
```

---

## 8. Persistent State Manager

### 8.1 Problem

Stellar RPC nodes retain contract events for ~17,280 ledgers (~7 days at 5s/ledger). New users who were not online cannot reconstruct their note set from the public RPC endpoint alone.

### 8.2 Architecture

The real reference implementation (`indexer/`, TypeScript/Node) uses `node:sqlite` rather than the PostgreSQL store this diagram originally specified as the target — no external database dependency, and `merkle_root`/`merkle_path` are proxied live to `ct20` itself rather than duplicated into their own tables, since the contract is already the source of truth for current tree state:

```
                ┌─────────────────────────────┐
                │       zkella-indexer         │
                │       (Node/TypeScript)       │
  Stellar RPC ──► Event Listener               │
                │   └── Soroban event stream   │
                │                              │
                │  Note Store (node:sqlite)     │
                │   ├── encrypted_notes table  │
                │   └── nullifiers table        │
                │   (merkle state: proxied live │
                │    to ct20, not stored here)  │
                │                              │
                │  REST API                    │◄── zkella-sdk
                │   ├── GET /notes             │
                │   ├── GET /merkle/path/{idx} │
                │   ├── GET /merkle/root        │
                │   ├── GET /commitment/{hex}   │
                │   ├── POST /nullifiers/batch  │
                │   └── GET /health             │
                └─────────────────────────────┘
```

### 8.3 Indexer API Specification

Self-hosted only today — there is no hosted `indexer.zkella.io` endpoint; run `npm run indexer` per `indexer/README.md` and point the SDK at your own instance (default `http://localhost:8787`).

```
GET /notes?from_ledger={n}&limit={m}
Response: {
  notes: [{ leaf_index, commitment, encrypted_note, ledger }],
  next_ledger: n
}

GET /merkle/path/{leaf_index}
Response: {
  path: [BytesN<32> × 32],
  path_index: [0|1 × 32],
  root: BytesN<32>
}

GET /merkle/root
Response: { root: BytesN<32>, leaf_count: u32 }

POST /nullifiers/batch
Body: { nullifiers: [BytesN<32>] }
Response: { spent: { [nullifier]: bool } }

GET /health
Response: { synced_ledger: u32, tip_ledger: u32, lag: u32 }
```

### 8.4 Client-Side Sync Protocol

```typescript
async function syncWallet(vk: ViewingKey, lastSyncLedger: number): Promise<Note[]> {
  const newNotes: Note[] = []
  let cursor = lastSyncLedger

  while (true) {
    const { notes, next_ledger } = await indexer.getNotes(cursor)
    if (notes.length === 0) break

    for (const { commitment, encrypted_note, leaf_index } of notes) {
      const plaintext = tryDecrypt(vk, encrypted_note)
      if (plaintext === null) continue  // not ours

      const computed_cm = computeCommitment(plaintext)
      if (computed_cm !== commitment) continue  // integrity check

      newNotes.push({ ...plaintext, leaf_index, commitment })
    }
    cursor = next_ledger
  }

  // Check which of our notes are spent
  const nullifiers = newNotes.map(n => computeNullifier(vk.nk, n.rho))
  const spent = await indexer.batchCheckNullifiers(nullifiers)

  return newNotes.filter((n, i) => !spent[nullifiers[i]])
}
```

### 8.5 Encrypted Backup

Users can export their full note set as an encrypted backup file:

```json
{
  "version": 1,
  "birthday_ledger": 12345678,
  "encrypted_payload": "<base64>",
  "mac": "<base64>"
}
```

Payload encrypted with AES-256-GCM using a key derived from the spending key:  
`backup_key = BLAKE2b-256(sk || "zkella_backup_v1")`

---

## 9. Shielded Swap Primitive

**This section describes the real, implemented, audited design** (`contracts/swap`). An earlier draft of this spec described a Stellar-DEX-execution model (relayers calling `PathPaymentStrictReceive`/`ManageSellOffer`, an off-chain P2P relay server) that was never built this way — nothing in the current contract calls the Stellar DEX. What's implemented instead is simpler and already real: the relayer directly fronts the output asset as SEP-41 liquidity, and the contract's own escrow/payout logic (reusing `ct20`'s shield/unshield paths) does the rest. Routing that liquidity through the actual DEX, if the relayer chooses to, is an off-chain concern the contract doesn't need to know about — wiring an on-chain DEX call into the flow itself remains roadmap work, not something this section should describe as already specified in detail.

### 9.1 Trust Model

The shielded swap uses a **weak privacy model**:
- Amount hidden from on-chain passive observers, up to what `commit_swap`'s public arguments reveal (`asset_in`, `asset_out`, `amount_in` are currently public call arguments, not hidden — see the note in §12.4)
- Amount revealed to the designated relayer, who must front the exact `amount_out` in `execute_swap`

This is sufficient for basic front-running protection on the *output* amount and price (which is only revealed at `reveal_and_claim` time, bound to the original commitment by the fairness proof), not for full amount confidentiality throughout the swap.

### 9.2 Relayer Model

Relayers are permissioned per-swap-contract via `set_relayer(relayer, approved)` (admin-gated, scoped to `contracts/swap` itself — not the governance contract). A relayer:
- calls `execute_swap` to front `amount_out` of `asset_out` into escrow — a real SEP-41 transfer, checked by the contract, not a promise,
- is paid the escrowed `asset_in` once the claimant reveals a valid fairness proof via `reveal_and_claim`,
- can reclaim their fronted `asset_out` (and the claimant's `refund_to` gets `asset_in` back) via `reclaim_expired_swap` if the claimant never claims within the grace window after expiry.

How a relayer actually sources `amount_out` — including whether they route through the Stellar DEX — is entirely off-chain and outside the contract's concern; the contract only verifies that the real transfer happened.

### 9.3 Swap Flow Detail (real, matches `contracts/swap/src/lib.rs`)

```
Step 1 — User: build the fairness-circuit witness off-chain
  intent = { asset_in, asset_out, amount_in, max_slippage_bps, intent_nonce }
  min_amount_out = floor(amount_in * (10000 - max_slippage_bps) / 10000)
  intent_commitment = Poseidon2(Poseidon2(asset_in, asset_out),
                                 Poseidon2(amount_in * 2^32 + max_slippage_bps, intent_nonce))
  (amount_out and min_amount_out stay private until reveal_and_claim)

Step 2 — User: commit_swap(nullifier_in, intent_commitment, asset_in, asset_out,
                            amount_in, anchor, refund_to, ownership_proof, expiry_ledger)
  - ownership_proof is a real unshield.circom proof; the call cross-calls
    ct20::unshield(nullifier_in, swap_contract_address, ownership_proof, ...),
    which both verifies note ownership and atomically escrows amount_in of
    asset_in into the swap contract's own balance
  - swap_id = sha256(intent_commitment) is returned

Step 3 — Relayer: execute_swap(swap_id, amount_out, relayer)
  - relayer really transfers amount_out of asset_out into escrow
  - state moves Committed -> Executed

Step 4 — User: reveal_and_claim(swap_id, out_rho, out_rcm, out_commitment,
                                 out_value_commit, encrypted_note,
                                 fairness_proof, fairness_pub, shield_proof)
  - fairness_proof (real swap_fairness.circom proof) is checked against the
    verifier; binds the now-revealed amount_out/min_amount_out back to
    intent_commitment
  - relayer is paid the escrowed asset_in
  - a separate, real shield.circom proof (shield_proof) re-shields amount_out
    of asset_out as a brand-new note via ct20::shield, returning its leaf index

Step 5 (fallback) — anyone: cancel_swap(swap_id) once expired and never executed
  refunds asset_in to refund_to

Step 5b (fallback) — anyone: reclaim_expired_swap(swap_id), once executed but
  never claimed and the post-expiry grace window has passed
  returns the relayer's fronted asset_out AND refunds asset_in to refund_to,
  in the same call
```

This full lifecycle has been run end-to-end on live Stellar Testnet with real Groth16 proofs at every stage — see `docs/POC_IMPLEMENTATION.md` for transaction hashes.

---

## 10. Viewing Key and Compliance Layer

### 10.1 Auditor Workflow

```
Regulated Institution (Auditor)          Account Holder
         │                                      │
         │  Request viewing key for audit       │
         ├─────────────────────────────────────>│
         │                                      │
         │                          Export vk JSON
         │<─────────────────────────────────────│
         │                                      │
         │  Import vk into zkella-sdk           │
         │  sync from birthday_ledger           │
         │  decrypt all notes → full history    │
         │                                      │
```

The viewing key allows the auditor to see:
- All incoming note amounts and asset types
- All outgoing nullifiers (can match to commitments)
- Full transaction history reconstruction

The viewing key does NOT allow:
- Spending funds
- Deriving the spending key

### 10.2 FATF Travel Rule Compliance

For transfers above the threshold (€1,000 / $1,000 per FATF Recommendation 16):

```
Originating VASP                       Beneficiary VASP
       │                                      │
       │  Travel Rule payload (encrypted)     │
       │  { originator_info,                  │
       │    beneficiary_address,              │
       │    amount_commitment,                │
       │    asset_id }                        │
       ├─────────────────────────────────────>│
       │                                      │
       │                         Verify amount_commitment
       │                         matches on-chain transfer
       │                         using amount_commitment
       │                         (no amount revealed to public)
```

Amount commitments allow VASPs to verify transfer amounts between themselves without publishing amounts publicly.

### 10.3 Sanctions Screening

```typescript
// Published by compliance providers as a Merkle tree over sorted addresses
interface SanctionsList {
  root: BytesN<32>
  version: string
  published_ledger: number
}

// User generates proof locally — never sends spending key to compliance provider
async function generateComplianceProof(
  sk: SpendingKey,
  sanctions: SanctionsList
): Promise<ComplianceProof> {
  const address = deriveAddress(sk)
  const { path, boundary_leaves } = await sanctions.nonMembershipPath(address)
  const proof = await proveNonMembership(sk, path, boundary_leaves, sanctions.root)
  return { proof, sanctions_root: sanctions.root, version: sanctions.version }
}
```

---

## 11. Developer SDK

### 11.1 Package Structure

The package is named `@zkella/sdk` (`sdk/package.json`) but has not been published to the npm registry yet — it's consumed today via local TypeScript imports within this monorepo (`sdk/src/...`). The real structure, current as of this writing:

```
sdk/
├── src/
│   ├── keys/          # ZKELLAKeys — real: spending/nullifier/viewing/transmission key derivation, diversified addresses
│   ├── notes/         # Real: note construction, commitment/nullifier/value-commit computation, ECDH encryption
│   ├── crypto/         # Real: Poseidon2 (circomlibjs) and BN254 G1 ops (ffjavascript) backing keys/notes
│   ├── prover/         # Real: snarkjs-based Groth16 proof generation for shield, transfer, transfer4, unshield, swapFairness
│   ├── wallet/
│   │   ├── wallet.ts    # Real — ZKELLAWallet: shield()/transfer()/unshield() build real proofs and submit real signed Soroban transactions
│   │   ├── swap.ts      # Stub — ZKELLASwap's methods return placeholders; the real contracts/swap contract works, this wrapper isn't wired to it yet
│   │   └── auditor.ts   # Stub — ZKELLAAuditor.sync() never actually decrypts anything
│   ├── compliance/      # Stub — ZKELLACompliance's proof generation/publishing are placeholders
│   ├── indexer/         # Real — IndexerClient, matches the real indexer/ service's HTTP API
│   └── types.ts
```

There is no `sdk/src/circuits/` or `sdk/src/contracts/` directory — compiled circuit artifacts live under the top-level `circuits/<name>/build/` (referenced by path from `sdk/src/prover/*`), and there are no generated Soroban contract-client bindings for TypeScript yet; `sdk/src/wallet/wallet.ts` builds `ScVal`s by hand (see its `structScVal`/`vecScVal` helpers).

### 11.2 Core API (real methods marked; stubs marked explicitly)

```typescript
// Key management — real
const keys = await ZKELLAKeys.fromSeed(seed)   // async — derives sk/nk/vk/tk
// keys.spendingKey.{raw, nullifierKey, viewingKey, transmissionKey}

// Wallet — real
const wallet = new ZKELLAWallet({
  keys: keys.spendingKey,
  network:     'testnet',                               // 'testnet' | 'mainnet'
  sorobanRpc:  'https://soroban-testnet.stellar.org',
  indexerUrl:  'http://localhost:8787',
  ct20Address: 'CXXX...YYY',
  stellarSecret: 'S...',                                 // signs the submitted transactions
  shieldCircuit:    { wasmPath: '...shield.wasm',    zkeyPath: '...shield.zkey' },
  transferCircuit:  { wasmPath: '...transfer.wasm',  zkeyPath: '...transfer.zkey' },
  unshieldCircuit:  { wasmPath: '...unshield.wasm',  zkeyPath: '...unshield.zkey' },
})

await wallet.sync()  // fetch and decrypt all notes from the indexer

const balance = await wallet.balance(USDC_CONTRACT)
// { shielded: 1000n } — there is no separate public/"unshielded" balance field;
// that's the wallet's own Stellar account balance, tracked outside this SDK

// Shield — real: builds a real note + real Groth16 proof, returns a submit() thunk
const { note, submit } = await wallet.shield({ asset: USDC_CONTRACT, amount: 100_000_000n })
const { leafIndex } = await submit()

// Transfer — real, but needs >=2 spendable notes (no dummy-input support yet);
// only 2-in/2-out is wired into the wallet today (transfer4's prover exists,
// but the wallet doesn't do 4-input note selection yet)
const { submit: submitTransfer } = await wallet.transfer({
  to:     '<recipient's raw hex transmission key>',       // full zkella1... diversified-address parsing isn't wired into the wallet yet
  asset:  USDC_CONTRACT,
  amount: 50_000_000n,
})
await submitTransfer()

// Unshield — real; full-note withdrawal only (no unshield-with-change entrypoint)
const { submit: submitUnshield } = await wallet.unshield({
  asset:  USDC_CONTRACT,
  amount: 25_000_000n,
  to:     'GABCD...WXYZ',
})
await submitUnshield()

// Viewing key export — real
const vkExport = wallet.exportViewingKey()

// Shielded swap — STUB: contracts/swap itself is real, audited, and has been
// run end-to-end on live Testnet (see docs/POC_IMPLEMENTATION.md), but the
// ZKELLASwap wrapper class shown in earlier drafts of this spec (commitSwap/
// waitForExecution/revealAndClaim/cancelSwap) is not implemented — its
// methods return placeholder values today.

// Compliance / auditor — STUB: ZKELLACompliance.generateNonSanctionedProof()
// and ZKELLAAuditor's note decryption are both placeholders today, even
// though the underlying contracts/compliance contract is real.
```

### 11.3 Note Selection Strategy (target design; wallet.ts's current implementation is simpler)

The real `wallet.transfer()` today picks the two largest unspent notes of the target asset (simple, not fee-optimal coin selection) rather than the smallest-first/fragmentation-minimizing strategy below, which remains the intended target:

```typescript
function selectNotes(
  notes: Note[],
  targetAmount: bigint,
  maxInputs: 2 | 4
): Note[] {
  const sorted = notes.slice().sort((a, b) => Number(a.value - b.value))
  const selected: Note[] = []
  let total = 0n

  for (const note of sorted) {
    if (total >= targetAmount) break
    selected.push(note)
    total += note.value
    if (selected.length === maxInputs) break
  }

  if (total < targetAmount) throw new InsufficientBalanceError()
  return selected
}
```

---

## 12. Security Analysis

### 12.1 Threat Model

| Threat | Mitigation |
|---|---|
| Observer learns transfer amount | Amounts stay private circuit witnesses, never on-chain in plaintext — the transfer circuit's balance check runs over the private values directly (see §2.3's implementation note: today's `value_commit` is Poseidon-based, not yet the real homomorphic Pedersen-over-G1 construction, but this doesn't weaken the balance-conservation guarantee itself) |
| Observer links sender to recipient | Note commitments are unlinkable; nullifiers reveal nothing about notes |
| Double spend | On-chain nullifier set; contract rejects duplicate nullifiers atomically |
| Invalid proof accepted | BN254 multi-pairing verification on Soroban; forgery requires breaking BN254 DL |
| Malicious verifying key update | 7-day timelock on governance; users can exit before upgrade takes effect |
| 7-day RPC retention | Persistent indexer retains full note history |
| Malicious indexer | Client verifies every decrypted note's commitment against on-chain Merkle root |
| Front-running of unshield | Unshield binds to specific recipient address in circuit public inputs |
| Relayer censorship (swap) | Multiple competing relayers; expiry + cancel path for user recovery |
| Note theft by compromised vk | Viewing key cannot derive spending key or nullifier key |
| Grinding attack on Merkle root | Anchor validity: contract accepts proofs against any root in the last 100 insertions |

### 12.2 Soundness Dependencies

- Groth16 soundness under the Generic Group Model (GGM) and q-PKE assumption over BN254
- BN254 discrete logarithm hardness (no known attack below 128-bit security)
- Poseidon2 collision resistance (cryptanalysis ongoing; considered secure for ZK applications)
- Pedersen commitment binding under BN254 DL hardness

### 12.3 Trusted Setup Risk

Groth16 requires a circuit-specific trusted setup. The toxic waste from Phase 2 must be destroyed. If any single participant in the MPC ceremony destroys their contribution, the setup is sound. ZKELLA will run a public ceremony with:
- Minimum 10 independent participants
- Beacon randomization from a Stellar ledger hash
- All contributions posted publicly for verification
- Final parameters committed to a Git repository with an immutable tag

If users do not trust the ceremony, they should wait for a PLONK-based circuit (no trusted setup) in a future version.

### 12.4 Known Limitations (v1)

1. The shielded swap relayer learns swap parameters off-chain
2. A global passive adversary observing the Stellar network can correlate shield/unshield timing with external activity
3. Note set size is limited to 2^32 (~4 billion) by the 32-level Merkle tree
4. Circuit support is limited to homogeneous asset transfers (all inputs and outputs must share the same asset_id in one proof)
5. `commit_swap`'s `asset_in`, `asset_out`, and `amount_in` are plain, public call arguments today, not hidden inside the intent commitment's proof — only `min_amount_out`/`amount_out` stay private until `reveal_and_claim`. Full amount/asset confidentiality throughout the swap (not just at reveal time) is not yet part of the implemented design.

---

## 13. Performance and Resource Budget

### 13.1 Client-Side Proving Times (snarkjs WASM, Node/browser)

| Circuit | Gates | Proving Time | Proof Size |
|---|---|---|---|
| Shield | ~2,000 | ~200ms | 256 bytes |
| Unshield | ~6,000 | ~600ms | 256 bytes |
| Transfer 2-in/2-out | ~15,450 | ~2.0s | 256 bytes |
| Transfer 4-in/4-out | ~28,000 | ~4.5s | 256 bytes |
| Swap fairness | ~3,500 | ~400ms | 256 bytes |
| Sanctions non-membership | ~9,000 | ~1.0s | 256 bytes |

All Groth16 proofs are 256 bytes regardless of circuit size (uncompressed BN254 points — see `docs/CIRCUIT_SPEC.md` §1 for why this isn't the 192-byte compressed size some Groth16 tooling defaults to). Proving-time estimates in this table are unmeasured design-time guesses, not benchmarked numbers.

### 13.2 Soroban On-Chain Verification Cost

The table below is the original design-time estimate. It has since been superseded by a **real measurement**: a full `shield()` call (commitment computation + Merkle insert + real on-chain Groth16 verification) costs **~104M instructions** in Soroban's own host environment (`InvocationResourceLimits::mainnet()`, 400M budget) — about 26%, with the verifier's cross-contract Groth16 check alone at ~30M of that. This was also confirmed on live Stellar Testnet across four real `shield()` transactions. See `docs/POC_IMPLEMENTATION.md` for the methodology, the regression test, and transaction hashes. The estimate below undercounts by roughly two orders of magnitude — kept here only to show how far an unmeasured guess can be from Soroban's real per-operation cost, not as a usable budget figure.

| Operation | Soroban Instructions (original, unmeasured estimate) |
|---|---|
| Deserialize proof + public inputs | ~50,000 |
| Compute vk_x (N public inputs × G1 mul + add) | ~200,000–400,000 |
| bn254_multi_pairing_check (4 pairs) | Native — does not consume instruction budget proportionally |
| Merkle root update (32 levels × Poseidon2) | ~160,000 |
| Nullifier storage write (×N) | ~50,000 per nullifier |

### 13.3 Indexer Resource Requirements

The real reference implementation (`indexer/`) uses Node's built-in `node:sqlite` — no external database dependency — which is a meaningfully different (and much lighter) profile than the PostgreSQL-based target this table originally described:

| Resource | Real reference implementation | Target (production, multi-operator) |
|---|---|---|
| Runtime | Node.js 22.5+ (`node:sqlite`, experimental) | Same, or a compiled service |
| Storage | SQLite file, size scales with note/nullifier event volume | PostgreSQL or equivalent durable store, sized for target throughput |
| CPU / RAM | Single core / low RAM sufficient for reference-scale testnet use | 2–4 cores, 2–8 GB RAM depending on load |
| Operators | One (this repo's reference deployment) | Multiple independent operators for resilience |

---

## 14. Deployment Plan

The deployment plan starts from the current soft PoC baseline. Before any final release, all existing contracts and SDK modules must move through review, implementation completion, resource profiling, and hardening. The current PoC contracts should not be promoted directly to production.

### 14.0 Reviewer-readiness milestones

To address the main review concerns directly, the roadmap now includes explicit milestones for:

- a real testnet shield transaction that completes with on-chain proof verification within Soroban budget,
- a documented custom-indexer deployment model with replay support, health monitoring, and independent operator compatibility,
- an operational runbook and incident-response plan for contract failures, indexer outages, and key handling,
- a clear compliance narrative around viewing keys and selective disclosure,
- public testnet evidence and a visible milestone cadence for Stellar ecosystem engagement.

**Current status against this plan:** shield/transfer/unshield are past "review and improve" and have real Groth16 verification, exercised on live Testnet (§14.1's "shield → transfer → unshield full cycle" is done for shield and unshield end-to-end with real value movement; transfer is validated locally with real proofs, not yet on live Testnet). The shielded swap contract has also been audited and run end-to-end on live Testnet — ahead of where this phased plan originally placed it. The trusted-setup ceremony used for every real-circuit test and every live-Testnet transaction to date is explicitly a local, single-contributor dev ceremony (§14.1's testnet ceremony step, not §14.3's production one) — see `docs/POC_IMPLEMENTATION.md` for exactly what's been validated where. The SDK has not been published to npm under any tag yet (§14.1's `@zkella/sdk@0.1.0-testnet` milestone), and no external security review (§14.2) has happened — the audit work in this repository so far was performed by the team building the protocol.

### 14.1 Testnet Phase (Months 1–4)

- Review and improve existing soft PoC contracts before expanding testnet coverage
- Deploy completed testnet versions of all contracts to Stellar Testnet
- Run trusted setup ceremony (testnet parameters — NOT for production)
- Publish circuit artifacts and verifying keys to GitHub
- Internal end-to-end testing: shield → transfer → unshield full cycle
- Indexer deployed on a public testnet endpoint
- SDK published to npm as `@zkella/sdk@0.1.0-testnet`

### 14.2 Security Review Phase (Months 5-6)

- Run independent review of CT-20 contract, viewing key contract, swap contract, and Circom circuits
- Scope: CT-20 contract, viewing key contract, swap contract, all Circom circuits
- Address all security findings before mainnet
- Re-profile Soroban resource usage after every material contract or circuit change
- Freeze final contract interfaces only after review findings and performance issues are resolved

### 14.3 Mainnet Phase (Month 7–8)

- Production trusted setup ceremony (multi-party, public)
- Verifying keys committed to immutable Git tag
- Deploy to Stellar Mainnet
- Indexer deployed with redundancy (minimum 2 independent operators)
- SDK published as `@zkella/sdk@1.0.0`
- Reference wallet deployed at `app.zkella.io`

### 14.4 Repository Layout

Real, current layout (contract and circuit sets have grown since this section was first written):

```
ZKELLA/
├── circuits/
│   ├── common/                  # shared Circom templates (Poseidon2, Merkle, range, commitments)
│   ├── shield/
│   ├── unshield/
│   ├── transfer_2in2out/
│   ├── transfer_4in4out/
│   ├── swap/                    # swap_fairness.circom
│   └── compliance/               # non_membership.circom
│       each with build/*.r1cs, *.zkey, *_js/*.wasm, verification_key.json (generated; dev ceremony only so far)
├── contracts/
│   ├── ct20/                    # confidential token: shield/transfer/transfer4/unshield
│   ├── ct20-interface/           # #[contractclient]-only crate — lets other contracts call ct20 without pulling in its own #[contract] exports
│   ├── verifier/                 # shared Groth16 verifying-key registry + verify()
│   ├── verifier-interface/        # same #[contractclient]-only pattern for verifier
│   ├── governance/               # timelocked verifying-key rotation
│   ├── viewing_keys/              # viewing-key commitment registry
│   ├── compliance/               # sanctions non-membership proof storage
│   └── swap/                     # shielded swap primitive
├── indexer/                      # TypeScript/Node service (not Go/Rust) — node:sqlite, no build step
├── sdk/                          # @zkella/sdk — not yet published to npm; consumed via local imports
├── app/                          # reference wallet (planned, not yet started)
└── docs/
    ├── TECHNICAL_SPEC.md         # this document
    ├── CIRCUIT_SPEC.md           # detailed constraint listings
    ├── ARCHITECTURE.md           # full system architecture
    ├── POC_IMPLEMENTATION.md     # what's validated where (local vs. live Testnet)
    ├── SCF_READINESS.md          # reviewer-response and milestone package
    └── INTEGRATION_GUIDE.md      # for third-party builders
```

---

*ZKELLA Protocol — Technical Specification v0.1.0*
