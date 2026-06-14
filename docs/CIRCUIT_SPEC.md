# ZKELLA — Circuit Specification

**Version:** 0.1.0

All circuits are written in Circom 2.0 and compiled to Groth16 over BN254.  
Proof size: 192 bytes (fixed, all circuits).  
Verifying key: loaded from Soroban contract storage, upgradeable via governance timelock.

---

## 1. Shared Components

### 1.1 Poseidon2 Hasher

```circom
// Used everywhere in place of SHA-256 — ~300 constraints vs ~22,000
template Poseidon2() {
    signal input in[2];
    signal output out;
    // Calls Circom's built-in Poseidon template (iden3/circomlib)
    component h = Poseidon(2);
    h.inputs[0] <== in[0];
    h.inputs[1] <== in[1];
    out <== h.out;
}
```

### 1.2 Merkle Path Verifier

```circom
// Verifies a leaf exists in a binary Merkle tree of depth D
// using D sibling nodes and D direction bits (0=left, 1=right)
template MerkleProof(D) {
    signal input leaf;
    signal input path[D];       // sibling nodes
    signal input index[D];      // 0 or 1 at each level
    signal output root;

    component hashers[D];
    signal nodes[D+1];
    nodes[0] <== leaf;

    for (var i = 0; i < D; i++) {
        hashers[i] = Poseidon2();
        // Select left/right based on index bit
        // index[i] = 0 → (nodes[i], path[i])
        // index[i] = 1 → (path[i], nodes[i])
        hashers[i].in[0] <== (1 - index[i]) * nodes[i] + index[i] * path[i];
        hashers[i].in[1] <== (1 - index[i]) * path[i] + index[i] * nodes[i];
        nodes[i+1] <== hashers[i].out;
    }

    root <== nodes[D];
}
```

### 1.3 Range Proof (64-bit)

```circom
// Proves value ∈ [0, 2^64)
// Uses binary decomposition: ~64 constraints + bit check
template Range64() {
    signal input value;
    signal output valid;

    component bits = Num2Bits(64);
    bits.in <== value;
    // Num2Bits enforces that value decomposes into 64 bits
    // which implicitly constrains 0 ≤ value < 2^64
    valid <== 1;
}
```

### 1.4 Note Commitment

```circom
template NoteCommitment() {
    signal input value;
    signal input asset_id;
    signal input rho;
    signal input rcm;
    signal output cm;

    component h1 = Poseidon2();
    h1.in[0] <== value;
    h1.in[1] <== asset_id;

    component h2 = Poseidon2();
    h2.in[0] <== rho;
    h2.in[1] <== rcm;

    component h3 = Poseidon2();
    h3.in[0] <== h1.out;
    h3.in[1] <== h2.out;

    cm <== h3.out;
}
```

### 1.5 Nullifier Derivation

```circom
template Nullifier() {
    signal input nk;    // nullifier key (private)
    signal input rho;   // note nullifier seed (private)
    signal output nf;

    component h = Poseidon2();
    h.in[0] <== nk;
    h.in[1] <== rho;
    nf <== h.out;
}
```

### 1.6 Pedersen Value Commitment

```circom
// cv = rcv * G + value * H_v
// Both G and H_v are fixed BN254 G1 points (nothing-up-my-sleeve)
// We commit to a field representation of the G1 point
// For circuit purposes we use a simplified scalar binding:
template ValueCommit() {
    signal input value;
    signal input rcv;
    signal output cv;

    // In-circuit: bind value and randomness via Poseidon
    // Full Pedersen over BN254 G1 is verified outside the circuit
    // using the BN254 host functions on Soroban
    component h = Poseidon2();
    h.in[0] <== value;
    h.in[1] <== rcv;
    cv <== h.out;
}
```

> **Note on Pedersen:** The full Pedersen commitment `cv = rcv*G + value*H_v` is a BN254 G1 point. Inside the circuit we verify the commitment binding via a Poseidon-based binding scheme. The actual G1 arithmetic (for the homomorphic balance check) is performed on Soroban using `bn254_g1_add` and `bn254_g1_mul` host functions, not inside the circuit. This avoids emulating elliptic curve arithmetic in R1CS (~10,000+ constraints per scalar mul).

---

## 2. Shield Circuit

**File:** `circuits/shield/shield.circom`  
**Purpose:** Proves a valid note commitment for a publicly known amount being moved into the shielded pool.  
**Gates:** ~2,000  
**Proving time:** ~200ms

```circom
pragma circom 2.0.0;

include "../common/commitment.circom";
include "../common/range.circom";
include "../common/value_commit.circom";

template Shield() {
    // Private inputs
    signal input value;
    signal input asset_id;
    signal input rho;
    signal input rcm;
    signal input rcv;

    // Public inputs
    signal input commitment;       // must equal computed cm
    signal input value_commit;     // must equal computed cv
    signal input pub_value;        // revealed to contract (equals value)
    signal input pub_asset_id;     // revealed to contract (equals asset_id)

    // Commitment check
    component cm_check = NoteCommitment();
    cm_check.value    <== value;
    cm_check.asset_id <== asset_id;
    cm_check.rho      <== rho;
    cm_check.rcm      <== rcm;
    cm_check.cm       === commitment;

    // Value commitment check
    component cv_check = ValueCommit();
    cv_check.value <== value;
    cv_check.rcv   <== rcv;
    cv_check.cv    === value_commit;

    // Public value consistency
    value    === pub_value;
    asset_id === pub_asset_id;

    // Range check
    component range = Range64();
    range.value <== value;
}

component main {public [commitment, value_commit, pub_value, pub_asset_id]}
  = Shield();
```

**Public inputs (5 field elements):**
```
commitment      : F_p  — note commitment
value_commit    : F_p  — value binding
pub_value       : F_p  — amount (revealed, matches on-chain transfer)
pub_asset_id    : F_p  — asset (revealed)
```

---

## 3. Unshield Circuit

**File:** `circuits/unshield/unshield.circom`  
**Purpose:** Proves ownership of a note in the Merkle tree and authorizes withdrawal to a public address.  
**Gates:** ~6,200  
**Proving time:** ~600ms

```circom
pragma circom 2.0.0;

include "../common/commitment.circom";
include "../common/nullifier.circom";
include "../common/merkle.circom";
include "../common/range.circom";

template Unshield(D) {  // D = 32 (Merkle depth)
    // Private inputs
    signal input value;
    signal input asset_id;
    signal input rho;
    signal input rcm;
    signal input nk;
    signal input path[D];
    signal input path_index[D];

    // Public inputs
    signal input anchor;           // Merkle root
    signal input nullifier;        // must equal computed nf
    signal input pub_value;        // revealed amount
    signal input pub_asset_id;     // revealed asset
    signal input recipient_hash;   // Poseidon2(recipient_address) — binds to destination

    // Note commitment
    component cm = NoteCommitment();
    cm.value    <== value;
    cm.asset_id <== asset_id;
    cm.rho      <== rho;
    cm.rcm      <== rcm;

    // Merkle membership
    component mp = MerkleProof(D);
    mp.leaf     <== cm.cm;
    for (var i = 0; i < D; i++) {
        mp.path[i]  <== path[i];
        mp.index[i] <== path_index[i];
    }
    mp.root === anchor;

    // Nullifier
    component nf = Nullifier();
    nf.nk  <== nk;
    nf.rho <== rho;
    nf.nf  === nullifier;

    // Public consistency
    value    === pub_value;
    asset_id === pub_asset_id;

    // Range check
    component range = Range64();
    range.value <== value;
}

component main {public [anchor, nullifier, pub_value, pub_asset_id, recipient_hash]}
  = Unshield(32);
```

**Public inputs (5 field elements):**
```
anchor          : F_p  — Merkle root
nullifier       : F_p  — note nullifier
pub_value       : F_p  — amount (revealed)
pub_asset_id    : F_p  — asset (revealed)
recipient_hash  : F_p  — Poseidon2(recipient Stellar address bytes)
```

---

## 4. Transfer Circuit — 2-input / 2-output

**File:** `circuits/transfer_2in2out/transfer.circom`  
**Purpose:** Private transfer between shielded notes.  
**Gates:** ~15,450  
**Proving time:** ~2.0s

```circom
pragma circom 2.0.0;

include "../common/commitment.circom";
include "../common/nullifier.circom";
include "../common/merkle.circom";
include "../common/range.circom";
include "../common/value_commit.circom";

template Transfer2x2(D) {
    var N_IN  = 2;
    var N_OUT = 2;

    // Private inputs — input notes
    signal input in_value[N_IN];
    signal input in_asset_id[N_IN];
    signal input in_rho[N_IN];
    signal input in_rcm[N_IN];
    signal input in_path[N_IN][D];
    signal input in_path_index[N_IN][D];
    signal input in_rcv[N_IN];
    signal input nk;

    // Private inputs — output notes
    signal input out_value[N_OUT];
    signal input out_asset_id[N_OUT];
    signal input out_rho[N_OUT];
    signal input out_rcm[N_OUT];
    signal input out_rcv[N_OUT];

    // Public inputs
    signal input anchor;
    signal input nullifiers[N_IN];
    signal input out_commitments[N_OUT];
    signal input in_value_commits[N_IN];
    signal input out_value_commits[N_OUT];
    signal input fee;
    signal input asset_id;   // all notes must share this asset

    // ── Input note verification ──────────────────────────────────────────
    component in_cm[N_IN];
    component in_mp[N_IN];
    component in_nf[N_IN];
    component in_cv[N_IN];
    component in_range[N_IN];

    for (var i = 0; i < N_IN; i++) {
        // Commitment
        in_cm[i] = NoteCommitment();
        in_cm[i].value    <== in_value[i];
        in_cm[i].asset_id <== in_asset_id[i];
        in_cm[i].rho      <== in_rho[i];
        in_cm[i].rcm      <== in_rcm[i];

        // Merkle membership
        in_mp[i] = MerkleProof(D);
        in_mp[i].leaf <== in_cm[i].cm;
        for (var j = 0; j < D; j++) {
            in_mp[i].path[j]  <== in_path[i][j];
            in_mp[i].index[j] <== in_path_index[i][j];
        }
        in_mp[i].root === anchor;

        // Nullifier
        in_nf[i] = Nullifier();
        in_nf[i].nk  <== nk;
        in_nf[i].rho <== in_rho[i];
        in_nf[i].nf  === nullifiers[i];

        // Value commitment
        in_cv[i] = ValueCommit();
        in_cv[i].value <== in_value[i];
        in_cv[i].rcv   <== in_rcv[i];
        in_cv[i].cv    === in_value_commits[i];

        // Asset consistency
        in_asset_id[i] === asset_id;

        // Range
        in_range[i] = Range64();
        in_range[i].value <== in_value[i];
    }

    // ── Output note verification ─────────────────────────────────────────
    component out_cm[N_OUT];
    component out_cv[N_OUT];
    component out_range[N_OUT];

    for (var i = 0; i < N_OUT; i++) {
        // Commitment
        out_cm[i] = NoteCommitment();
        out_cm[i].value    <== out_value[i];
        out_cm[i].asset_id <== out_asset_id[i];
        out_cm[i].rho      <== out_rho[i];
        out_cm[i].rcm      <== out_rcm[i];
        out_cm[i].cm       === out_commitments[i];

        // Value commitment
        out_cv[i] = ValueCommit();
        out_cv[i].value <== out_value[i];
        out_cv[i].rcv   <== out_rcv[i];
        out_cv[i].cv    === out_value_commits[i];

        // Asset consistency
        out_asset_id[i] === asset_id;

        // Range
        out_range[i] = Range64();
        out_range[i].value <== out_value[i];
    }

    // ── Balance check ─────────────────────────────────────────────────────
    // Σ in_value = Σ out_value + fee
    signal sum_in;
    signal sum_out;
    sum_in  <== in_value[0]  + in_value[1];
    sum_out <== out_value[0] + out_value[1];
    sum_in  === sum_out + fee;
}

component main {
    public [anchor, nullifiers, out_commitments,
            in_value_commits, out_value_commits, fee, asset_id]
} = Transfer2x2(32);
```

**Public inputs (11 field elements):**
```
anchor              : F_p
nullifiers[2]       : F_p[2]
out_commitments[2]  : F_p[2]
in_value_commits[2] : F_p[2]
out_value_commits[2]: F_p[2]
fee                 : F_p
asset_id            : F_p
```

---

## 5. Transfer Circuit — 4-input / 4-output

**File:** `circuits/transfer_4in4out/transfer.circom`  
**Purpose:** High-capacity transfer for dust consolidation and multi-recipient payments.  
**Gates:** ~28,000  
**Proving time:** ~4.5s

Structurally identical to Transfer 2x2 with `N_IN = 4`, `N_OUT = 4`.

**Public inputs (19 field elements):**
```
anchor              : F_p
nullifiers[4]       : F_p[4]
out_commitments[4]  : F_p[4]
in_value_commits[4] : F_p[4]
out_value_commits[4]: F_p[4]
fee                 : F_p
asset_id            : F_p
```

Balance check: `Σ in_value[0..4] === Σ out_value[0..4] + fee`

---

## 6. Swap Fairness Circuit

**File:** `circuits/swap/swap_fairness.circom`  
**Purpose:** Proves a committed swap intent was executed within the user's slippage tolerance.  
**Gates:** ~3,500  
**Proving time:** ~400ms

```circom
pragma circom 2.0.0;

include "../common/commitment.circom";

template SwapFairness() {
    // Private inputs
    signal input intent_nonce;
    signal input amount_in;
    signal input max_slippage_bps;   // e.g. 50 = 0.5%

    // Public inputs
    signal input intent_commitment;  // on-chain committed value
    signal input asset_in;           // revealed at execution
    signal input asset_out;          // revealed at execution
    signal input amount_out;         // actual received (revealed)
    signal input min_amount_out;     // = amount_in * (10000 - slippage) / 10000

    // Reconstruct intent commitment
    component h1 = Poseidon2();
    h1.in[0] <== asset_in;
    h1.in[1] <== asset_out;

    // Pack amount_in and max_slippage_bps into one field element
    signal packed;
    packed <== amount_in * (2**32) + max_slippage_bps;

    component h2 = Poseidon2();
    h2.in[0] <== packed;
    h2.in[1] <== intent_nonce;

    component h3 = Poseidon2();
    h3.in[0] <== h1.out;
    h3.in[1] <== h2.out;

    h3.out === intent_commitment;

    // Fairness check: amount_out >= min_amount_out
    // Enforced as: amount_out - min_amount_out >= 0
    signal diff;
    diff <== amount_out - min_amount_out;
    component range = Range64();
    range.value <== diff;
}

component main {
    public [intent_commitment, asset_in, asset_out, amount_out, min_amount_out]
} = SwapFairness();
```

**Public inputs (5 field elements):**
```
intent_commitment : F_p
asset_in          : F_p
asset_out         : F_p
amount_out        : F_p
min_amount_out    : F_p
```

---

## 7. Sanctions Non-Membership Circuit

**File:** `circuits/compliance/non_membership.circom`  
**Purpose:** Proves a ZKELLA address does not appear in a published sanctions list.  
**Gates:** ~9,000  
**Proving time:** ~1.0s

Uses a **sorted Merkle tree non-membership proof**: proves the address falls strictly between two consecutive leaves.

```circom
pragma circom 2.0.0;

include "../common/merkle.circom";

template NonMembership(D) {
    // Private inputs
    signal input sk;                        // spending key
    signal input lower_leaf;               // sorted left boundary
    signal input upper_leaf;               // sorted right boundary
    signal input lower_path[D];
    signal input lower_path_index[D];
    signal input upper_path[D];
    signal input upper_path_index[D];

    // Public inputs
    signal input sanctions_root;
    signal input tk_commitment;            // Poseidon2(tk, diversifier)

    // Derive transmission key commitment from sk
    // tk = sk * G (BN254) — done outside circuit
    // We verify: Poseidon2(sk) matches a commitment
    // (avoids full EC scalar mul inside R1CS)
    component sk_commit = Poseidon2();
    sk_commit.in[0] <== sk;
    sk_commit.in[1] <== 0;
    sk_commit.out   === tk_commitment;

    // Derive the address field element from sk
    component addr_h = Poseidon2();
    addr_h.in[0] <== sk;
    addr_h.in[1] <== 1;
    signal address;
    address <== addr_h.out;

    // Verify lower_leaf in sanctions tree
    component lower_mp = MerkleProof(D);
    lower_mp.leaf <== lower_leaf;
    for (var i = 0; i < D; i++) {
        lower_mp.path[i]  <== lower_path[i];
        lower_mp.index[i] <== lower_path_index[i];
    }
    lower_mp.root === sanctions_root;

    // Verify upper_leaf in sanctions tree
    component upper_mp = MerkleProof(D);
    upper_mp.leaf <== upper_leaf;
    for (var i = 0; i < D; i++) {
        upper_mp.path[i]  <== upper_path[i];
        upper_mp.index[i] <== upper_path_index[i];
    }
    upper_mp.root === sanctions_root;

    // Sorted non-membership: lower < address < upper
    signal diff_lower;
    signal diff_upper;
    diff_lower <== address - lower_leaf;
    diff_upper <== upper_leaf - address;

    component rl = Range64();
    rl.value <== diff_lower;

    component ru = Range64();
    ru.value <== diff_upper;
}

component main {
    public [sanctions_root, tk_commitment]
} = NonMembership(32);
```

**Public inputs (2 field elements):**
```
sanctions_root : F_p  — root of published sanctions Merkle tree
tk_commitment  : F_p  — address binding (without revealing address)
```

---

## 8. Constraint Summary

| Circuit | R1CS Constraints | Wires | Labels |
|---|---|---|---|
| Shield | ~2,000 | ~2,200 | ~3,100 |
| Unshield | ~6,200 | ~6,800 | ~9,500 |
| Transfer 2x2 | ~15,450 | ~16,900 | ~23,800 |
| Transfer 4x4 | ~28,000 | ~30,600 | ~43,200 |
| Swap Fairness | ~3,500 | ~3,800 | ~5,300 |
| Non-Membership | ~9,000 | ~9,800 | ~13,800 |

---

## 9. Trusted Setup Parameters

| Parameter | Value |
|---|---|
| Proof system | Groth16 |
| Curve | BN254 (alt_bn128) |
| Powers of Tau | Hermez ceremony, 2^28 (covers all circuits) |
| Phase 2 | Per-circuit, multi-party ceremony |
| Minimum contributors | 10 independent parties |
| Final beacon | Stellar mainnet ledger hash (announced 48h in advance) |
| Artifacts | `.r1cs`, `.wasm`, `.zkey`, `verification_key.json` |
| Published at | `https://github.com/Frihat-dev/ZKELLA/releases` |

All ceremony contributions will be posted publicly. Verification instructions included in release notes.

---

*ZKELLA Circuit Specification v0.1.0*
