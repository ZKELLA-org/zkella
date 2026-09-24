# Tranche 1 deliverables: success-criteria audit

Every success criterion of the six Tranche 1 deliverables, quoted from the roadmap, with the verdict, the explanation, and the proof (a test you can run, a transaction you can open, or a measurement). Where a criterion is met with a deviation or a limit, that is stated next to it.

Verdicts: **Met** (the criterion as written is satisfied), **Met, with a deviation** (satisfied in substance, the method differs from the literal wording, explained), **Partly met** (part of the criterion is not achievable or not done, explained).

## How to reproduce

```
cd contracts && cargo build --workspace --target wasm32v1-none --release && cargo test --workspace --release
npm test                                   # JS unit tests (circuits, SDK, wallet, indexer)
npm run check:browser-worker               # real-browser Web Worker check (needs a local Chromium)
cd contracts/token && cargo +nightly fuzz run shield_arbitrary   # also transfer_arbitrary, verifier_arbitrary
cd contracts && cargo llvm-cov -p zkella-token --release --summary-only
```

Test totals at the time of writing: token 111, verifier 25, swap 12, governance 4, compliance 3, viewing keys 2 (157 in the Rust workspace, all passing); 128 JS unit tests, all passing. Live evidence is in `docs/TESTNET_DEPLOYMENT.md`.

Current Testnet stack (built from the current source): verifier `CBBKTJ4FHPDZRVQO6OQZDRHSKPVN7NVRZRZXQQQWW6BKDR57JKQDAE22`, token `CDQ53BGUQA6K5E6VIUR23D7P4R6FUVBUOXVS6XSQ256ZB4TBDOZIEVRM`, swap `CA5S2JRD3OFNI7RZSGKPN3AUKVQRWDBNHFPWNYEOSTBCD7D4QVT6BTM3`, compliance `CC55I2ZEZRQLZ4VZN2OLCSAPBLPCI3XPYRTHKNI7QOIKWRU652GYNTUK`. Its verifier is administered directly by the deployer, so it validates the proving and verification path, not governance's timelock (that ran on the earlier stack in the same document).

---

## Deliverable 1: Asset custody and commitment logic

**1.1 Custody and commitment-creation logic builds successfully and deploys to Stellar Testnet. Met.**
The contracts build for `wasm32v1-none` in CI (`cargo build --workspace --target wasm32v1-none --release`) and are deployed on Testnet (addresses above). Proof: `docs/TESTNET_DEPLOYMENT.md`; four shields on the fresh deployment: `2a9d480b...`, `2c8594eb...`, `4de68504...`, `4954a5cf...`.

**1.2 Commitment generation is validated against representative shield inputs, including duplicate-commitment rejection. Met.**
`cm = H(H(H(value, asset), H(rho, rcm)), pk)` is computed inside the contract, compared with the commitment argument, and independently cross-checked against circomlibjs and the compiled circuit (`poseidon.rs`: `note_commitment_matches_real_shield_circuit_v2_500stroops_vector`; `commitment.test.ts`: three vectors plus the circuit's own witness). Duplicates are rejected with `DuplicateCommitment` (`shield_rejects_duplicate_commitment`, `shield_replay_by_different_caller_rejected_at_duplicate_check`, a duplicate inside a batch in `tests/shield_flow.rs`). Representative inputs, zero and negative amounts, amounts below the minimum, oversized values and wrong note lengths (0, 175, 177 bytes) each assert the exact error and that state is unchanged.

**1.3 MIN_SHIELD_AMOUNT is a governance-settable parameter rather than a hardcoded constant, confirmed by changing it without a redeploy. Met.**
`set_min_shield_amount` (admin-gated) and `min_shield_amount` replace the constant; non-positive values are rejected. Proof, live: the minimum was read as 1,000, raised to 2,000,000 (`db1b3a9c...`), a shield below it was rejected by the contract, and it was restored (`12236807...`), all on the same deployed contract. Unit test: `min_shield_amount_is_governance_settable_without_redeploy`.

**1.4 A documented, implemented policy exists for what happens to shielded notes if a non-native asset's issuer claws back the contract's custodied balance. Met.**
Implemented in three layers (`docs/TECHNICAL_SPEC.md` section 6.1, `docs/RUNBOOK.md` category 5):
- *Prevention:* an asset must be explicitly approved by governance before it can be shielded (`set_asset_approved`); accepting an issuer's clawback right is an on-chain decision. Native XLM has no issuer.
- *Detection:* `custody_shortfall(asset)` = `max(0, shielded supply - custodied balance)`.
- *Loss sharing:* when the pool holds less than it owes, `unshield` pays `value * balance / supply`, so every holder shares the loss pro rata instead of the first to withdraw taking everything. `commit_swap` refuses to escrow a short payout, leaving the note unspent.
Proof: `unshield_shares_a_clawback_loss_pro_rata_and_reports_the_shortfall` runs a real Stellar Asset Contract with clawback enabled: 1,000,000 shielded, 400,000 clawed back, shortfall reported as 400,000, a 500,000 withdrawal pays 300,000, leaving 300,000 held against 500,000 owed (same 60% backing).
Limit: this is proven against the Soroban test environment's asset contract, not against a live clawback on Testnet.

**1.5 A batched multi-deposit entrypoint exists and is exercised against multiple simultaneous shield inputs in a single call. Met.**
`shield_batch` deposits several notes with one aggregated token transfer. Proof: `shield_batch_deposits_multiple_notes_in_one_call_with_one_aggregated_transfer` (three deposits, one transfer of the summed amount), `shield_batch_real_wasm_instruction_cost` (three items on the compiled WASM), and eleven rejection tests plus an atomicity test (a failure on item 2 leaves nothing from item 1 behind). The batch is capped at 3 items (`MAX_SHIELD_BATCH`) because three items already use 347.2M of the 400M limit.

---

## Deliverable 2: Merkle tree and events

**2.1 New commitments are correctly inserted into the Merkle tree, confirmed via a real Testnet transaction and published transaction hash. Met.**
Every shield transaction above inserts a leaf and returns its index (leaves 0 to 3 on the fresh stack). The root is checked against an independently computed root in `merkle_root_matches_independent_recomputation` (five leaves, pure-Rust reference, every level) and `merkle_path_reconstructs_the_current_root`.

**2.2 Shield-event emission is covered by passing tests. Met.**
`shield_emits_shield_and_note_events` asserts the `("zkella","shield")` and `("zkella","note")` events with the right leaf index, commitment and 176-byte encrypted note, for a single shield and for each item of a batch, in order. (An independent coverage review found events were previously not asserted anywhere; this closed that.)

**2.3 MerkleTreeFull is actually enforced against the leaf count, confirmed by a test that fills the tree to capacity and shows the next insert correctly rejected. Met, with a deviation.**
The check is enforced in `shield`, `shield_batch` and `transfer` (`merkle::is_full`, error `MerkleTreeFull`, before any state changes). The tree holds 2^32 - 1 leaves, so literally inserting four billion leaves is not possible in a test. The tests instead set the leaf counter to capacity through contract storage: `merkle_tree_full_is_rejected_gracefully_not_via_panic`, and `merkle_last_slot_boundary`, which shows the last slot (index `MAX_LEAVES - 1`) still succeeds and the next insert is rejected. This tests the boundary the criterion is about; it does not build a four-billion-leaf tree.

**2.4 Per-insert instruction cost is measured against a tree populated to a realistic scale of thousands of leaves, and published. Met, with a deviation.**
The network caps one transaction at 400 ledger entries, so thousands of real inserts cannot happen inside one test invocation. An insert only reads the left siblings on its path, so `merkle_insert_cost_and_correctness_at_thousands_of_leaves` builds a full 5,000-leaf tree in memory (independent pure-Rust Poseidon), writes exactly the boundary nodes a real 5,000-leaf tree would hold, and runs a real `shield` on the compiled WASM as leaf 5,000. Result: **118,635,905 instructions into an empty tree, 118,779,794 into the 5,000-leaf tree (+0.12%)**, and the resulting root equals the independently computed root of all 5,001 leaves. So insert cost is flat with tree size (it is a fixed 32-level path), which is what a realistic-scale measurement was meant to establish. Published here and in `docs/TESTNET_DEPLOYMENT.md`. Limit: the populated tree is constructed, not grown by 5,000 real transactions.

---

## Deliverable 3: Shared verifier registry contract

**3.1 The verifier contract builds successfully and deploys to Stellar Testnet. Met.** Address above; registration transactions in `docs/TESTNET_DEPLOYMENT.md`.

**3.2 It accepts only valid shield proofs and rejects invalid proofs when called from the token contract. Met.**
Accepts: five real circuit proofs verify (`verify_accepts_real_*`) and the token accepts a proof through the real verifier. Rejects: tampered proofs, wrong public inputs (`verify_rejects_*`), non-canonical inputs (`verify_rejects_non_canonical_public_input_encoding`, exact `NonCanonicalInput`), and at the token, exact `InvalidProof` for a corrupted proof and for a valid proof with a different public input (`tests/shield_flow.rs`).

**3.3 A shield transaction reaches the entrypoint and completes within Soroban's instruction budget, repeated at least three times in a fresh environment. Met.**
Four shield transactions on a freshly deployed verifier and token, each with a real proof: instructions declared 126.1M, 126.3M, 126.3M, 126.3M (limit 400M), so the result is consistent across all four. Measured on the compiled WASM in tests: 118.6M (`cost_parity_shield`).

**3.4 Results are published with transaction hashes, both contract addresses, and a resource/instruction-usage profile. Met.**
`docs/TESTNET_DEPLOYMENT.md` lists the hashes, both addresses, and a resource table (instructions, ledger entries, written entries, write bytes) for each live transaction, generated by `scripts/tx_resource_profile.cjs`. The profile also shows the layout is ready for the transfer and swap work: `transfer4` (378.7M), `unshield` (35.5M) and the swap and compliance transactions all ran on the same contracts.

**3.5 Public-input aggregation uses Soroban's native batched multi-scalar-multiplication host function, with the resulting instruction-cost reduction measured and published against the prior per-input loop. Met.**
`verify` computes `vk_x` with one `bn254_g1_msm` call. Proof: `msm_aggregation_is_cheaper_than_the_per_input_loop_on_transfer4` runs the heaviest entrypoint (19 public inputs) on the compiled WASM with the verifier as it was before the change (kept as `contracts/token/tests_data/verifier_pre_msm.wasm`, built from the pre-change source) and with the current one:

| Verifier | transfer4 instructions | vs 400M limit |
| --- | --- | --- |
| per-input loop (before) | 412,111,949 | over the limit (call aborts) |
| batched MSM (now) | 397,910,589 | under, 99.5% |

The MSM saves 14.2M instructions on this call and is what brings it under the limit. Honest note: the margin is thin (about 0.5%, 2.1M instructions); the live `transfer4` (real circuit) declared 378.7M. The test fails the build if the MSM verifier ever costs more than the loop or crosses 400M.

**3.6 update_verifying_key retains the pre-rotation key for a defined window, confirmed by a test proving a proof built against the old key still verifies during that window and the new key verifies immediately. Met.**
`update_verifying_key_retains_old_key_within_window_then_expires`: after a rotation, the new key's proof verifies at once, the old key's proof still verifies inside the 17,280-ledger (about one day) window, and stops verifying after it. `revoke_previous_vk_ends_the_retention_window_immediately` covers ending the window early, and governance exposes it (`revoke_previous_vk_is_admin_gated_and_forwards_to_the_verifier`).

**3.7 A batch-verification path exists for verifying multiple proofs in a single combined pairing check, with its instruction cost measured against verifying the same proofs individually. Met.**
`verify_batch` performs K+3 pairings instead of 4K. Its challenges hash the circuit and every item's public inputs and proofs (a review found challenges derived from the proof alone were weak; fixed). Measured on the compiled WASM by `verify_batch_cost_vs_individual_verification_on_real_wasm` (the same real proof repeated K times, so the comparison is like for like):

| Proofs | Individually (sum of `verify`) | `verify_batch` | Ratio |
| --- | --- | --- | --- |
| 2 | 58.4M | 43.4M | 0.74 |
| 4 | 116.7M | 63.0M | 0.54 |
| 8 | 233.4M | 102.3M | 0.44 |

Batching wins at every size measured and the advantage grows with K. Soundness tests: accepts valid batches, rejects a batch with one tampered item, rejects an empty batch, rejects non-canonical inputs.

---

## Deliverable 4: Shield circuit

**4.1 The shield circuit compiles and produces a correct witness for representative valid and invalid inputs. Met.**
Compiled with circom; `shield-circuit-negative.test.ts` generates a witness for a valid input (accepted, positive control) and for each invalid input (rejected), against the compiled circuit. Real proofs from it verify off-chain with snarkjs and on-chain (live shields).

**4.2 Constraint count and correctness are documented against the design's expected shape. Met.**
`docs/CIRCUIT_SPEC.md` section 2: 1,264 constraints (measured with `snarkjs r1cs info`), with the derivation. The design is five Poseidon2 hashes (four for the commitment, one for the value commitment) plus a 64-bit range check plus equalities: 5 x about 240 + 64 = about 1,264. A missing hash or range check would move the count by about 240 or 64, so the measured number matches the design.

**4.3 A structured negative-testing pass against deliberately malformed witnesses is documented, with every under-constrained path found fixed and covered by a regression test. Met.**
Documented in `docs/SECURITY_AUDIT_TRANCHE1.md` ("Shield circuit negative-testing pass"): seven malformed witnesses, each isolating one constraint, all rejected (forged commitment, forged value commitment, forged public amount, forged public asset, value 2^64, value near the field modulus, a commitment for a different owner). No under-constrained path was found in the shield circuit itself. The pass and the follow-up reviews did find under-constrained paths elsewhere, each fixed and covered by a regression test: the nullifier key not bound to the note in the spend circuits (`circuit-owner-binding.test.ts`), and the compliance circuit's non-membership soundness (`circuit-compliance.test.ts`).

**4.4 The Poseidon2 round-constant parameterization is confirmed, with a specific test case, to exactly match the Rust contract's native hash. Met.**
`note_commitment_matches_real_shield_circuit_v2_500stroops_vector` recomputes the commitment in Rust from the shield circuit's own test vector (value 500, the real testnet asset's field value, rho 3, rcm 4, the fixture's owner key) and requires it to equal the commitment inside a real proof of the compiled circuit. The contract's native hash is separately checked bit for bit against the pure-Rust implementation (`poseidon2_native_matches_pure_rust_*`) and against circomlibjs (`poseidon2_zero_zero_matches_circomlibjs`). Chained, the contract's hash equals the circuit's on real data, not merely a similar-looking parameter set. `token/src/lib.rs` also pins `address_to_field_bytes_and_commitment_match_real_testnet_asset` against a value computed independently with circomlibjs.

---

## Deliverable 5: WASM compilation and proof-generation library

**5.1 The shield circuit compiles to WASM and exports usable JavaScript bindings. Met.**
`circuits/shield/build/shield_js/shield.wasm` with circom's `witness_calculator.js`, wrapped by `sdk/src/prover/shield.ts` (`generateShieldProof`), built by `npm run build --workspace=sdk`. The browser check below loads it in Chromium.

**5.2 The proof-generation library creates valid Groth16 proofs for representative shield inputs, verified against the deployed Testnet contracts, not only a local harness. Met.**
The four live shields above each carry a proof from `generateShieldProof`, verified by the deployed verifier on-chain. Off-chain checks (`prover-worker.test.ts` verifies with snarkjs) are additional, not the only ones.

**5.3 A documented example demonstrates end-to-end proof generation and on-chain verification, usable by the SDK to construct a valid shield transaction. Met.**
`docs/INTEGRATION_GUIDE.md` section 7 documents the example and the runnable reference `scripts/testnet_live_validation.cjs` (four shields, `transfer4`, `unshield`, each proof verified on-chain), with the exact command and the code path `wallet.shield` follows.

**5.4 Witness generation runs off the main JavaScript thread via a Web Worker, confirmed by the page remaining responsive during proof generation. Met.**
`sdk/src/prover/worker.ts` is a browser Web Worker entry point. `scripts/browser_worker_check.mjs` bundles it with esbuild, serves a page with the circuit artifacts, drives headless Chromium and measures the page's worst main-thread stall while a real shield proof is generated: about 205 ms inline versus about 19 ms with the worker (one run; `npm run check:browser-worker`). Why a browser and not Node: Node's `worker_threads` cannot run snarkjs (its `web-worker` dependency crashes on import), so the worker targets browsers, where it runs. Limit: the check is a script that needs a local Chromium, not part of CI.

**5.5 generateTransfer4Proof, generateUnshieldProof and the swap-fairness generator have each produced a proof used in a fresh, individually-submitted and verified live transaction. Met.**
Each is its own live transaction on the current stack (`docs/TESTNET_DEPLOYMENT.md`):
- `generateTransfer4Proof`: `transfer4`, tx `a7858b39...` (19 public signals, 378.7M instructions).
- `generateUnshieldProof`: `unshield`, tx `668fa5bf...`.
- swap-fairness generator: `reveal_and_claim`, tx `ddc95005...`, verifying a real swap-fairness proof (with the real unshield ownership proof in `commit_swap` `a90cb778...` and a real shield proof for the output note).
Limit: these ran on a validation deployment whose verifier is administered by the deployer rather than through governance's timelock.

---

## Deliverable 6: Shield flow test suite

**6.1 All shield flow unit and integration tests pass. Met.** 157 Rust tests and 128 JS tests, all passing (commands above).

**6.2 Coverage includes valid proofs, invalid proofs, edge cases, Merkle consistency, duplicate-commitment rejection, contract state transitions, and event emission. Met.**
An independent reviewer (not the author) mapped each category to tests and found gaps: event emission and auth were untested, zero and maximum amounts were missing, many `shield_batch` and spend-path error branches were unexecuted, and several tests only asserted `is_err()`. All were closed with 49 new tests (`tests/shield_flow.rs`, `tests/spend_paths.rs`) that assert exact errors and unchanged state: valid and invalid proofs (exact `InvalidProof`, wrong public input), zero, negative, `i128::MAX`, 2^64+1 and overflowing amounts, Merkle consistency against an independent root, duplicates, pause / admin-transfer / allowlist / minimum-amount transitions, events, and authorisation without `mock_all_auths`.

**6.3 Integration tests exercise the on-chain verifier with proofs generated from the WASM circuit library, with a published coverage report. Met.**
The verifier's tests verify proofs produced by the compiled circuits and the SDK-side encoders (`verify_accepts_real_*`, plus the SDK's encoding tests against the same proof files), and the live transactions exercise the deployed verifier with SDK-generated proofs. Coverage report: `docs/COVERAGE.md` (token crate: 99.66% of regions, 99.68% of lines, 93.15% of functions; `lib.rs` 100% of lines).

**6.4 Fuzz testing has been run against the shield flow, with any findings triaged and either fixed or explicitly documented as accepted risk. Met, with a limit.**
Three cargo-fuzz targets in `contracts/token/fuzz`: `shield_arbitrary`, `transfer_arbitrary`, `verifier_arbitrary`. Invariants: junk proofs and inputs are never accepted, and every rejected call leaves the leaf count, root, supply and nullifiers unchanged. A five-minute run per target on an idle machine executed 4,007, 2,011 and 20,693 inputs with no crash and no invariant break; an earlier run of about 85,000 executions also found nothing. CI runs a 60-second smoke per target on nightly. Triage: nothing to fix. Limit, stated as accepted: this is a modest run, not a long campaign; each execution builds a full contract environment, so throughput is low (single digits to tens of executions per second for the token targets). There is no committed corpus and no swap or governance target.

**6.5 The published coverage report's adequacy has been independently reviewed, not only generated, with any gap either closed or documented. Met.**
An independent review assessed `docs/COVERAGE.md` and found it not adequate as first published (no itemised gaps, `types.rs` omitted, no assessment of assertion quality, no event, auth or extreme-value tests). The gaps were closed by the 49 tests above, and the remaining uncovered code is documented in `docs/COVERAGE.md`: derive glue in `types.rs` and one branch of a test-only helper. Line coverage is not correctness; the report says so and lists what coverage cannot show.

**6.6 The real-WASM-versus-native-Rust instruction-cost comparison runs automatically in CI for every entrypoint, not just shield, transfer, and transfer4, and fails the build on a material discrepancy. Met, with a scope note.**
`contracts/token/src/tests/cost_parity.rs` runs each entrypoint twice, native and on the compiled WASM, prints both figures, and fails the build if the WASM cost exceeds 400M or is more than 25% above native. `cost_parity_verify_and_verify_batch` does the same for the verifier's two verification entrypoints. CI builds the WASM and runs `cargo test --workspace`, so these run on every push. Measured (native / WASM):

| Entrypoint | Native | Compiled WASM | Gap | Of 400M |
| --- | --- | --- | --- | --- |
| `shield` | 103.7M | 118.6M | +14% | 30% |
| `shield_batch` (3) | 306.2M | 347.2M | +13% | 87% |
| `transfer` (2 in, 2 out) | 203.2M | 233.8M | +15% | 58% |
| `transfer4` (4 in, 4 out) | 343.4M | 397.9M | +16% | 99.5% |
| `unshield` | 31.0M | 34.1M | +10% | 8.5% |
| verifier `verify` | 28.5M | 29.2M | +2% | 7% |
| verifier `verify_batch` (4) | 61.4M | 63.0M | +3% | 16% |

Scope note: "every entrypoint" is taken as every entrypoint that verifies a proof or mutates the Merkle tree, which are the ones that can approach the limit: the five token entrypoints and the verifier's two. The swap and compliance entrypoints that verify proofs do so by calling `token.unshield` / `token.shield` or `verifier.verify`, which are measured above; their own live costs are in the resource profile (`commit_swap` 44.0M, `reveal_and_claim` 158.5M, `publish_compliance_proof` 30.4M). The remaining entrypoints are constant-time administration calls with no proof or tree work.

---

## What is not done, and what is thin

- **`transfer4` margin.** 397.9M of 400M on the compiled WASM (0.5%). It passes and the build guards it, but any additional on-chain work in that entrypoint will need a further optimisation first. The live transaction declared 378.7M.
- **Clawback:** proven against the Soroban test environment's asset contract, not by a live clawback on Testnet.
- **Fuzzing:** a modest run and a CI smoke, not a long campaign.
- **Browser check:** a script needing a local Chromium; not in CI.
- **Governance timelock** was exercised on the earlier Testnet stack, not on the current validation stack.
- **Development keys:** the circuit keys come from a single-contributor development ceremony; a real multi-party ceremony is required before mainnet. Nothing here is deployed to mainnet.

## Beyond the roadmap

Two review rounds (`docs/SECURITY_AUDIT_TRANCHE1.md`) found and fixed: the unbound nullifier key, non-canonical public-input aliasing, weak batch challenges, an unsound compliance circuit, and a swap front-running path; accepted risks are listed there.
