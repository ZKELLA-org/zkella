# Test coverage

Line and region coverage of the token contract's Rust tests, how it was measured, an independent review of whether it is adequate, and what remains uncovered.

## Command

From `contracts/`:

```
cargo llvm-cov -p zkella-token --release --summary-only
cargo llvm-cov report --release --show-missing-lines     # itemised uncovered lines
```

The run compiles the crate and its dependencies with coverage instrumentation and takes several minutes on a cold build.

## Result (token crate, `zkella-token`, 127 tests)

| File | Regions | Lines | Functions |
| --- | --- | --- | --- |
| `src/lib.rs` | 99.74% | 99.89% | 100% |
| `src/merkle.rs` | 99.40% | 99.18% | 93.75% |
| `src/poseidon.rs` | 99.68% | 100% | 100% |
| `src/test_groth16.rs` | 100% | 100% | 100% |
| `src/types.rs` | 0% | 0% | 0% |
| Total | 99.59% | 99.55% | 92.94% |

Not separately measured: the `verifier`, `swap`, `governance`, `compliance` and `viewing_keys` crates. Their behaviour is exercised by their own tests (37, 30, 25, 13 and 5), but no coverage figure exists for them.

## Independent review of adequacy

The first version of this report was reviewed by someone other than its author. Verdict on that version: **not adequate**. Line coverage was high (about 99%), but:

- event emission (`shield` and `note` events, which wallets and the indexer depend on) was not asserted by any test;
- there was no authorisation test (every test used blanket auth mocking), so a missing `require_auth` would not have been caught;
- zero amounts and extreme amounts (`i128::MAX`, values above 2^64, overflowing sums) were not tested;
- every per-item error branch of `shield_batch` was unexecuted, and there was no test that a mid-batch failure rolls everything back;
- the branches that bind the proof's public inputs to the call arguments (`pub_value`, `pub_asset_id`, commitment) were unexecuted;
- several tests asserted only `is_err()`, which passes on an unrelated error;
- the Merkle root was only compared with "not equal" to the previous root, never with an independently computed value.

These were all closed with 49 new tests (`contracts/token/src/tests/shield_flow.rs`, 33 tests, and `tests/spend_paths.rs`, 16 tests). They assert the exact error and that leaf count, root, supply, nullifiers and balances are unchanged on failure. Together with the pause, admin-transfer and `merkle_path` tests added earlier, and the clawback, at-scale and cost-parity tests, this took the crate from 55 to 116 tests, with `lib.rs` at 100% of lines at that point.

Since then, Tranche 2's relayer-fee and unshield change-note features added real code without, in three places, a dedicated negative test for a defensive branch — see "What remains uncovered" below. A later audit pass added `transfer`/`transfer4`/`unshield` `encrypted_note`-length validation (closing a real fund-stranding gap, not a coverage gap) together with its own three new tests, which fully cover the new branches they add — `lib.rs` stays at 100% of functions. The crate is now at 127 tests, with the same three Tranche-2-era lines (now at different line numbers) still the only gaps.

## What remains uncovered

- `types.rs` (10 lines, 10 functions): derive and conversion glue generated for the contract types. Not behaviour.
- `lib.rs`, 3 lines, each a defensive error branch introduced by a Tranche 2 feature, correctly implemented but not yet exercised by its own test:
  - `transfer`'s guard against a relayer fee driving `shielded_supply` negative (an underflow that passes `checked_sub` because the result is still a representable negative `i128`, but is semantically invalid).
  - `unshield`'s change-note `change_commitment` binding-mismatch check (the same pattern as the already-tested nullifier mismatch check immediately above it).
  - `unshield`'s change-note `MerkleTreeFull` pre-check (the same pattern already tested for `shield`/`shield_batch`/`transfer`, not yet repeated for this call site).
- `merkle.rs`, 2 lines: `is_known_root`'s fallback branch for when `RootHistory` has never been written (every test inserts at least one leaf first, so this closure never runs), and a diagnostic `panic!` message inside the test helper `assert_tree_matches` that is only reached if a test were to fail (by design, never during a passing run).

The three `lib.rs` gaps are genuine, open test-coverage gaps on real (if narrow) code paths, not yet closed.

## Limits of this measure

Coverage says which code the tests executed, not that the tests check the right things; the review above is what addressed that for the token contract. Some of the most important properties here (circuit soundness, the verifier's pairing check against adversarial inputs, behaviour across contract upgrades) are not what line coverage measures; they are covered by the circuit witness tests, the real-proof verifier tests, the fuzz targets and the two security reviews. The fuzz targets (`contracts/token/fuzz`) are a separate, modest effort and are not part of this figure. The other contract crates have not had the same independent adequacy review.
