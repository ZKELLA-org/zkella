# Instruction-cost optimisation

How the instruction cost of ZKELLA's transactions was reduced, why each change works, how each was proven correct, and how the result is protected against regression. Every figure is an instruction count measured by the Soroban host on the **compiled `wasm32v1-none` contracts** (what the network runs), against the network's limit of **400,000,000 instructions per transaction**.

## 1. Summary

| Transaction | Before | After | Change | Share of the 400M limit (after) |
| --- | --- | --- | --- | --- |
| `transfer4` (4 in, 4 out) | 397.9M | 80.8M | -80% | 20% |
| `transfer` (2 in, 2 out) | 233.8M | 74.9M | -68% | 19% |
| `shield` | 118.6M | 76.4M | -36% | 19% |
| `shield_batch` | 3 items: 347.2M (the maximum) | 3 items: 144.2M; **8 items: 314.4M (the new maximum)** | -58% for 3 items | 79% at 8 items |
| `unshield` | 34.1M | 32.8M | -4% | 8% |
| verifier `verify` (4 inputs) | 29.2M | 28.0M | -4% | 7% |
| verifier `verify_batch` (4 proofs) | 63.0M | 63.1M | unchanged | 16% |

The same transactions on Testnet (declared resources, which include the client's safety margin): `transfer4` 378.7M to 86.3M, `shield` 126.1M-126.3M to 80.6M-81.2M, `swap reveal_and_claim` 158.5M to 112.2M, and a `shield_batch` of 8 items, which was not possible before, at 335.0M. Details in section 7.

`transfer4` was the roadmap's main risk: it sat at 99.5% of the limit, with about 2M instructions of headroom. It now uses 20%.

## 2. Where the cost was

The obvious suspect was proof verification (a Groth16 check with 19 public inputs). It was not the main cost. Measuring the pieces separately showed:

- A Groth16 verification costs about **28M** instructions, almost independent of the circuit (the pairing dominates).
- One Poseidon2 hash costs about **1.2M to 1.5M** instructions on the compiled WASM (the host's Poseidon permutation plus the surrounding value conversions).
- Everything else (storage, events, checks) is small next to those two.

So the cost of a transaction is roughly `28M x (number of proofs) + 1.3M x (number of hashes)`. For `transfer4` that is 28M for the proof plus the hashing of the four new notes into the Merkle tree. The tree hashing was the problem:

`merkle::insert` (before) did, for every inserted leaf:

1. 32 hashes to climb the 32 levels of the tree (needed), and
2. 32 more hashes to recompute the roots of *empty subtrees* at every level (not needed: they are constants that depend only on the hash function).

That is **64 hashes per leaf**. `transfer4` inserts four leaves: **256 hashes, about 330M instructions, 83% of the whole transaction**. `shield` inserts one leaf (plus 3 hashes to recompute its own commitment): 67 hashes.

Earlier in the project the same function had been even worse (it recomputed the empty-root chain from scratch at every level, roughly 500 hashes per insert); that was fixed once by tracking the empty root incrementally, which is the "32 extra hashes" above. This work removed the remainder.

## 3. The four changes, in order

Each change was measured on its own against the previous state.

| Step | Change | `transfer4` | `transfer` | `shield` | `shield_batch` (3) | `unshield` |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | before | 397.9M | 233.8M | 118.6M | 347.2M | 34.1M |
| 1 | precomputed empty-subtree roots | 210.5M | 121.4M | 81.1M | 234.6M | 34.1M |
| 2 | batched Merkle insertion | 83.3M | 77.4M | 79.0M | 149.5M | 34.0M |
| 3 | Poseidon input limbs passed to the host directly | 82.0M | 76.1M | 77.5M | 147.7M | 34.0M |
| 4 | proof point negated on its bytes | 80.8M | 74.9M | 76.4M | 144.2M | 32.8M |

### Step 1: precomputed empty-subtree roots (`contracts/token/src/merkle.rs`)

**Problem.** `insert` carried a "running empty root" and hashed it once per level: `running_empty = hash(running_empty, running_empty)`. Those 32 hashes per insert produce the same 32 constants every time.

**Change.** The 33 values (`EMPTY_ROOTS[0..=32]`, where `EMPTY_ROOTS[0] = Poseidon2(0, 0)` and `EMPTY_ROOTS[i] = Poseidon2(EMPTY_ROOTS[i-1], EMPTY_ROOTS[i-1])`) are now a `const` table, generated with circomlibjs. `insert`, `get_path` and `root()` read the table.

**Effect.** Half of every insert disappears: `transfer4` -187M, `shield` -37.5M.

**Why it is correct.** The values are the definition of an empty subtree at each level. `empty_roots_table_matches_poseidon_chain` recomputes the whole chain in pure Rust (a different Poseidon implementation from the contract's host call) and requires an exact match at all 33 levels. If the constants were ever wrong, that test fails.

### Step 2: batched Merkle insertion (`merkle::insert_many`)

**Problem.** A transfer inserts 2 or 4 consecutive leaves. Inserting them one by one climbs 32 levels each time, but consecutive leaves share almost every ancestor above the first couple of levels, so most of that hashing recomputes the same nodes and overwrites the same storage entries.

**Change.** `insert_many(commitments)` inserts consecutive leaves together:

1. Write the new leaves at their indices.
2. For each level, compute the parents of the *range* of nodes touched at that level, once. The left child of the leftmost parent may be outside the batch (a node already in the tree): that one is read from storage. Every other child is either inside the batch (held in memory) or to the right of the last node, where everything is empty because leaves are only ever appended, so the constant from step 1 is used and no storage read happens.
3. After 32 levels a single node remains: the new root.

A batch of 4 leaves needs about 33 to 40 hashes instead of 4 x 32 = 128, and about a third of the storage writes. `insert` (one leaf) now calls `insert_many` with a batch of one, so there is a single implementation.

Callers: `transfer` and `transfer4` insert all their output commitments in one call; `shield_batch` validates every item, records each commitment as seen, then inserts all commitments in one call before the single token transfer.

**Why it is correct.** The tree stored after a batch is exactly the tree that inserting the leaves one at a time would store: same leaf indices, same node values at every level, same final root. This is not argued, it is tested against a tree computed independently in pure Rust:

- `insert_many_matches_an_independent_tree_for_every_batch_size_and_alignment`: 30 consecutive batches with sizes 1 to 8 in a deliberately irregular order, so batches start at every alignment and cross the power-of-two boundaries at 2, 4, 8, 16, 32 and 64 (121 leaves in total). After every batch it compares **every node at every level** and the root with the independent tree.
- `single_and_batched_inserts_can_be_mixed`: interleaves single and batched inserts.
- `merkle_insert_cost_and_correctness_at_thousands_of_leaves`: a real insert into a 5,000-leaf tree on the compiled WASM produces the root of the independently computed 5,001-leaf tree.
- Existing tests that read paths and roots (`merkle_root_matches_independent_recomputation`, `merkle_path_reconstructs_the_current_root`) pass unchanged.
- On Testnet, the `transfer4` and `unshield` proofs are built from Merkle paths read back from the batch-built tree and verify on-chain; a wrong node would make those proofs fail.

**Capacity.** `has_capacity(env, n)` checks `next_index + n <= MAX_LEAVES` in 64-bit arithmetic *before any state changes*, so a batch that does not fit fails as `MerkleTreeFull` without partial writes (`has_capacity_boundaries`, `merkle_last_slot_boundary`, the full-tree tests for `shield`, `shield_batch` and `transfer`).

**One behavioural difference.** Sequential inserts appended one entry to the root-history ring buffer per leaf; a batch appends one entry (its final root). Intermediate roots exist only inside the call, so no transaction can be anchored to them, and an anchor that was accepted before is still accepted. The 32-entry window now counts *calls that inserted leaves*, not leaves, which makes it slightly longer in wall-clock terms. `transfer_accepts_anchor_still_within_root_history_window` and `..._evicted_...` pass unchanged (they insert through single shields).

### Step 3: Poseidon input limbs (`contracts/token/src/poseidon.rs`)

**Problem.** Each hash converted its two inputs from little-endian bytes to big-endian `U256` through reversed byte arrays, a `Bytes` object and `from_be_bytes`, and converted the output back the same way.

**Change.** The inputs are reduced modulo the field order as before (`Fr::from_bytes`), and their four 64-bit limbs are passed straight to the host with `U256::from_parts`; the output is copied out with `copy_into_slice` and reversed once. Saves about 1.3M to 1.7M per transaction.

**Why it is correct.** Same reduction, same host call, same byte order in and out. The existing equivalence tests compare the host-backed hash with the pure-Rust implementation on zero, small, arbitrary and non-canonical inputs (`poseidon2_native_matches_pure_rust_*`), and `note_commitment_matches_real_shield_circuit_v2_500stroops_vector` ties the result to a real circuit proof.

### Step 4: negating the proof point on its bytes (`contracts/verifier/src/lib.rs`)

**Problem.** The Groth16 check needs `-A`. The verifier computed it as `A * (r - 1)`, a full scalar multiplication on the curve, because the host has no negation call.

**Change.** Negating a G1 point only flips its y coordinate: `(x, y) -> (x, q - y)`, where `q` is the base-field modulus. `negate_g1_bytes` does that on the 64-byte encoding with a 32-byte borrow subtraction. Saves about 1.2M per verification.

**Why it is correct, including for bad input.** The scalar-multiplication version rejected an invalid point (the host validates points). To keep that, `y = 0` and `y >= q` are returned unchanged, so the host's point validation in the pairing check still rejects them (there is no curve point with `y = 0`, and coordinates must be reduced). Tests:

- `byte_negation_matches_scalar_multiplication_negation`: on the generator and eleven of its multiples (twelve real curve points), the byte negation equals the old scalar-multiplication result.
- `byte_negation_leaves_invalid_y_coordinates_unchanged`: `y = 0`, `y = q`, `y > q`.
- `verify_rejects_a_proof_with_an_invalid_a_point`: a real proof whose `A` has `y = 0` or `y = q` does not verify, and the untouched proof still does.
- All five real circuit proofs still verify.

`verify_batch` is unchanged: it scales `A` by a random challenge (`-r_j`), which is a real scalar multiplication, not a plain negation.

## 4. Two changes that changed the limits, not just the costs

- **`MAX_SHIELD_BATCH` rose from 3 to 8.** Before, three items already cost 347M. After, each extra item costs about 34M (its own proof and commitment check; the tree hashing is shared), so 8 items cost 314.4M (79%). The cap is not a guess: `shield_batch_size_sweep_on_real_wasm` runs every size from 1 to 8 on the compiled WASM (1 item 76.4M, 2: 109.7M, 3: 144.2M, 4: 177.5M, 5: 213.3M, 6: 246.6M, 7: 281.1M, 8: 314.4M) and **fails the build if the cap is raised past 85% of the limit**. The batch also has to fit the network's other per-transaction limits (ledger entries, written entries, write bytes): the live 8-item transaction used 64 entries, 58 written and 9,256 write bytes, well inside them.
- **The root history counts calls, not leaves** (see step 2).

## 5. What was measured and rejected

| Idea | Result | Decision |
| --- | --- | --- |
| Store each circuit's verifying key under its own storage entry, on the theory that every `verify` deserialises all registered keys | Registering all six keys changed `verify` by 0.1% (42,110 instructions) | Rejected: no benefit, would need a storage migration |
| Read the empty-subtree roots from storage instead of a `const` | Slower than a `const` and adds reads | Rejected |
| Build with `opt-level = 3` instead of the size-optimised `"z"` | transfer4 76.7M (-5%), shield 72.8M (-5%), shield_batch(3) 139.6M; token WASM 72.2KB instead of 88.5KB | Not adopted: at 20% of the limit it buys nothing visible and changes every artefact hash. It is the first thing to try if an entrypoint ever needs headroom |
| Call the host Poseidon permutation directly, skipping the sponge's own input checks and state setup | Estimated single-digit-percent gain | Not pursued: it means bypassing the library's canonical-input validation, a safety-relevant check, for a small gain |
| Disable overflow checks in release builds | Not measured | Not done: money arithmetic here relies on checked operations; trading arithmetic safety for cost is not justified at 20% of the limit |

## 6. How the result is protected

- **Cost cannot silently regress or drift from the native estimate.** `contracts/token/src/tests/cost_parity.rs` runs every proof-verifying entrypoint (`shield`, `shield_batch`, `transfer`, `transfer4`, `unshield`) twice, on the native contract and on the compiled WASM, prints both, and fails the build if the WASM cost exceeds 400M or is more than 25% above native (measured gaps: token entrypoints 9% to 15%, verifier 3%). The verifier's `verify` and `verify_batch` have the same test. CI builds the WASM and runs these on every push.
- **The batch cap is enforced by measurement** (`shield_batch_size_sweep_on_real_wasm`, above).
- **The MSM saving is guarded** by `msm_aggregation_is_cheaper_than_the_per_input_loop_on_transfer4`, which compares the current verifier with the pre-optimisation verifier WASM kept as `contracts/token/tests_data/verifier_pre_msm.wasm` (transfer4: 96.2M with the per-input loop, 80.8M with the batched multi-scalar multiplication).
- **Correctness of the changed code** is tested against independent implementations (Steps 1 to 4), not against itself.

## 7. Before and after, live on Testnet

Declared resources of the transactions (`scripts/tx_resource_profile.cjs`); the instruction figure is the simulation result plus the client's margin, so it sits a few percent above the measured cost. Before: the earlier validation stack. After: the current stack (`docs/TESTNET_DEPLOYMENT.md`).

| Transaction | Before | After |
| --- | --- | --- |
| `shield` | 126.1M to 126.3M | 80.6M to 81.2M |
| `shield_batch` | 3 items was the maximum | 8 items: 335.0M |
| `transfer4` | 378.7M | 86.3M |
| `unshield` | 35.5M | 34.4M |
| swap `commit_swap` | 44.0M | 42.9M |
| swap `reveal_and_claim` (a shield inside) | 158.5M | 112.2M |
| compliance `publish_compliance_proof` | 30.4M | 29.1M |

Ledger footprint also fell for anything that inserts leaves (a `shield` touches 41 to 43 entries instead of 73, `transfer4` 50 instead of 79), because each level's node is written once per call instead of once per leaf.

## 8. What this does not change

- No proof, circuit, verifying key or public-input format changed; every existing proof still verifies.
- The stored tree, the leaf indices, and the roots are identical to the previous algorithm; only the root history's granularity changed, as described.
- The optimisations reduce cost, not security: the canonical-input checks, the on-curve validation and the capacity guard are all still in place, and each was re-tested.

## 9. Reproducing the numbers

```
cd contracts
cargo build --workspace --target wasm32v1-none --release
cargo test -p zkella-token --release -- cost_parity shield_batch_size_sweep merkle_insert_cost_and_correctness msm_aggregation --nocapture
cargo test -p zkella-verifier --release -- cost_parity verify_batch_cost --nocapture
```

The tests print `COST_PARITY ...`, `BATCH_SWEEP ...`, `MERKLE_SCALE ...`, `MSM_VS_LOOP ...` and `BATCH_COST ...` lines with the figures used above.

## 10. The lesson

The cost that put a roadmap deliverable at risk was not in the cryptography everyone suspected but in a loop that recomputed constants. Two habits found it: measuring on the compiled WASM instead of the native estimate (which read 89% against the real 97%), and breaking the cost into hashes and proofs before optimising anything. The optimisation that mattered most (removing 32 redundant hashes per insert) was also the least risky, and the largest structural one (batch insertion) was accepted only after a test proved it stores the same tree.
