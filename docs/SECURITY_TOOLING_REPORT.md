# Security tooling report

Self-review by the development team using automated tooling. This is not an independent third-party audit.

Status: **in progress**. The sections below record what has been run, what was found, what was fixed, and what is still open. Open items are listed explicitly rather than implied as done.

## Scope

- Rust contracts workspace (`contracts/`): token, verifier, swap, governance, compliance, viewing_keys
- Circuits (`circuits/`): shield, transfer, unshield, swap fairness, compliance non-membership
- JavaScript SDK, indexer, and scripts (`sdk/`, `indexer/`, `scripts/`), production dependencies and dev dependencies

## Tools and results

### Rust dependency advisories (`cargo audit`)

| Finding | Status |
| --- | --- |
| `spin` 0.9.8 yanked from crates.io | **Fixed**: updated to 0.9.9 (commit `8133468`) |
| `paste` 1.0.15 unmaintained (RUSTSEC-2024-0436) | **Accepted risk**: no known vulnerability. Pulled in through `ark-ff` → `soroban-env-host` → `soroban-sdk`, so it can only be removed by a Soroban SDK upgrade. Revisit on the next SDK upgrade. |

### JavaScript dependency advisories (`npm audit --omit=dev`)

| | Before | After |
| --- | --- | --- |
| High | 5 | 0 |
| Moderate | 2 | 0 |
| Low | 13 | 15 |

Fixes (commit `d38087e`):
- In-range `npm audit fix` updates (bfj, jsonpath, brace-expansion, ethers sub-packages).
- npm `overrides` pinning `ws` to `^8.22.0` (memory exhaustion and uninitialized-memory disclosure) and `underscore` to `^1.13.8` (unbounded recursion DoS). Neither override crosses a major version.

Accepted risk, low severity: the remaining 15 findings are the ethers v5 chain (`ethers`, `@ethersproject/*`, `elliptic`) pulled in by `circomlibjs@0.1.7`. The only fix npm offers is a downgrade to `circomlibjs@0.0.8`, which is breaking. ZKELLA uses `circomlibjs` only for Poseidon and babyjubjub. The ethers-importing modules (`mimc7`, `mimcsponge`, `evmasm`, and the `*_gencontract` generators) are not referenced anywhere in the repository. Revisit when `circomlibjs` is upgraded.

Full JS test suite after the fix: 158/158 passing against PostgreSQL.

### Rust lints (`cargo clippy --workspace --all-targets --release`)

- Rustc compiler warnings: **0**.
- Clippy correctness-class lints: **0**.
- Clippy style-class lints: 31 distinct occurrences across token, swap, and verifier. Categories: too many function arguments (10 occurrences, 8 to 11 arguments per function), index-based loops (9), `clone` that can be `from_ref` (3), `is_multiple_of` suggestion (3), redundant casts, a `RangeInclusive::contains` suggestion, and two other style suggestions. **Accepted for now**: none of these is a correctness finding. Fixing the argument-count lints would change the signatures of proof-verification helpers and their call sites, which is a larger refactor than this review pass covers, so it is deferred.

### Fuzzing (`cargo fuzz`, `contracts/token/fuzz`)

- Existing targets: `shield_arbitrary`, `transfer_arbitrary`, `verifier_arbitrary`. Short local runs (20 to 60 seconds each) found no crashes. These are smoke runs, not a full fuzzing campaign.
- A build break in `transfer_arbitrary` (the `relayer` argument added in Tranche 2) was fixed in commit `cb5b409`. The CI fuzz-smoke job now passes.
- **Open**: fuzz harnesses do not yet cover every contract entrypoint. The remaining entrypoints across swap, governance, compliance, viewing_keys, and the rest of token are listed under "Open items".

### Instruction budget (real WASM)

- Measured with the real-WASM methodology for `shield`, `transfer`, and `transfer4`, plus the shield-batch and unshield cost tests.
- **Open**: the remaining entrypoints across all six contracts have not been measured yet.

## Open items

1. Fuzz harnesses for every state-changing entrypoint across all six contracts.
2. Real-WASM instruction-budget measurement for every remaining entrypoint, with results published.
3. Triage of any new findings from items 1 and 2, with each finding fixed or documented as accepted risk.
