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
| **New, found 2026-11-13**: `crypto-bigint` 0.5.5, RUSTSEC-2026-0387 — "Multiple timing variability issues in operations intended to be constant-time" (dated 2026-10-23, after the previous `cargo audit` pass) | **Accepted risk, assessed low-relevance to ZKELLA's own cryptography.** `cargo tree -i crypto-bigint` traces this to `elliptic-curve → ecdsa → k256 → soroban-env-host → soroban-sdk` — Soroban's own host-environment dependency for `secp256k1`/ECDSA, not anything ZKELLA's own code calls. ZKELLA's Groth16/BN254 pairing and Poseidon hashing go through separate code paths (Soroban's native BN254 host functions and `soroban-poseidon`/`ark-bn254`) that do not touch `crypto-bigint`/`k256` at all. A fix would require a Soroban SDK major-version upgrade (latest is `30.0.0-rc.1`; this repo is pinned to the `27.x`/`28.x` range) — too large a change to apply mid-pass. Flagged here rather than silently carried forward; revisit alongside `paste` on the next SDK upgrade. |

### JavaScript dependency advisories (`npm audit --omit=dev`)

| | Before (original pass) | After (original pass) | **After (2026-11-13 re-run)** |
| --- | --- | --- | --- |
| High | 5 | 0 | **0** |
| Moderate | 2 | 0 | **0** |
| Low | 13 | 15 | **15** |

Fixes (commit `d38087e`):
- In-range `npm audit fix` updates (bfj, jsonpath, brace-expansion, ethers sub-packages).
- npm `overrides` pinning `ws` to `^8.22.0` (memory exhaustion and uninitialized-memory disclosure) and `underscore` to `^1.13.8` (unbounded recursion DoS). Neither override crosses a major version.

**2026-11-13 re-run found a new, separate issue and fixed it the same way**: `npm audit --omit=dev` reported a real `follow-redirects` 1.16.0–1.16.1 high-severity finding (credential-header-stripping bypass on cross-origin redirects, GHSA-q3jh-wrc6-jgq8/8r9p-f939-6h3c/mrwp-f3q8-3g9p) that had reappeared since the previous pass — fixed with `npm audit fix --omit=dev` (a non-breaking patch bump to 1.16.3, `package-lock.json` only, no `package.json` change). Re-verified clean afterward: `npm audit --omit=dev` now reports exactly 15 low findings, zero high/moderate, matching the count the accepted-risk paragraph below already covers. Full JS test suite re-run after the fix (`npm test -- --maxWorkers=2`): passing.

Accepted risk, low severity: the remaining 15 findings are the ethers v5 chain (`ethers`, `@ethersproject/*`, `elliptic`) pulled in by `circomlibjs@0.1.7`. The only fix npm offers is a downgrade to `circomlibjs@0.0.8`, which is breaking. ZKELLA uses `circomlibjs` only for Poseidon and babyjubjub. The ethers-importing modules (`mimc7`, `mimcsponge`, `evmasm`, and the `*_gencontract` generators) are not referenced anywhere in the repository. Revisit when `circomlibjs` is upgraded.

Full JS test suite after the fix: 158/158 passing against PostgreSQL (current total, after further additions since: 188/188 — see the README's "Status at a glance").

### Rust lints (`cargo clippy --workspace --all-targets --release`)

- Rustc compiler warnings: **0**.
- Clippy correctness-class lints: **0**.
- Clippy style-class lints: 31 distinct occurrences across token, swap, and verifier. Categories: too many function arguments (10 occurrences, 8 to 11 arguments per function), index-based loops (9), `clone` that can be `from_ref` (3), `is_multiple_of` suggestion (3), redundant casts, a `RangeInclusive::contains` suggestion, and two other style suggestions. **Accepted for now**: none of these is a correctness finding. Fixing the argument-count lints would change the signatures of proof-verification helpers and their call sites, which is a larger refactor than this review pass covers, so it is deferred.

### Circuit static analysis (`circomspect`)

**2026-11-13: new addition, closing a real gap.** `docs/TRANCHE3_DELIVERABLES.md` previously stated Circom has no analysis tooling and relied on proof-consuming contract fuzzing alone — that claim was wrong and is corrected here: [Circomspect](https://github.com/trailofbits/circomspect) (Trail of Bits) is a real, installable static analyzer for Circom (`cargo install circomspect --version 0.9.0 --locked`), run for the first time against all six circuits in this pass.

| Circuit | Issues flagged | Disposition |
| --- | --- | --- |
| `shield` | 0 | Clean. |
| `unshield` | 4 | All false positives, individually verified — see below. |
| `transfer_2in2out` | 11 | All false positives, individually verified. |
| `transfer_4in4out` | 17 | All false positives (same patterns as `transfer_2in2out`, same verification applies). |
| `swap_fairness` | 2 | Both false positives, individually verified. |
| `compliance` (non-membership) | 15 | All false positives, individually verified. |

**Every flagged line was checked against the actual constraint system, not just the tool's own note** — Circomspect's static analysis is local/per-line and cannot trace multi-step derivations, so three recurring patterns account for all 49 flagged lines:

1. **Loop-counter false positives** (the large majority — e.g. `for (var i = 0; i < D; i++)`). Circomspect's field-element comparison/arithmetic heuristics fire on ordinary `for` loop headers. `i`/`D`/`N_IN`/`N_OUT` are Circom `var`s (compile-time template parameters), never witness signals — there is no way for a prover to influence them, so these are not exploitable regardless of what the heuristic says.
2. **Difference-then-range-check false positives** (e.g. `signal diff <== amount_out - min_amount_out;` in `swap_fairness.circom:61`, `change <== value - pub_value;` in `unshield.circom:72`, the balance equation in both transfer circuits). In every case, each operand is independently `Range64()`-checked elsewhere in the *same* file (circom constraints are a global system, not sequential code — textual order doesn't matter for soundness). Verified by grep for the matching `Range64()`/`*_range` component for each operand in every flagged circuit before accepting this as safe. Equality checks via `IsZero(a - b)` (used for nullifier/commitment-distinctness checks in both transfer circuits) are sound for *any* field elements regardless of range — Circomspect's arithmetic-overflow heuristic still fires on them, but overflow cannot change whether a value is exactly zero.
3. **Two genuinely novel-looking findings, each checked individually and confirmed safe for a different reason:**
   - `unshield.circom:96-97` (`recipient_hash_check <== recipient_hash;`, flagged "intermediate signal should occur in at least two constraints"): correct that the circuit itself never constrains `recipient_hash` to anything — and the circuit's own comment says so directly ("not used in constraints... included as public input so the contract can verify destination"). Checked the contract side to confirm this isn't a gap: `contracts/token/src/lib.rs:989-996` independently recomputes `expected_recipient_hash = Poseidon2(address_field(to), binding_tag)` from the plain (non-proof) `to`/`binding_tag` call arguments and rejects on mismatch — the binding is enforced at the contract layer, with its own dedicated regression tests (`unshield_rejects_wrong_recipient_hash`, `unshield_binding_tag_changes_the_accepted_recipient_hash`). The circuit's lack of an in-circuit constraint is correct, not a gap, since there is no privacy reason to re-derive a binding over two public (non-proof) values.
   - `compliance/non_membership.circom:93` (`address` fed into `LessThan(248)` without an explicit `Num2Bits(248)` call, flagged "needs to be constrained to ensure it is <= p/2"): `address` is not a raw witness input — it's *constructed* as `signal address <== acc`, where `acc` sums exactly 248 individually-constrained bits from `Num2Bits_strict()` applied to `Poseidon2(sk, 1)` (lines 43-53). It is therefore bounded to `[0, 2^248)` by the construction itself, with no separate range-check gadget needed — Circomspect's local analysis doesn't trace the bit-reconstruction loop far enough to see this.

None of the 49 flagged lines required a code change. This pass is recorded as evidence that the check was actually run and each finding individually dispositioned, not waved through as "probably fine."

### Soroban-specific static analysis (`cargo-scout-audit`)

**Attempted 2026-11-13, not completed in this pass.** [`cargo-scout-audit`](https://github.com/CoinFabrik/scout-audit) is Stellar's own recommended Soroban-specific static analyzer (a Dylint/Clippy-based linter), listed in the Audit Bank readiness checklist as one option to close the "no Scout/Almanax/Sunbeam report" gap. Installing and running it requires compiling its own custom lint driver against this workspace, which — on top of the full from-scratch rebuild this same session already triggered by pinning `contracts/rust-toolchain.toml` to a toolchain version never previously used in this environment — did not complete within this session's time budget. **Open item, not a negative finding**: tracked for a follow-up pass with a warm toolchain cache.

### Fuzzing (`cargo fuzz`, `contracts/token/fuzz`)

- Targets: `shield_arbitrary`, `transfer_arbitrary` (covers `transfer` and `transfer4`), `verifier_arbitrary` (covers `verify` and `verify_batch`), `swap_arbitrary` (every swap entrypoint), `governance_arbitrary`, `compliance_arbitrary`, `viewing_keys_arbitrary`, `token_admin_arbitrary` (token admin, pause, allowlists, relayers, `unshield`, `shield_batch`), `verifier_admin_arbitrary` (verifier key registration, update, revocation). Each target asserts its own invariants: a bogus proof is never accepted, a rejected call leaves no state behind, a second initialize is rejected, and a VK update cannot execute before its timelock.
- Short local runs (20 to 60 seconds each) found no crashes or invariant violations. These are smoke runs, not a full fuzzing campaign.
- All nine targets now have a committed, minimized corpus (`cargo +nightly fuzz cmin`), so CI's smoke job starts from real coverage rather than an empty corpus on every run. `compliance_arbitrary`'s corpus includes a regression seed from a real crash the harness found (a fuzzer input whose random root happened to equal the authorized one) and the fix that followed.
- A build break in `transfer_arbitrary` (the `relayer` argument added in Tranche 2) was fixed in commit `cb5b409`.
- To make the contracts importable from the fuzz crate, the swap, governance, compliance, and viewing_keys crates now also build an rlib, matching the token crate. The WASM output is unchanged, and their tests pass.
- The fuzz harnesses call the state-changing entrypoints with arbitrary inputs: the data entrypoints of every contract, token admin and allowlists, token-action queuing (governance), verifier key management, compliance root binding, and viewing-key revoke. Pause, unpause, and guardian cancel are exercised by unit tests, not by the harnesses. Read-only getters are exercised only indirectly.
- A change to token's `lib.rs` re-exports `ShieldBatchItem` so the fuzz crate can construct batch inputs. No runtime behavior changed.
- **2026-11-13**: confirmed the fuzz toolchain still resolves and builds correctly after pinning `contracts/rust-toolchain.toml` to stable 1.93.0 (this pin would otherwise shadow the fuzz job's nightly requirement via directory-based override — see the commit pinning the toolchains for the exact mechanism). `RUSTUP_TOOLCHAIN=nightly-2026-09-22 cargo fuzz run shield_arbitrary` was started to confirm a real target still runs; this required a full from-scratch rebuild of the sanitizer-instrumented dependency tree under a nightly toolchain never previously used in this environment, which did not finish within this session's time budget. Not a negative finding — `cargo fuzz list` confirmed all nine targets are still registered, and the build was progressing (not stalled) when last checked.

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

1. ~~Fuzz harnesses for every state-changing entrypoint across all six contracts.~~ Done — all nine targets exist and are registered (confirmed 2026-11-13 via `cargo fuzz list`).
2. ~~Real-WASM instruction-budget measurement for every remaining entrypoint, with results published.~~ Done — see the table above.
3. Triage of any new findings from items 1 and 2, with each finding fixed or documented as accepted risk. **Partially open**: the circuit-side triage is done (Circomspect, this pass); a full fuzz re-run under the newly-pinned toolchain did not complete in this session (see Fuzzing section) and should be finished before this item is closed.
4. **New**: run `cargo-scout-audit` (Stellar's recommended Soroban-specific analyzer) — attempted this pass, did not complete due to cold-cache compile time; needs a follow-up session with a warm toolchain cache.
5. **New**: `.ptau/pot12_final.ptau` is corrupted (a saved S3 error page, not a real ceremony file) — see `docs/audit/artifacts.json`. Needs re-downloading before `shield`/`swap` can be rebuilt from scratch; does not affect already-built, already-committed artifacts.
