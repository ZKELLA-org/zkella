# Test coverage

Line and region coverage of the token contract's Rust tests, how it was measured, an independent review of whether it is adequate, and what remains uncovered.

## Command

From `contracts/`:

```
cargo llvm-cov -p zkella-token --release --summary-only
cargo llvm-cov report --release --show-missing-lines     # itemised uncovered lines
```

The run compiles the crate and its dependencies with coverage instrumentation and takes several minutes on a cold build.

## Result (token crate, `zkella-token`, 111 tests)

| File | Regions | Lines | Functions |
| --- | --- | --- | --- |
| `src/lib.rs` | 99.80% | 100% | 100% |
| `src/merkle.rs` | 99.10% | 99.26% | 92.31% |
| `src/poseidon.rs` | 99.69% | 100% | 100% |
| `src/types.rs` | 0% | 0% | 0% |
| Total | 99.66% | 99.68% | 93.15% |

Not separately measured: the `verifier`, `swap`, `governance`, `compliance` and `viewing_keys` crates. Their behaviour is exercised by their own tests (25, 12, 4, 3 and 2), but no coverage figure exists for them.

## Independent review of adequacy

The first version of this report was reviewed by someone other than its author. Verdict on that version: **not adequate**. Line coverage was high (about 99%), but:

- event emission (`shield` and `note` events, which wallets and the indexer depend on) was not asserted by any test;
- there was no authorisation test (every test used blanket auth mocking), so a missing `require_auth` would not have been caught;
- zero amounts and extreme amounts (`i128::MAX`, values above 2^64, overflowing sums) were not tested;
- every per-item error branch of `shield_batch` was unexecuted, and there was no test that a mid-batch failure rolls everything back;
- the branches that bind the proof's public inputs to the call arguments (`pub_value`, `pub_asset_id`, commitment) were unexecuted;
- several tests asserted only `is_err()`, which passes on an unrelated error;
- the Merkle root was only compared with "not equal" to the previous root, never with an independently computed value.

These were all closed with 49 new tests (`contracts/token/src/tests/shield_flow.rs`, 33 tests, and `tests/spend_paths.rs`, 16 tests). They assert the exact error and that leaf count, root, supply, nullifiers and balances are unchanged on failure. Together with the pause, admin-transfer and `merkle_path` tests added earlier, and the clawback, at-scale and cost-parity tests, this took the crate from 55 to 111 tests and `lib.rs` to 100% of lines.

## What remains uncovered

- `types.rs` (9 lines, 9 functions): derive and conversion glue generated for the contract types. Not behaviour.
- One line in `merkle.rs`: the false branch of a test-only helper, `verify_path`.

There are no uncovered error branches left in the shield, `shield_batch`, `transfer`, `transfer4` or `unshield` entrypoints.

## Limits of this measure

Coverage says which code the tests executed, not that the tests check the right things; the review above is what addressed that for the token contract. Some of the most important properties here (circuit soundness, the verifier's pairing check against adversarial inputs, behaviour across contract upgrades) are not what line coverage measures; they are covered by the circuit witness tests, the real-proof verifier tests, the fuzz targets and the two security reviews. The fuzz targets (`contracts/token/fuzz`) are a separate, modest effort and are not part of this figure. The other contract crates have not had the same independent adequacy review.
