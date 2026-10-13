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

Full JS test suite after the fix: 158/158 passing against PostgreSQL (current total, after further additions since: 188/188 — see the README's "Status at a glance").

### Rust lints (`cargo clippy --workspace --all-targets --release`)

- Rustc compiler warnings: **0**.
- Clippy correctness-class lints: **0**.
- Clippy style-class lints: 31 distinct occurrences across token, swap, and verifier. Categories: too many function arguments (10 occurrences, 8 to 11 arguments per function), index-based loops (9), `clone` that can be `from_ref` (3), `is_multiple_of` suggestion (3), redundant casts, a `RangeInclusive::contains` suggestion, and two other style suggestions. **Accepted for now**: none of these is a correctness finding. Fixing the argument-count lints would change the signatures of proof-verification helpers and their call sites, which is a larger refactor than this review pass covers, so it is deferred.

### Fuzzing (`cargo fuzz`, `contracts/token/fuzz`)

- Targets: `shield_arbitrary`, `transfer_arbitrary` (covers `transfer` and `transfer4`), `verifier_arbitrary` (covers `verify` and `verify_batch`), `swap_arbitrary` (every swap entrypoint), `governance_arbitrary`, `compliance_arbitrary`, `viewing_keys_arbitrary`, `token_admin_arbitrary` (token admin, pause, allowlists, relayers, `unshield`, `shield_batch`), `verifier_admin_arbitrary` (verifier key registration, update, revocation). Each target asserts its own invariants: a bogus proof is never accepted, a rejected call leaves no state behind, a second initialize is rejected, and a VK update cannot execute before its timelock.
- Short local runs (20 to 60 seconds each) found no crashes or invariant violations. These are smoke runs, not a full fuzzing campaign.
- All nine targets now have a committed, minimized corpus (`cargo +nightly fuzz cmin`), so CI's smoke job starts from real coverage rather than an empty corpus on every run. `compliance_arbitrary`'s corpus includes a regression seed from a real crash the harness found (a fuzzer input whose random root happened to equal the authorized one) and the fix that followed.
- A build break in `transfer_arbitrary` (the `relayer` argument added in Tranche 2) was fixed in commit `cb5b409`.
- To make the contracts importable from the fuzz crate, the swap, governance, compliance, and viewing_keys crates now also build an rlib, matching the token crate. The WASM output is unchanged, and their tests pass.
- The fuzz harnesses call the state-changing entrypoints with arbitrary inputs: the data entrypoints of every contract, token admin and allowlists, token-action queuing (governance), verifier key management, compliance root binding, and viewing-key revoke. Pause, unpause, and guardian cancel are exercised by unit tests, not by the harnesses. Read-only getters are exercised only indirectly.
- A change to token's `lib.rs` re-exports `ShieldBatchItem` so the fuzz crate can construct batch inputs. No runtime behavior changed.

### Instruction budget (real WASM)

- Proof-consuming entrypoints (`shield`, `shield_batch`, `transfer`, `transfer4`, `unshield`, verifier `verify` and `verify_batch`, swap commit/execute/reveal, governance queue and execute, compliance publish) are measured by the cost-parity tests inside each contract crate, against real compiled WASM.
- Non-proof entrypoints are measured by `contracts/budget/tests/instruction_budget.rs` (run with `cd contracts/budget && cargo test --release -- --nocapture`). It registers the real compiled WASM from `contracts/target/wasm32v1-none/release`, which must be built first. Every row is a single successful call, measured as CPU instructions from the Soroban cost tracker, against the 400,000,000 mainnet budget.
- Highest non-proof cost is `token::merkle_root` at 0.55% of the budget. Every measured non-proof entrypoint is under 0.6%.

| Contract | Entrypoint | CPU instructions | % of budget |
| --- | --- | ---: | ---: |
| token | set_min_shield_amount | 724,308 | 0.18 |
| token | min_shield_amount | 653,149 | 0.16 |
| token | set_asset_approved | 725,625 | 0.18 |
| token | is_asset_approved | 658,071 | 0.16 |
| token | set_relayer | 738,398 | 0.18 |
| token | is_approved_relayer | 665,953 | 0.17 |
| token | merkle_root | 2,196,785 | 0.55 |
| token | leaf_count | 666,046 | 0.17 |
| token | shielded_supply | 665,890 | 0.17 |
| token | is_spent | 660,881 | 0.17 |
| token | pause | 745,239 | 0.19 |
| token | unpause | 745,165 | 0.19 |
| token | transfer_admin | 749,713 | 0.19 |
| token | accept_admin | 783,153 | 0.20 |
| verifier | initialize | 491,818 | 0.12 |
| verifier | register_verifying_key | 562,583 | 0.14 |
| verifier | update_verifying_key | 624,766 | 0.16 |
| verifier | get_verifying_key | 501,132 | 0.13 |
| verifier | revoke_previous_vk | 566,231 | 0.14 |
| governance | initialize | 525,991 | 0.13 |
| governance | timelock_ledgers | 412,394 | 0.10 |
| governance | queue_vk_update | 557,723 | 0.14 |
| governance | cancel_vk_update | 515,077 | 0.13 |
| governance | execute_vk_update | 1,638,802 | 0.41 |
| governance | revoke_previous_vk | 1,037,359 | 0.26 |
| governance | transfer_admin | 527,778 | 0.13 |
| governance | accept_admin | 550,851 | 0.14 |
| governance | queue_token_action | 572,889 | 0.14 |
| governance | execute_token_action | 1,338,209 | 0.33 |
| governance | guardian_cancel_token_action | 504,743 | 0.13 |
| compliance | initialize | 413,602 | 0.10 |
| compliance | get_compliance_proof | 377,773 | 0.09 |
| compliance | set_sanctions_root | 434,117 | 0.11 |
| viewing_keys | register | 380,531 | 0.10 |
| viewing_keys | get_viewing_key_commitment | 331,662 | 0.08 |
| viewing_keys | revoke | 345,412 | 0.09 |
| swap | initialize | 602,003 | 0.15 |
| swap | set_relayer | 624,340 | 0.16 |

- Swap `cancel_swap` and `reclaim_expired_swap` are measured by `cost_parity_swap_cancel_and_reclaim` in the swap crate, on the compiled WASM: `cancel_swap` 1,076,688 instructions (0.27% of budget) and `reclaim_expired_swap` 1,449,177 (0.36%). Their native figures are 422,170 and 733,539. The WASM figures exceed native by more than 25%, because fixed WASM instantiation overhead dominates these small calls. The test applies the 2M-instruction allowance the governance parity test uses, and it asserts the absolute budget limit as well.

## Open items

1. Fuzz harnesses for every state-changing entrypoint across all six contracts.
2. Real-WASM instruction-budget measurement for every remaining entrypoint, with results published.
3. Triage of any new findings from items 1 and 2, with each finding fixed or documented as accepted risk.
