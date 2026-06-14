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

**Note plaintext format (88 bytes):**
```
value       : u64    (8 bytes)
asset_id    : [u8;32] (32 bytes)  — SEP-41 asset contract address
rho         : [u8;32] (32 bytes)  — nullifier seed, random
rcm         : [u8;16] (16 bytes)  — commitment randomness, random
```

**Transmitted note (ciphertext bundle, 152 bytes):**
```
ephemeral_pk  : [u8;32]  — compressed G1 point
ciphertext    : [u8;104] — encrypted note plaintext + 16-byte Poly1305 MAC
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
    value:    u64,        // token amount
    asset_id: BytesN<32>, // SEP-41 contract address
    rho:      BytesN<32>, // nullifier seed (random, unique per note)
    rcm:      BytesN<16>, // randomness for commitment
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
    anchor:           BytesN<32>,    // Merkle root at proof generation time
    nullifiers:       Vec<BytesN<32>>, // one per input note
    commitments:      Vec<BytesN<32>>, // one per output note
    value_commitments: Vec<BytesN<32>>, // Pedersen commitments for balance check
    fee:              u64,           // transaction fee in stroops
    asset_id:         BytesN<32>,    // must be consistent across all notes
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

```
seed (32 bytes, BIP-39 mnemonic or random)
│
└── spending_key (sk)
     = BLAKE2b-256(seed || "zkella_spend_v1")
     ∈ F_p
     │
     ├── nullifier_key (nk)
     │    = H_pos2(sk, 1)
     │    ∈ F_p
     │    [used to compute nullifiers, must stay secret]
     │
     ├── viewing_key (vk)
     │    = H_pos2(sk, 2)
     │    ∈ F_p
     │    [can decrypt incoming notes, cannot spend, shareable with auditors]
     │
     └── transmission_key (tk)
          = sk * G   (BN254 G1 point, compressed 32 bytes)
          [public, used by senders to encrypt notes to this recipient]
```

### 4.2 Address

A ZKELLA shielded address encodes the transmission key and a diversifier:

```
diversifier   ←  random 11 bytes
d_G           =  diversifier * G   (BN254 G1 point)
pk_d          =  sk * d_G          (BN254 G1 point)

address = Base58Check(diversifier || pk_d_compressed)
```

Multiple addresses can be generated from one spending key. All resolve to the same viewing key. This is the diversified address model from Zcash Sapling.

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
  1. tk = sk * G  (I control this address)
  2. address ∉ sanctions_merkle_tree  (non-membership proof)
  3. sanctions_root = <public value>  (against latest published root)"
```

Public inputs: `[sanctions_root, tk_commitment]`  
Circuit: Poseidon-based Merkle non-membership proof + key ownership check  
Proof size: ~200 bytes (Groth16)

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
Public inputs: `commitment, value_commitment`

Constraints: ~2,000 gates. Proving time: ~200ms.

### 5.4 Unshield Circuit (shielded → public)

**File:** `circuits/unshield/unshield.circom`

Private inputs: `value, asset_id, rho, rcm, nk, path[32], path_index[32]`  
Public inputs: `anchor, nullifier, value, asset_id, recipient` (value and asset are revealed)

Constraints: ~6,000 gates. Proving time: ~600ms.

### 5.5 Swap Fairness Circuit

**File:** `circuits/swap/swap_fairness.circom`

Proves that a swap execution honoured the user's committed slippage tolerance.

Private inputs: `intent_nonce, asset_in, asset_out, amount_in, max_slippage_bps`  
Public inputs: `intent_commitment, amount_out, execution_price_bps`

```
intent_commitment === Poseidon2(
  Poseidon2(asset_in, asset_out),
  Poseidon2(amount_in || max_slippage_bps, intent_nonce)
)

execution_price_bps >= (10000 - max_slippage_bps) * amount_in / amount_out
```

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

The contract interfaces in this section define the intended protocol surface. The repository currently includes soft PoC contract code only. Existing Soroban contracts must be reviewed, improved, and completed before they can be treated as final CT-20, viewing-key, swap, or governance implementations.

### 6.1 CT-20 Token Contract

**File:** `contracts/ct20/src/lib.rs`

```rust
pub trait CT20Interface {

    /// Deposit a public SEP-41 token amount and receive a shielded note.
    /// The note commitment is added to the Merkle tree.
    /// Emits: NoteCommitmentEvent { index, commitment, encrypted_note }
    fn shield(
        env:            Env,
        from:           Address,     // must authorize
        asset:          Address,     // SEP-41 token contract
        amount:         i128,
        commitment:     BytesN<32>,  // note commitment
        encrypted_note: Bytes,       // 152-byte ciphertext bundle
        shield_proof:   Bytes,       // Groth16 proof (~200 bytes)
        shield_pub:     ShieldPublicInputs,
    ) -> u32;                        // leaf index in Merkle tree

    /// Transfer between shielded notes (2-in/2-out or 4-in/4-out).
    /// Spends nullifiers, adds output commitments to tree.
    /// Emits: NullifierEvent { nullifier } × N_in
    /// Emits: NoteCommitmentEvent { index, commitment, encrypted_note } × N_out
    fn transfer(
        env:             Env,
        nullifiers:      Vec<BytesN<32>>,
        commitments:     Vec<BytesN<32>>,
        encrypted_notes: Vec<Bytes>,
        proof:           Bytes,
        pub_inputs:      TransferPublicInputs,
    ) -> Vec<u32>;                   // leaf indices of output commitments

    /// Reveal a note and withdraw to a public address.
    /// Emits: NullifierEvent, UnshieldEvent { to, amount, asset }
    fn unshield(
        env:        Env,
        nullifier:  BytesN<32>,
        to:         Address,
        proof:      Bytes,
        pub_inputs: UnshieldPublicInputs,
    );

    /// Read the current Merkle root.
    fn merkle_root(env: Env) -> BytesN<32>;

    /// Check if a nullifier has been spent.
    fn is_spent(env: Env, nullifier: BytesN<32>) -> bool;

    /// Return total shielded supply for an asset.
    fn shielded_supply(env: Env, asset: Address) -> i128;

    /// Emergency pause — governance only.
    fn pause(env: Env);
    fn unpause(env: Env);
}
```

**Verification logic (transfer):**

```rust
fn verify_transfer_proof(
    env: &Env,
    proof: &Bytes,
    pub_inputs: &TransferPublicInputs,
) -> bool {
    // 1. Deserialize proof into (A: G1, B: G2, C: G1)
    // 2. Load verifying key from contract storage
    // 3. Compute vk_x = Σ pub_inputs[i] * vk_IC[i]  (BN254 G1 mul + add)
    // 4. Call bn254_multi_pairing_check:
    //    e(A, B) == e(alpha, beta) * e(vk_x, gamma) * e(C, delta)
    // 5. Return result
}
```

**Merkle tree insertion (incremental):**

```rust
fn insert_commitment(env: &Env, cm: BytesN<32>) -> u32 {
    let index = env.storage().instance().get::<_, u32>(&StorageKey::NextLeafIndex)
                   .unwrap_or(0);
    // Store leaf
    env.storage().persistent().set(&StorageKey::MerkleLeaf(index), &cm);
    // Update path from leaf to root using Poseidon2
    let root = recompute_root(env, index, cm);
    env.storage().instance().set(&StorageKey::MerkleRoot, &root);
    env.storage().instance().set(&StorageKey::NextLeafIndex, &(index + 1));
    index
}
```

### 6.2 Viewing Key Registry Contract

**File:** `contracts/viewing_keys/src/lib.rs`

```rust
pub trait ViewingKeyRegistry {

    /// Register a viewing key commitment on-chain.
    /// Allows auditors to verify they hold a valid key for an address.
    /// vk_commitment = Poseidon2(vk, address_diversifier)
    fn register(
        env:           Env,
        owner:         Address,      // must authorize
        vk_commitment: BytesN<32>,
        birthday:      u32,          // ledger number for sync start
    );

    /// Publish a compliance proof for a specific audit request.
    /// proof: ZK proof that address ∉ sanctions_merkle_tree
    fn publish_compliance_proof(
        env:             Env,
        owner:           Address,
        sanctions_root:  BytesN<32>,
        proof:           Bytes,
        pub_inputs:      CompliancePublicInputs,
    );

    /// Retrieve the latest compliance proof for an address.
    fn get_compliance_proof(
        env:     Env,
        owner:   Address,
    ) -> Option<ComplianceRecord>;
}
```

### 6.3 Shielded Swap Contract

**File:** `contracts/swap/src/lib.rs`

```rust
pub trait ShieldedSwap {

    /// Commit to a swap intent. Locks the input shielded note nullifier.
    /// intent_commitment = Poseidon2(Poseidon2(asset_in, asset_out),
    ///                               Poseidon2(amount_in || max_slippage, nonce))
    fn commit_swap(
        env:               Env,
        nullifier_in:      BytesN<32>,   // spends input note
        intent_commitment: BytesN<32>,
        commitment_proof:  Bytes,        // proves nullifier is valid
        expiry_ledger:     u32,          // swap expires if not executed
    ) -> BytesN<32>;                     // swap_id

    /// Relayer calls this to execute the swap via Stellar DEX.
    /// Reveals intent and executes the actual token swap.
    fn execute_swap(
        env:        Env,
        swap_id:    BytesN<32>,
        asset_in:   Address,
        asset_out:  Address,
        amount_in:  i128,
        amount_out: i128,      // actual amount received from DEX
        relayer:    Address,   // receives relayer fee
    );

    /// User confirms execution and receives output as shielded note.
    fn reveal_and_claim(
        env:               Env,
        swap_id:           BytesN<32>,
        intent_nonce:      BytesN<32>,
        max_slippage_bps:  u32,
        out_commitment:    BytesN<32>,
        encrypted_note:    Bytes,
        fairness_proof:    Bytes,
        fairness_pub:      SwapFairnessPublicInputs,
    ) -> u32;                  // output note leaf index

    /// Reclaim input if swap was not executed before expiry.
    fn cancel_swap(
        env:     Env,
        swap_id: BytesN<32>,
        proof:   Bytes,        // proves ownership of original note
    );
}
```

### 6.4 Governance Contract

**File:** `contracts/governance/src/lib.rs`

Manages verifying key upgrades, pause authority, and relayer whitelist.

```rust
pub trait ZKELLAGovernance {
    fn update_verifying_key(env: Env, admin: Address, circuit_id: u8, new_vk: Bytes);
    fn set_relayer(env: Env, admin: Address, relayer: Address, approved: bool);
    fn transfer_admin(env: Env, current_admin: Address, new_admin: Address);
}
```

Verifying key update has a 7-day timelock enforced at the contract level.

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

```
                ┌─────────────────────────────┐
                │       zkella-indexer         │
                │                              │
  Stellar RPC ──► Event Listener               │
                │   └── Soroban event stream   │
                │                              │
                │  Note Store (PostgreSQL)      │
                │   ├── encrypted_notes table  │
                │   ├── nullifiers table        │
                │   └── merkle_leaves table    │
                │                              │
                │  REST API                    │◄── zkella-sdk
                │   ├── GET /notes             │
                │   ├── GET /merkle/path/{idx} │
                │   ├── GET /nullifiers/batch  │
                │   └── GET /root              │
                └─────────────────────────────┘
```

### 8.3 Indexer API Specification

**Base URL:** `https://indexer.zkella.io/v1` (also self-hostable)

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

### 9.1 Trust Model

The shielded swap uses a **weak privacy model**:
- Amount hidden from on-chain passive observers ✓
- Amount revealed to the designated relayer ✗ (relayer must see intent to execute)

This is sufficient for: front-running protection, competitive intelligence protection, and basic financial privacy. A fully private model (hidden from relayer) would require a TEE relayer or private mempool, which is out of scope for v1.

### 9.2 Relayer Model

Relayers are permissioned via the governance contract. They:
- Monitor committed swap intents on-chain
- Off-chain receive encrypted swap parameters from users (direct P2P via relay server)
- Execute swaps via Stellar DEX `PathPaymentStrictReceive` or `ManageSellOffer`
- Earn a relayer fee (configurable, deducted from output amount)

Multiple relayers compete for execution. Users can set a relayer preference or use any available relayer.

### 9.3 Swap Flow Detail

```
Step 1 — User: Generate intent off-chain
  intent = { asset_in, asset_out, amount_in, max_slippage_bps, nonce }
  intent_commitment = Poseidon2(Poseidon2(asset_in, asset_out),
                                 Poseidon2(amount_in || max_slippage_bps, nonce))

Step 2 — User: Submit commit_swap transaction
  - Spends input note nullifier (locks the funds)
  - Publishes intent_commitment on-chain
  - Sends encrypted intent to relayer relay server (off-chain)
  - Sets expiry_ledger = current + 720  (~1 hour)

Step 3 — Relayer: Execute swap
  - Receives and decrypts intent from relay server
  - Calls execute_swap(swap_id, asset_in, asset_out, amount_in, amount_out)
  - Executes actual Stellar DEX path payment
  - amount_out recorded on-chain

Step 4 — User: Claim output as shielded note
  - Generates fairness_proof: "execution_price >= (1 - max_slippage) * reference_price"
  - Constructs output note for amount_out minus relayer_fee
  - Calls reveal_and_claim(swap_id, nonce, max_slippage, out_commitment, encrypted_note, proof)
  - Output note added to Merkle tree

Step 5 (fallback) — User: Cancel if expired
  - If relayer did not execute by expiry_ledger
  - Calls cancel_swap(swap_id, proof)
  - Input funds returned as new shielded note
```

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

```
zkella-sdk/
├── src/
│   ├── keys/          # Key generation and derivation
│   ├── notes/         # Note construction, commitment, encryption
│   ├── circuits/      # WASM proof generation (snarkjs)
│   │   ├── shield.wasm
│   │   ├── transfer_2in2out.wasm
│   │   ├── transfer_4in4out.wasm
│   │   ├── unshield.wasm
│   │   └── swap_fairness.wasm
│   ├── contracts/     # Soroban contract bindings (generated)
│   ├── indexer/       # Indexer client
│   ├── wallet/        # High-level wallet abstraction
│   └── compliance/    # Viewing key export, compliance proofs
```

### 11.2 Core API

```typescript
// Key management
const keys = ZKELLAKeys.fromSeed(seed)
// keys.spendingKey, keys.viewingKey, keys.transmissionKey

// Wallet
const wallet = new ZKELLAWallet({
  keys,
  indexerUrl: 'https://indexer.zkella.io/v1',
  network: 'mainnet',
  sorobanRpc: 'https://soroban-rpc.stellar.org'
})

await wallet.sync()  // fetch and decrypt all notes from indexer

const balance = await wallet.balance(USDC_CONTRACT)
// { shielded: 1000n, unshielded: 500n }

// Shield
const tx = await wallet.shield({
  asset: USDC_CONTRACT,
  amount: 100_000_000n,  // 100 USDC (7 decimals)
})
await tx.submit()

// Transfer
const tx = await wallet.transfer({
  to: 'zkella1abc...xyz',  // recipient shielded address
  asset: USDC_CONTRACT,
  amount: 50_000_000n,
  memo: 'optional plaintext memo',
})
await tx.submit()

// Unshield
const tx = await wallet.unshield({
  asset: USDC_CONTRACT,
  amount: 25_000_000n,
  to: 'GABCD...WXYZ',    // public Stellar address
})
await tx.submit()

// Compliance
const vkExport = wallet.exportViewingKey()
const proof = await wallet.generateComplianceProof(sanctionsListUrl)
```

### 11.3 Note Selection Strategy

Notes are selected using a **greedy smallest-first** algorithm to minimize fragmentation:

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
| Observer learns transfer amount | Amounts inside Pedersen commitments, never on-chain in plaintext |
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

---

## 13. Performance and Resource Budget

### 13.1 Client-Side Proving Times (snarkjs WASM, modern desktop browser)

| Circuit | Gates | Proving Time | Proof Size |
|---|---|---|---|
| Shield | ~2,000 | ~200ms | 192 bytes |
| Unshield | ~6,000 | ~600ms | 192 bytes |
| Transfer 2-in/2-out | ~15,450 | ~2.0s | 192 bytes |
| Transfer 4-in/4-out | ~28,000 | ~4.5s | 192 bytes |
| Swap fairness | ~3,500 | ~400ms | 192 bytes |
| Sanctions non-membership | ~9,000 | ~1.0s | 192 bytes |

All Groth16 proofs are 192 bytes regardless of circuit size.

### 13.2 Soroban On-Chain Verification Cost

| Operation | Soroban Instructions (estimate) |
|---|---|
| Deserialize proof + public inputs | ~50,000 |
| Compute vk_x (N public inputs × G1 mul + add) | ~200,000–400,000 |
| bn254_multi_pairing_check (4 pairs) | Native — does not consume instruction budget proportionally |
| Merkle root update (32 levels × Poseidon2) | ~160,000 |
| Nullifier storage write (×N) | ~50,000 per nullifier |

Total estimated per-transfer transaction cost: ~1–3 XLM at current fee levels (dominated by ledger entry writes, not compute).

### 13.3 Indexer Resource Requirements

| Resource | Minimum | Recommended |
|---|---|---|
| CPU | 2 cores | 4 cores |
| RAM | 2 GB | 8 GB |
| Storage (year 1 at 10K tx/day) | ~20 GB | 100 GB SSD |
| Bandwidth | 10 Mbps | 100 Mbps |
| Database | PostgreSQL 14+ | PostgreSQL 16 + read replica |

---

## 14. Deployment Plan

The deployment plan starts from the current soft PoC baseline. Before any final release, all existing contracts and SDK modules must move through review, implementation completion, resource profiling, and hardening. The current PoC contracts should not be promoted directly to production.

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

```
ZKELLA/
├── circuits/
│   ├── transfer_2in2out/
│   │   ├── transfer.circom
│   │   ├── transfer.r1cs        # generated
│   │   ├── transfer.wasm        # generated
│   │   └── transfer.zkey        # after ceremony
│   ├── transfer_4in4out/
│   ├── shield/
│   ├── unshield/
│   ├── swap/
│   └── compliance/
├── contracts/
│   ├── ct20/
│   ├── viewing_keys/
│   ├── swap/
│   └── governance/
├── indexer/                      # Go or Rust service
├── sdk/                          # TypeScript npm package
├── app/                          # React reference wallet
└── docs/
    ├── TECHNICAL_SPEC.md         # this document
    ├── CIRCUIT_SPEC.md           # detailed constraint listings
    └── INTEGRATION_GUIDE.md     # for third-party builders
```

---

*ZKELLA Protocol — Technical Specification v0.1.0*
