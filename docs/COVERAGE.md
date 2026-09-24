# Test coverage

This document records line and region coverage of the token contract's Rust tests, how it was measured, and what it does and does not show.

## Command

From `contracts/`:

```
cargo llvm-cov -p zkella-token --release --summary-only
```

The run compiles the crate and its dependencies with coverage instrumentation and takes several minutes on a cold build.

## Result (token crate, `zkella-token`)

| File | Regions | Functions |
| --- | --- | --- |
| `src/lib.rs` | 99.30% | 100% |
| `src/merkle.rs` | 96.41% | not recorded here |
| `src/poseidon.rs` | 99.69% | not recorded here |
| Total | 99.14% | 92.09% |

Total line coverage for the crate is about 99%. Per-file line and function figures other than those in the table were not recorded in this document; rerun the command above for the full report.

Not separately measured: the `verifier`, `swap`, `governance`, `compliance` and `viewing_keys` crates. Their `#[test]` counts are in `docs/POC_IMPLEMENTATION.md`, but no coverage figure exists for them.

## What the earlier review found

A first coverage pass on the token crate showed:

- `pause` / `unpause`, the two-step admin transfer (`transfer_admin` / `accept_admin`) and `merkle_path` had no tests. They are now covered.
- `get_path_indices` and `verify_path` in `merkle.rs` were not referenced by any code path. A test now exercises them.

## What remains uncovered

The remaining uncovered regions are the roughly 0.7% of `lib.rs` regions, 3.6% of `merkle.rs` regions and 0.3% of `poseidon.rs` regions not hit by any test, and the 7.9% of functions in the crate total that no test called (which functions was not itemised here). This document does not list them individually; the per-line report from `cargo llvm-cov --html` does.

## Limits of this measure

Coverage says which code the tests executed, not that the tests check the right things. A line can be executed by a test that asserts nothing about its result, and some of the most important properties here (soundness of the circuits, the verifier's pairing check against adversarial inputs, behaviour across contract upgrades) are not the kind of thing line coverage measures. The measure has not been independently reviewed. The token fuzz targets (`contracts/token/fuzz`) are a separate, small effort and are not part of this figure.
