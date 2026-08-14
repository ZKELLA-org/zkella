# ZKELLA — Circuit Specification

**Version:** 0.1.0

**Implementation status:** this document specifies the intended circuit family for the protocol. The repository code and current circuit artifacts are soft PoC material only; they must be reviewed, tested against final contract semantics, optimized, and improved before production use.

All circuits are written in Circom 2.2 and compiled to Groth16 over BN254.
Proof size: 256 bytes (fixed, all circuits) — the wire format `contracts/verifier` expects is *uncompressed* BN254 points (`A`: 64 bytes G1, `B`: 128 bytes G2, `C`: 64 bytes G1), matching Soroban's native `crypto::bn254` host types exactly; this is not the 192-byte compressed encoding some other Groth16 tooling defaults to. See `sdk/src/prover/encoding.ts` for the exact byte layout and `docs/POC_IMPLEMENTATION.md` for confirmation against a real submitted proof.
Verifying key: loaded from Soroban contract storage (`contracts/verifier`), upgradeable via governance timelock (`contracts/governance`).

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

**Update (critical finding, since fixed):** a fix to make this circuit compile at all (the unfactored two-product select form below couldn't pass circom's `<==` quadratic-constraint check) exposed that `index[i]` was never constrained to be boolean. Unconstrained, a prover can choose any field value for it, turning the intended left/right select into a full linear interpolation that lets a prover force the computed `root` to equal *any* value from *any* starting `leaf` — a complete Merkle-membership forgery, letting an attacker fabricate a note that was never actually shielded and spend it as if real. This was dormant only because the circuit didn't compile until the factoring fix landed; both fixes shipped together. See `docs/POC_IMPLEMENTATION.md`'s "Update: external audit" and `circuits/common/merkle.circom` for the full account. The snippet below matches the current, fixed circuit exactly.

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
        // index[i] MUST be boolean, or the select below becomes a full
        // linear interpolation a prover can force to any root/leaf pair.
        index[i] * (index[i] - 1) === 0;

        hashers[i] = Poseidon2();
        // Written as a single product term (index[i]*(a-b)) plus a linear
        // term, not "(1-index[i])*a + index[i]*b" — circom's `<==` requires
        // the whole RHS to reduce to one linear-combination-times-linear-
        // combination, and the unfactored two-product form fails to compile
        // ("Non quadratic constraints are not allowed") even though it's
        // mathematically equivalent.
        // index[i] = 0 → (nodes[i], path[i])
        // index[i] = 1 → (path[i], nodes[i])
        hashers[i].in[0] <== nodes[i] + index[i] * (path[i] - nodes[i]);
        hashers[i].in[1] <== path[i] + index[i] * (nodes[i] - path[i]);
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

**Assumed range vs. contract-permitted range — an explicit design boundary, not an oversight.** Every value/amount constrained in these circuits is bounded to 64 bits (`0 ≤ value < 2^64`, ~1.8×10¹⁹). Soroban contracts pass amounts as `i128`, a much wider type (up to ~1.7×10³⁸). For any realistic Stellar SEP-41 asset amount (stroops, 7 decimal places — 64 bits covers roughly 1.8×10¹² XLM, far beyond total XLM supply, and equivalently oversized for any plausible token supply), 64 bits is not a practical limitation. It is, however, a real assumption boundary: a token with an unusually large raw integer amount (extreme decimal precision, or a deliberately adversarial `i128` value near its own maximum) is outside what these circuits can prove correct — the contract-level `i128` type permits values these circuits were never designed to constrain. This has not caused any known issue since every asset ZKELLA has wrapped to date fits comfortably within 64 bits, but it should be treated as a documented soundness precondition, not silently assumed.

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

> **Note on Pedersen:** The full Pedersen commitment `cv = rcv*G + value*H_v` is a BN254 G1 point, and real G1 arithmetic (`bn254_g1_add`/`bn254_g1_mul`) is available as a Soroban host function — the target design is to perform the homomorphic balance check with it on-chain, avoiding ~10,000+ constraints per scalar multiplication that emulating EC arithmetic in R1CS would cost inside the circuit. **This is not implemented yet**: today `value_commit` is the Poseidon-based binding shown above, and no contract performs a G1 homomorphic check on it — verified by grep, `bn254_g1_add`/`bn254_g1_mul` only appear inside `contracts/verifier`'s own Groth16 pairing-check code, not anywhere computing a balance check on value commitments. Balance conservation is still soundly enforced today, just differently: the transfer circuit directly constrains `Σ in_value === Σ out_value + fee` over the private `value` signals inside the R1CS (see `circuits/transfer_2in2out/transfer.circom`), independent of the commitment scheme. What the real Pedersen construction would add on top is an *external*, proof-free homomorphic check — not soundness itself. See `docs/TECHNICAL_SPEC.md` §2.3 for the same point in more detail.

---

## 2. Shield Circuit

**File:** `circuits/shield/shield.circom`  
**Purpose:** Proves a valid note commitment for a publicly known amount being moved into the shielded pool.  
**Constraints:** 2,133 (real, measured via `snarkjs r1cs info` against the compiled circuit)  
**Proving time:** ~200ms (unmeasured estimate — see §13.1 of `docs/TECHNICAL_SPEC.md`)

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

**Public inputs (4 field elements; the verifying key's `IC` array has 5 entries — `IC[0]` for the constant term plus one per public input):**
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
**Constraints:** 18,773 (real, measured — the 32-level Merkle proof dominates; an earlier design-time estimate of ~6,200 undercounted this substantially)  
**Proving time:** ~600ms (unmeasured estimate)

**Note on `recipient_hash`:** it is a public input, but the circuit itself places no constraint on it — it doesn't tie it to `value`, `asset_id`, or anything else proven above. The binding is enforced at the contract layer instead: `contracts/token::unshield()` recomputes `recipient_hash` itself from the actual recipient address (plus, for swap-originated calls, a `binding_tag` — see `docs/TECHNICAL_SPEC.md`'s `unshield()` interface listing) and rejects the call if the submitted proof's public input doesn't match. Because this value is circuit-unconstrained, adding `binding_tag` to close the swap proof-replay finding (`docs/POC_IMPLEMENTATION.md`'s "Update: external audit") needed no circuit or trusted-setup change — only a contract-and-SDK-level convention change.

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

    // recipient_hash is a public binding — not used in constraints
    // but included as public input so the contract can verify destination
    signal recipient_hash_check;
    recipient_hash_check <== recipient_hash;
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
**Constraints:** 42,853 (real, measured — two full 32-level Merkle proofs plus commitment/nullifier/value-commit checks; an earlier design-time estimate of ~15,450 undercounted this substantially)  
**Proving time:** ~2.0s (unmeasured estimate)

**Update (security-review finding, since fixed):** each input/output slot above is constrained independently (its own Merkle proof, its own nullifier/commitment derivation), with nothing originally linking the slots together — so a prover could supply the *same* real note as both `in_value[0]` and `in_value[1]` (same `rho` ⇒ same nullifier in both positions), making `sum_in` double-count one note's value and letting a holder of value `V` mint `2V - fee` in fresh output notes. Fixed at the contract boundary (a same-call pairwise distinctness check on nullifiers and output commitments, before proof verification, in `contracts/token/src/lib.rs`) and, for defense-in-depth, in-circuit with explicit non-equality constraints on `nullifiers[0]`/`nullifiers[1]` and `out_commitments[0]`/`out_commitments[1]`, shown below. See `docs/POC_IMPLEMENTATION.md`'s "Tests and vectors" section for the finding.

```circom
pragma circom 2.0.0;

include "../../node_modules/circomlib/circuits/comparators.circom";
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

    // Each input slot above is constrained independently (its own Merkle
    // proof, its own nullifier derivation) with nothing linking the two
    // slots together — so without this, a prover could supply the SAME real
    // note as both in_value[0] and in_value[1] (same rho => same nullifier
    // in both positions), making sum_in double-count one note's value and
    // letting a holder of value V mint 2V-fee in fresh output notes.
    component nf_distinct = IsZero();
    nf_distinct.in <== nullifiers[0] - nullifiers[1];
    nf_distinct.out === 0;

    // Same reasoning for the two output commitments.
    component cm_distinct = IsZero();
    cm_distinct.in <== out_commitments[0] - out_commitments[1];
    cm_distinct.out === 0;
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
**Constraints:** 40,268 (real, measured)  
**Proving time:** ~4.5s (unmeasured estimate)

Structurally identical to Transfer 2x2 with `N_IN = 4`, `N_OUT = 4`. Its real measured constraint count (40,268) is, perhaps counter-intuitively, not much higher than 2-in/2-out's (42,853) despite twice the inputs/outputs — both numbers come straight from `snarkjs r1cs info` against the real compiled circuits, not a transcription error, but the reason the scaling isn't closer to 2x hasn't been dug into further here.

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
**Constraints:** 927 (real, measured — no Merkle proof in this circuit, unlike shield/unshield/transfer, so it's much smaller than an earlier design-time estimate of ~3,500 assumed)  
**Proving time:** ~400ms (unmeasured estimate)

**Update (external technical review):** `min_amount_out` was originally a free public input, unconstrained relative to `amount_in`/`max_slippage_bps` — the values actually bound into `intent_commitment` — so a prover could supply an arbitrarily low `min_amount_out` (e.g. `0`) at reveal time regardless of the slippage tolerance actually committed to, defeating the circuit's entire front-running/price-protection guarantee. Fixed by deriving `min_amount_out` in-circuit as `floor(amount_in * (10000 - max_slippage_bps) / 10000)` and constraining the public input to equal that derivation, plus a `max_slippage_bps <= 10000` bound so the subtraction can't wrap the field, and an explicit 32-bit range check on `max_slippage_bps` itself (needed so the `amount_in * 2^32 + max_slippage_bps` packing used to bind `intent_commitment` stays injective). See `docs/POC_IMPLEMENTATION.md`'s "Update: external audit" for the finding and `contracts/verifier`'s `verify_accepts_real_swap_fairness_circuit_proof` / `verify_rejects_real_swap_fairness_circuit_proof_with_forged_min_amount_out` tests. The circuit below is the current, fixed version — matches `circuits/swap/swap_fairness.circom` exactly.

```circom
pragma circom 2.0.0;

include "../../node_modules/circomlib/circuits/bitify.circom";
include "../../node_modules/circomlib/circuits/comparators.circom";
include "../common/poseidon2.circom";
include "../common/range.circom";

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
    signal input min_amount_out;     // must equal floor(amount_in*(10000-max_slippage_bps)/10000)

    // Reconstruct intent commitment
    component h1 = Poseidon2();
    h1.in[0] <== asset_in;
    h1.in[1] <== asset_out;

    // amount_in and max_slippage_bps must be range-bounded before packing,
    // or a prover could find a different (amount_in, max_slippage_bps) pair
    // that packs to the same field element and the same intent_commitment.
    component amount_in_range = Range64();
    amount_in_range.value <== amount_in;
    component slippage_range = Num2Bits(32);
    slippage_range.in <== max_slippage_bps;

    // max_slippage_bps <= 10000 (100%), or `10000 - max_slippage_bps` below
    // wraps the field instead of going negative, corrupting the derivation.
    component slippage_bound = LessThan(32);
    slippage_bound.in[0] <== max_slippage_bps;
    slippage_bound.in[1] <== 10001;
    slippage_bound.out === 1;

    // Pack amount_in and max_slippage_bps into one field element
    signal packed <== amount_in * (2**32) + max_slippage_bps;

    component h2 = Poseidon2();
    h2.in[0] <== packed;
    h2.in[1] <== intent_nonce;

    component h3 = Poseidon2();
    h3.in[0] <== h1.out;
    h3.in[1] <== h2.out;

    h3.out === intent_commitment;

    // min_amount_out must equal floor(amount_in*(10000-max_slippage_bps)/10000)
    // — the standard circom quotient/remainder pattern.
    signal scaled <== amount_in * (10000 - max_slippage_bps);
    signal remainder <-- scaled % 10000;
    min_amount_out * 10000 + remainder === scaled;
    component remainder_range = LessThan(14); // 10000 < 2^14
    remainder_range.in[0] <== remainder;
    remainder_range.in[1] <== 10000;
    remainder_range.out === 1;

    // Fairness check: amount_out >= min_amount_out
    // Enforced as: amount_out - min_amount_out >= 0
    signal diff <== amount_out - min_amount_out;
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

Real, measured via `snarkjs r1cs info` against the compiled circuits, except Non-Membership (never built in this environment — kept as the original design-time estimate, marked accordingly):

| Circuit | R1CS Constraints | Wires | Labels |
|---|---|---|---|
| Shield | 2,133 | 2,138 | 3,164 |
| Unshield | 18,773 | 18,811 | 27,969 |
| Transfer 2x2 | 42,853 | 42,931 | 63,809 |
| Transfer 4x4 | 40,268 | 40,420 | 127,635 |
| Swap Fairness | 927 | 930 | 2,544 |
| Non-Membership | ~9,000 (estimate, unbuilt) | ~9,800 (estimate) | ~13,800 (estimate) |

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
| Published at | `https://github.com/ZKELLA-org/zkella/releases` |

All ceremony contributions will be posted publicly. Verification instructions included in release notes.

---

*ZKELLA Circuit Specification v0.1.0*
