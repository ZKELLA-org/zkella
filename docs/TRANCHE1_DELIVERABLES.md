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

Test totals at the time of writing: token 118, verifier 30, swap 13, governance 5, compliance 4, viewing keys 2 (172 in the Rust workspace, all passing); 128 JS unit tests, all passing. Live evidence is in `docs/TESTNET_DEPLOYMENT.md`.

Current Testnet stack (built from the current source, including the instruction-cost optimisations): verifier `CBHQUNPD42ZODQWCEK2SKLAARHHY75SGCVWHW6QLWLGLXWJ5JS2QORUY`, token `CDDM46ZV3KLULXUGUOWSCR5BGZ6BC5XJDDMVTV4JXOLZBJXD6EQCJ75Q`, swap `CB7TRLNTX6G3QNVDTHQHL46VNDQMMUUE4ZM5O6AIFFU6PWKGPKIQ7PYY`, compliance `CAUZB3RTW23QQ5CT6W7KLINZDYVO56DSUZQ5AHKL56KWBH64QD5LNA3Q`. Its verifier is administered directly by the deployer, so it validates the proving and verification path, not governance's timelock (that ran on the earlier stack in `docs/TESTNET_DEPLOYMENT.md`).

---

## Deliverable 1: Asset custody and commitment logic

**1.1 Custody and commitment-creation logic builds successfully and deploys to Stellar Testnet. Met.**
The contracts build for `wasm32v1-none` in CI (`cargo build --workspace --target wasm32v1-none --release`) and are deployed on Testnet (addresses above). Token deploy transaction (found the same way as 3.1's verifier/swap/compliance deploys): https://stellar.expert/explorer/testnet/tx/6109d7abf6fa1038bebc1eb372ca5d2114c3049cfd254a2c2089ebd788e1f165. Proof: `docs/TESTNET_DEPLOYMENT.md`; four shields on the fresh deployment:
https://stellar.expert/explorer/testnet/tx/e51f3be33e335917b663cb1969a7d9812cbac1c5e97e797c7fb2cdcb43abd2b1,
https://stellar.expert/explorer/testnet/tx/048f02332f51a0f5efe99c40e01f4b80b073d61755328630c4b28637d150b084,
https://stellar.expert/explorer/testnet/tx/0ac97d14310a692a35a7a3c9da71bc84e03e96eed0dcf0c97120a52df2183700,
https://stellar.expert/explorer/testnet/tx/244b995070978701a382202355d76c352d63e1b150092541da77d8c9a4910b1c.

**1.2 Commitment generation is validated against representative shield inputs, including duplicate-commitment rejection. Met.**
`cm = H(H(H(value, asset), H(rho, rcm)), pk)` is computed inside the contract, compared with the commitment argument, and independently cross-checked against circomlibjs and the compiled circuit (`poseidon.rs`: `note_commitment_matches_real_shield_circuit_v2_500stroops_vector`; `commitment.test.ts`: three vectors plus the circuit's own witness). Duplicates are rejected with `DuplicateCommitment` (`shield_rejects_duplicate_commitment`, `shield_replay_by_different_caller_rejected_at_duplicate_check`, a duplicate inside a batch in `tests/shield_flow.rs`). Representative inputs, zero and negative amounts, amounts below the minimum, oversized values and wrong note lengths (0, 175, 177 bytes) each assert the exact error and that state is unchanged. The same formula computes real on-chain commitments in all four live shields under 1.1, e.g. https://stellar.expert/explorer/testnet/tx/e51f3be33e335917b663cb1969a7d9812cbac1c5e97e797c7fb2cdcb43abd2b1.

**1.3 MIN_SHIELD_AMOUNT is a governance-settable parameter rather than a hardcoded constant, confirmed by changing it without a redeploy. Met.**
`set_min_shield_amount` (admin-gated) and `min_shield_amount` replace the constant; non-positive values are rejected. Proof, live, on the current Testnet stack's token (`CDDM46ZV3KLULXUGUOWSCR5BGZ6BC5XJDDMVTV4JXOLZBJXD6EQCJ75Q`): the minimum was read as 1,000, raised to 2,000,000
(https://stellar.expert/explorer/testnet/tx/09a8a09fc0efe4437b314c38ad6d565480573bc2806416d5a83a70c6a371bbff),
confirmed read back as 2,000,000, and restored to 1,000
(https://stellar.expert/explorer/testnet/tx/fb11dff8c2083c051034707466cf599d32db830f7448d5ea0313194611ca6ae2),
all without a redeploy. Unit test: `min_shield_amount_is_governance_settable_without_redeploy`.
(An earlier version of this evidence cited two transactions against an untracked, non-current contract; replaced here with fresh transactions against the actual current stack.)

**1.4 A documented, implemented policy exists for what happens to shielded notes if a non-native asset's issuer claws back the contract's custodied balance. Met.**
Implemented in three layers (`docs/TECHNICAL_SPEC.md` section 6.1, `docs/RUNBOOK.md` category 5):
- *Prevention:* an asset must be explicitly approved by governance before it can be shielded (`set_asset_approved`); accepting an issuer's clawback right is an on-chain decision. Native XLM has no issuer.
- *Detection:* `custody_shortfall(asset)` = `max(0, shielded supply - custodied balance)`.
- *Loss sharing:* when the pool holds less than it owes, `unshield` pays `value * balance / supply`, so every holder shares the loss pro rata instead of the first to withdraw taking everything. `commit_swap` refuses to escrow a short payout, leaving the note unspent.
Proof: `unshield_shares_a_clawback_loss_pro_rata_and_reports_the_shortfall` runs a real Stellar Asset Contract with clawback enabled: 1,000,000 shielded, 400,000 clawed back, shortfall reported as 400,000, a 500,000 withdrawal pays 300,000, leaving 300,000 held against 500,000 owed (same 60% backing).
Limit: this is proven against the Soroban test environment's asset contract, not against a live clawback on Testnet.

**1.5 A batched multi-deposit entrypoint exists and is exercised against multiple simultaneous shield inputs in a single call. Met.**
`shield_batch` deposits several notes with one aggregated token transfer. Proof: `shield_batch_deposits_multiple_notes_in_one_call_with_one_aggregated_transfer` (three deposits, one transfer of the summed amount), `shield_batch_real_wasm_instruction_cost` (eight items on the compiled WASM), eleven rejection tests plus an atomicity test (a failure on item 2 leaves nothing from item 1 behind), and live on Testnet: one transaction depositing eight real shield proofs
(https://stellar.expert/explorer/testnet/tx/22e3c4e31121a045f319e965a04761edca3d527f3dfb077423aaf0e5eac5964d,
`scripts/testnet_shield_batch.cjs`). The batch is capped at 8 items (`MAX_SHIELD_BATCH`): 8 items use 314M of the 400M limit on the compiled WASM (335M declared live), and a size sweep from 1 to 8 (`shield_batch_size_sweep_on_real_wasm`) fails the build if the cap is raised past 85% of the limit.

---

## Deliverable 2: Merkle tree and events

**2.1 New commitments are correctly inserted into the Merkle tree, confirmed via a real Testnet transaction and published transaction hash. Met.**
Every shield transaction above inserts a leaf and returns its index (leaves 0 to 3 on the fresh stack). The root is checked against an independently computed root in `merkle_root_matches_independent_recomputation` (five leaves, pure-Rust reference, every level) and `merkle_path_reconstructs_the_current_root`.

**2.2 Shield-event emission is covered by passing tests. Met.**
`shield_emits_shield_and_note_events` asserts the `("zkella","shield")` and `("zkella","note")` events with the right leaf index, commitment and 176-byte encrypted note, for a single shield and for each item of a batch, in order. (An independent coverage review found events were previously not asserted anywhere; this closed that.) Confirmed on-chain: `scripts/verify_onchain_evidence.cjs` reads back real events from the 4 live shields under 1.1, e.g. https://stellar.expert/explorer/testnet/tx/048f02332f51a0f5efe99c40e01f4b80b073d61755328630c4b28637d150b084.

**2.3 MerkleTreeFull is actually enforced against the leaf count, confirmed by a test that fills the tree to capacity and shows the next insert correctly rejected. Met.**
The check is enforced in `shield`, `shield_batch` and `transfer` (`merkle::has_capacity`, error `MerkleTreeFull`, before any state changes). The tree holds 2^32 - 1 leaves, so literally shielding four billion notes is not possible in a test; `tree_filled_to_capacity_by_real_inserts_then_rejects_every_insert` instead lowers capacity to 20 through a test-only override (`merkle::TEST_CAPACITY`, a thread-local `Cell` compiled only under `#[cfg(test)]`; production always uses the real `MAX_LEAVES = u32::MAX`) and fills every one of those 20 slots with real `shield`/`shield_batch` calls — two batches of 8, three singles, and a final single into the last slot — never writing the leaf counter directly. It checks that a 2-item batch is refused with `MerkleTreeFull` when only 1 slot remains (and changes nothing), that the last slot still succeeds, and that both a single shield and a batch of one are refused once full, with the root unchanged. The boundary logic exercised (the `has_capacity` arithmetic, the last-slot accept, the over-capacity reject) is identical code to the real `MAX_LEAVES`; only the capacity constant differs. `merkle_tree_full_is_rejected_gracefully_not_via_panic` and `merkle_last_slot_boundary` cover the same boundary at the real `MAX_LEAVES` by setting the leaf counter directly, as a second, storage-level check.

**2.4 Per-insert instruction cost is measured against a tree populated to a realistic scale of thousands of leaves, and published. Met.**
The network caps one transaction at 400 ledger entries, so thousands of real inserts cannot happen inside one test invocation; `merkle_insert_cost_and_correctness_at_thousands_of_leaves` instead builds a full 5,000-leaf tree in memory (independent pure-Rust Poseidon), writes exactly the boundary nodes a real 5,000-leaf tree would hold, and runs a real `shield` on the compiled WASM as leaf 5,000: **76,355,580 instructions into an empty tree, 76,941,303 into the 5,000-leaf tree (+0.77%)**, root verified against an independent recomputation of all 5,001 leaves.

This was then confirmed with real transactions: `scripts/testnet_scale_run.cjs` ran 171 further live `shield_batch` (8-item) calls against the Tranche 1 stack, growing `leaf_count` from 18 to **1,362** (`node scripts/verify_onchain_evidence.cjs`), and later activity has grown it further still. Comparing the resource profile (`scripts/tx_resource_profile.cjs`) of an early such batch against one re-run at the current scale (both real `shield_batch` calls of 8 items each, so directly comparable):

| Batch | Leaf count before | Instructions | Tx |
| --- | --- | --- | --- |
| Low leaf count | 10 | 334,977,222 | https://stellar.expert/explorer/testnet/tx/22e3c4e31121a045f319e965a04761edca3d527f3dfb077423aaf0e5eac5964d |
| High leaf count | 1394 | 334,283,407 | https://stellar.expert/explorer/testnet/tx/1d48e2bc2717ba491496464a8863abe7f6715f62b85f4cc1c60db60ece93ec1f |

(The batch that originally established the flat-cost result at 1,354 leaves is superseded by the second row above — its own full hash was only ever printed to a terminal during that earlier live run and was never saved to a file, so it's replaced here with a freshly re-measured transaction rather than left as an unverifiable prefix.)

Cost is flat (within measurement noise) between an empty tree and one with over a thousand real leaves, on-chain, matching the synthetic 5,000-leaf test's conclusion that insert cost does not grow with tree size (a fixed 32-level path; the tiny synthetic-test increase is the extra sibling reads at a specific, deliberately unfavourable index). The insert itself was optimised while doing this work (see "Instruction-cost optimisation" below); `insert_many_matches_an_independent_tree_for_every_batch_size_and_alignment` proves the optimised insert stores the same tree as inserting leaf by leaf.

---

## Deliverable 3: Shared verifier registry contract

**3.1 The verifier contract builds successfully and deploys to Stellar Testnet. Met.** Address above; registration transactions in `docs/TESTNET_DEPLOYMENT.md`. Not separately recorded in that doc, so independently located by deriving each `CreateContract` operation's resulting contract address from the deployer's own on-chain operation history (deployer address + salt, per Soroban's standard contract-ID derivation) and matching it against the known stack addresses — the actual deploy transactions for all four contracts on this stack:
- verifier: https://stellar.expert/explorer/testnet/tx/d39e77e45987670025efeba775139a25bd6e35e54d6ddd474fcf117b8b294e45
- token: https://stellar.expert/explorer/testnet/tx/6109d7abf6fa1038bebc1eb372ca5d2114c3049cfd254a2c2089ebd788e1f165
- swap: https://stellar.expert/explorer/testnet/tx/58b04a2b9685e0693d66a27e9cd1eb342f47391cb183ea4f224068ec48d42e8b
- compliance: https://stellar.expert/explorer/testnet/tx/5a66fc970032598a482e5af54c369682bb470239b53bdc8af3906c41a5241703

**3.2 It accepts only valid shield proofs and rejects invalid proofs when called from the token contract. Met.**
Accepts: five real circuit proofs verify (`verify_accepts_real_*`) and the token accepts a proof through the real verifier. Rejects: tampered proofs, wrong public inputs (`verify_rejects_*`), non-canonical inputs (`verify_rejects_non_canonical_public_input_encoding`, exact `NonCanonicalInput`), and at the token, exact `InvalidProof` for a corrupted proof and for a valid proof with a different public input (`tests/shield_flow.rs`).

**3.3 A shield transaction reaches the entrypoint and completes within Soroban's instruction budget, repeated at least three times in a fresh environment. Met.**
Four shield transactions on the same freshly deployed verifier and token, each with a real proof: instructions declared 80.6M, 81.1M, 81.1M, 81.2M (limit 400M), consistent across all four —
https://stellar.expert/explorer/testnet/tx/e51f3be33e335917b663cb1969a7d9812cbac1c5e97e797c7fb2cdcb43abd2b1,
https://stellar.expert/explorer/testnet/tx/048f02332f51a0f5efe99c40e01f4b80b073d61755328630c4b28637d150b084,
https://stellar.expert/explorer/testnet/tx/0ac97d14310a692a35a7a3c9da71bc84e03e96eed0dcf0c97120a52df2183700,
https://stellar.expert/explorer/testnet/tx/244b995070978701a382202355d76c352d63e1b150092541da77d8c9a4910b1c.
Measured on the compiled WASM in tests: 76.4M (`cost_parity_shield`).

**3.4 Results are published with transaction hashes, both contract addresses, and a resource/instruction-usage profile. Met.**
`docs/TESTNET_DEPLOYMENT.md` lists the hashes, both addresses, and a resource table (instructions, ledger entries, written entries, write bytes) for each live transaction, generated by `scripts/tx_resource_profile.cjs`. The profile also shows the layout is ready for the transfer and swap work: `transfer4` (86.3M, https://stellar.expert/explorer/testnet/tx/15cbeef9533724df6ea96d3e96152255039664b11dac8640dd3d4c01370678ba), `unshield` (34.4M, https://stellar.expert/explorer/testnet/tx/c99b6b23dd068d3c12c77697cb614efa06c805349233716851efc85a22f80891) and the swap and compliance transactions all ran on the same contracts.

**3.5 Public-input aggregation uses Soroban's native batched multi-scalar-multiplication host function, with the resulting instruction-cost reduction measured and published against the prior per-input loop. Met.**
`verify` computes `vk_x` with one `bn254_g1_msm` call. Proof: `msm_aggregation_is_cheaper_than_the_per_input_loop_on_transfer4` runs the heaviest entrypoint (19 public inputs) on the compiled WASM with the verifier as it was before the change (kept as `contracts/token/tests_data/verifier_pre_msm.wasm`, built from the pre-change source) and with the current one:

| Verifier | transfer4 instructions |
| --- | --- |
| per-input loop (before) | 96,157,278 |
| batched MSM (now) | 80,819,477 |

The MSM saves 15.3M instructions (16%) on this call. When it first shipped, transfer4 was dominated by tree hashing (see below) and the loop verifier put it over the limit (412.1M against 397.9M with the MSM), which is why it was the deciding optimisation then; after the tree-hashing fix both figures are far under the limit and the saving is unchanged. The test fails the build if the MSM verifier ever costs more than the loop.

**3.6 update_verifying_key retains the pre-rotation key for a defined window, confirmed by a test proving a proof built against the old key still verifies during that window and the new key verifies immediately. Met.**
`update_verifying_key_retains_old_key_within_window_then_expires`: after a rotation, the new key's proof verifies at once, the old key's proof still verifies inside the 17,280-ledger (about one day) window, and stops verifying after it. This rotation and retention behaviour is unit-tested; the live `queue_vk_update` → wait → `execute_vk_update` path ran for real on the earlier Testnet stack's governance contract (`docs/TESTNET_DEPLOYMENT.md`), not re-run on the current one:
- Shield: queue https://stellar.expert/explorer/testnet/tx/cc4809befb3742c283f612cc061e9006722968e3ec8005ae8375fe1074af3201 → execute https://stellar.expert/explorer/testnet/tx/0131928d88132a34d1e79f8ad262389e5e83095fe61b281d64517a24ff990d42
- Unshield: queue https://stellar.expert/explorer/testnet/tx/1c6f4870fa52218c760e7581e0f71be985f3eabe7dc1a94e3f56b852b448290d → execute https://stellar.expert/explorer/testnet/tx/41449cbea7e8bd32a17c6cf97d18ffa320b38b58310bfae088ed5a0b26bee20f
- SwapFairness: queue https://stellar.expert/explorer/testnet/tx/f253fad14be8f7c1797840c45fa07eede32321a4bb204d77311812e4bacf9c8d → execute https://stellar.expert/explorer/testnet/tx/a2ca1fbf2d8b2a3d5b3a0747fbc4a85fc9457f618d3d341dfbaab78aee3ad372

`revoke_previous_vk_ends_the_retention_window_immediately` covers ending the window early, and governance exposes it (`revoke_previous_vk_is_admin_gated_and_forwards_to_the_verifier`).

**3.7 A batch-verification path exists for verifying multiple proofs in a single combined pairing check, with its instruction cost measured against verifying the same proofs individually. Met.**
`verify_batch` performs K+3 pairings instead of 4K. Its challenges hash the circuit and every item's public inputs and proofs (a review found challenges derived from the proof alone were weak; fixed). Measured on the compiled WASM by `verify_batch_cost_vs_individual_verification_on_real_wasm` (the same real proof repeated K times, so the comparison is like for like):

| Proofs | Individually (sum of `verify`) | `verify_batch` | Ratio |
| --- | --- | --- | --- |
| 2 | 56.0M | 43.4M | 0.77 |
| 4 | 112.1M | 63.1M | 0.56 |
| 8 | 224.2M | 102.3M | 0.46 |

Batching wins at every size measured and the advantage grows with K. The cost table above measures the same real proof repeated K times (a like-for-like comparison against K individual `verify` calls); `verify_batch_accepts_three_distinct_genuine_proofs` additionally checks that three genuinely different, independently-generated proofs (different value/rho/rcm each) batch-verify correctly together, and `verify_batch_rejects_when_one_of_several_distinct_proofs_is_mismatched` checks that a batch of otherwise-distinct proofs still fails when one item's proof and public inputs don't match. Further soundness tests: accepts valid batches, rejects a batch with one tampered item, rejects an empty batch, rejects non-canonical inputs.

---

## Deliverable 4: Shield circuit

**4.1 The shield circuit compiles and produces a correct witness for representative valid and invalid inputs. Met.**
Compiled with circom; `shield-circuit-negative.test.ts` generates a witness for a valid input (accepted, positive control) and for each invalid input (rejected), against the compiled circuit. Real proofs from it verify off-chain with snarkjs and on-chain (live shields).

**4.2 Constraint count and correctness are documented against the design's expected shape. Met.**
`docs/CIRCUIT_SPEC.md` section 2: 1,264 constraints (measured with `snarkjs r1cs info`), with the derivation. The design is five Poseidon2 hashes (four for the commitment, one for the value commitment) plus a 64-bit range check plus equalities: 5 x about 240 + 64 = about 1,264. A missing hash or range check would move the count by about 240 or 64, so the measured number matches the design.

**4.3 A structured negative-testing pass against deliberately malformed witnesses is documented, with every under-constrained path found fixed and covered by a regression test. Met.**
Documented in `docs/SECURITY_AUDIT_TRANCHE1.md` ("Shield circuit negative-testing pass"): seven malformed witnesses, each isolating one constraint, all rejected (forged commitment, forged value commitment, forged public amount, forged public asset, value 2^64, value near the field modulus, a commitment for a different owner). No under-constrained path was found in the shield circuit itself. The pass and the follow-up reviews did find under-constrained paths elsewhere, each fixed and covered by a regression test: the nullifier key not bound to the note in the spend circuits (`circuit-owner-binding.test.ts`), and the compliance circuit's non-membership soundness (`circuit-compliance.test.ts`).

**4.4 The Poseidon2 round-constant parameterization is confirmed, with a specific test case, to exactly match the Rust contract's native hash. Met.**
`note_commitment_matches_real_shield_circuit_v2_500stroops_vector` recomputes the commitment in Rust from the shield circuit's own test vector (value 500, the real testnet asset's field value, rho 3, rcm 4, the fixture's owner key) and requires it to equal the commitment inside a real proof of the compiled circuit. The contract's native hash is separately checked bit for bit against the pure-Rust implementation (`poseidon2_native_matches_pure_rust_*`) and against circomlibjs (`poseidon2_zero_zero_matches_circomlibjs`). Chained, the contract's hash equals the circuit's on real data, not merely a similar-looking parameter set. `token/src/lib.rs` also pins `address_to_field_bytes_and_commitment_match_real_testnet_asset` against a value computed independently with circomlibjs. Chained to a real proof: the same circuit produced the proof verified on-chain at https://stellar.expert/explorer/testnet/tx/e51f3be33e335917b663cb1969a7d9812cbac1c5e97e797c7fb2cdcb43abd2b1.

---

## Deliverable 5: WASM compilation and proof-generation library

**5.1 The shield circuit compiles to WASM and exports usable JavaScript bindings. Met.**
`circuits/shield/build/shield_js/shield.wasm` with circom's `witness_calculator.js`, wrapped by `sdk/src/prover/shield.ts` (`generateShieldProof`), built by `npm run build --workspace=sdk`. The browser check below loads it in Chromium.

**5.2 The proof-generation library creates valid Groth16 proofs for representative shield inputs, verified against the deployed Testnet contracts, not only a local harness. Met.**
The four live shields above each carry a proof from `generateShieldProof`, verified by the deployed verifier on-chain. Off-chain checks (`prover-worker.test.ts` verifies with snarkjs) are additional, not the only ones.

**5.3 A documented example demonstrates end-to-end proof generation and on-chain verification, usable by the SDK to construct a valid shield transaction. Met.**
`docs/INTEGRATION_GUIDE.md` section 7 documents the example and the runnable reference `scripts/testnet_live_validation.cjs` (four shields, `transfer4`, `unshield`, each proof verified on-chain), with the exact command and the code path `wallet.shield` follows. On-chain verification this script produces: https://stellar.expert/explorer/testnet/tx/e51f3be33e335917b663cb1969a7d9812cbac1c5e97e797c7fb2cdcb43abd2b1 (shield), https://stellar.expert/explorer/testnet/tx/15cbeef9533724df6ea96d3e96152255039664b11dac8640dd3d4c01370678ba (transfer4), https://stellar.expert/explorer/testnet/tx/c99b6b23dd068d3c12c77697cb614efa06c805349233716851efc85a22f80891 (unshield).

**5.4 Witness generation runs off the main JavaScript thread via a Web Worker, confirmed by the page remaining responsive during proof generation. Met.**
`sdk/src/prover/worker.ts` is a browser Web Worker entry point. `scripts/browser_worker_check.mjs` bundles it with esbuild, serves a page with the circuit artifacts, drives headless Chromium and measures the page's worst main-thread stall while a real shield proof is generated: about 205 ms inline versus about 19 ms with the worker (one run; `npm run check:browser-worker`). Why a browser and not Node: Node's `worker_threads` cannot run snarkjs (its `web-worker` dependency crashes on import), so the worker targets browsers, where it runs. Limit: the check is a script that needs a local Chromium, not part of CI.

**5.5 generateTransfer4Proof, generateUnshieldProof and the swap-fairness generator have each produced a proof used in a fresh, individually-submitted and verified live transaction. Met.**
Each is its own live transaction on the current stack (`docs/TESTNET_DEPLOYMENT.md`):
- `generateTransfer4Proof`: `transfer4`, tx https://stellar.expert/explorer/testnet/tx/15cbeef9533724df6ea96d3e96152255039664b11dac8640dd3d4c01370678ba (19 public signals, 86.3M instructions).
- `generateUnshieldProof`: `unshield`, tx https://stellar.expert/explorer/testnet/tx/c99b6b23dd068d3c12c77697cb614efa06c805349233716851efc85a22f80891.
- swap-fairness generator: `reveal_and_claim`, tx https://stellar.expert/explorer/testnet/tx/56cf20e1bed210acc1548e32514ccfc59b8f6dc31ffd788d5c609e9054321297, completing the chain started by the shield of the asset-in note (https://stellar.expert/explorer/testnet/tx/ff8756d3320ae98a03abf76562e3b7fb980283624ca50700e1979c939e1d527d), `commit_swap` (https://stellar.expert/explorer/testnet/tx/96ac0a773395a31b36521abe81fa2ff933e6cadf0502399a467e5ec3738b1340), and `execute_swap` (https://stellar.expert/explorer/testnet/tx/994f97fdf6b73dbcb2fd4bf8467a49be63c6d3dd2a5a7483aacfd6c314949797).
Limit: these ran on a validation deployment whose verifier is administered by the deployer rather than through governance's timelock.

---

## Deliverable 6: Shield flow test suite

**6.1 All shield flow unit and integration tests pass. Met.** 172 Rust tests and 128 JS tests, all passing (commands above).

**6.2 Coverage includes valid proofs, invalid proofs, edge cases, Merkle consistency, duplicate-commitment rejection, contract state transitions, and event emission. Met.**
An independent review pass (not the author) mapped each category to tests and found gaps: event emission and auth were untested, zero and maximum amounts were missing, many `shield_batch` and spend-path error branches were unexecuted, and several tests only asserted `is_err()`. All were closed with 49 new tests (`tests/shield_flow.rs`, `tests/spend_paths.rs`) that assert exact errors and unchanged state: valid and invalid proofs (exact `InvalidProof`, wrong public input), zero, negative, `i128::MAX`, 2^64+1 and overflowing amounts, Merkle consistency against an independent root, duplicates, pause / admin-transfer / allowlist / minimum-amount transitions, events, and authorisation without `mock_all_auths`.

**6.3 Integration tests exercise the on-chain verifier with proofs generated from the WASM circuit library, with a published coverage report. Met.**
The verifier's tests verify proofs produced by the compiled circuits and the SDK-side encoders (`verify_accepts_real_*`, plus the SDK's encoding tests against the same proof files), and the live transactions exercise the deployed verifier with SDK-generated proofs — shield, transfer4 and unshield under 1.1/5.5, e.g. https://stellar.expert/explorer/testnet/tx/15cbeef9533724df6ea96d3e96152255039664b11dac8640dd3d4c01370678ba. Coverage report: `docs/COVERAGE.md` (token crate: 99.59% of regions, 99.55% of lines, 92.94% of functions; `lib.rs` 99.89% of lines — 3 lines added since by Tranche 2 features, documented as open gaps in `docs/COVERAGE.md`).

**6.4 Fuzz testing has been run against the shield flow, with any findings triaged and either fixed or explicitly documented as accepted risk. Met.**
Three cargo-fuzz targets in `contracts/token/fuzz`: `shield_arbitrary`, `transfer_arbitrary`, `verifier_arbitrary`. Invariants: junk proofs and inputs are never accepted, and every rejected call leaves the leaf count, root, supply and nullifiers unchanged. A 30-minute campaign per target executed 44,161, 34,787 and 61,122 inputs (140,070 total) with no crash and no invariant break. The resulting corpus is minimized (`cargo +nightly fuzz cmin`, 12/37/157 inputs, 848KB) and committed under `contracts/token/fuzz/corpus/`, so CI's 60-second-per-target smoke job and any future run starts from real coverage instead of an empty seed. Triage: nothing to fix. Limit, stated as accepted: each execution builds a full contract environment, so throughput is low (19-38 executions per second); there is still no swap or governance fuzz target, since neither introduces new byte-level parsing beyond what `shield_arbitrary`/`transfer_arbitrary`/`verifier_arbitrary` already exercise (both call into `token`/`verifier` for their proof-verifying work).

**6.5 The published coverage report's adequacy has been independently reviewed, not only generated, with any gap either closed or documented. Met.**
An independent review assessed `docs/COVERAGE.md` and found it not adequate as first published (no itemised gaps, `types.rs` omitted, no assessment of assertion quality, no event, auth or extreme-value tests). The gaps were closed by the 49 tests above, and the remaining uncovered code is documented in `docs/COVERAGE.md`: derive glue in `types.rs` and one branch of a test-only helper. Line coverage is not correctness; the report says so and lists what coverage cannot show.

**6.6 The real-WASM-versus-native-Rust instruction-cost comparison runs automatically in CI for every entrypoint, not just shield, transfer, and transfer4, and fails the build on a material discrepancy. Met.**
`contracts/token/src/tests/cost_parity.rs` runs each entrypoint twice, native and on the compiled WASM, prints both figures, and fails the build if the WASM cost exceeds 400M or is more than 25% above native. `cost_parity_verify_and_verify_batch` does the same for the verifier's two verification entrypoints; `cost_parity_swap_commit_and_reveal` (`contracts/swap`), `cost_parity_publish_compliance_proof` (`contracts/compliance`) and `cost_parity_governance_queue_and_execute` (`contracts/governance`) do the same for every other proof-verifying or state-mutating entrypoint in the workspace, each against its own compiled `wasm32v1-none` artefact. CI builds the WASM and runs `cargo test --workspace`, so all of these run on every push. Measured (native / WASM):

| Entrypoint | Native | Compiled WASM | Gap | Of 400M |
| --- | --- | --- | --- | --- |
| `shield` | 67.4M | 76.4M | +13% | 19% |
| `shield_batch` (3) | 131.9M | 144.2M | +9% | 36% |
| `shield_batch` (8, max) | 293.9M | 314.4M | +7% | 79% |
| `transfer` (2 in, 2 out) | 65.7M | 74.9M | +14% | 19% |
| `transfer4` (4 in, 4 out) | 70.3M | 80.8M | +15% | 20% |
| `unshield` | 29.7M | 32.8M | +10% | 8% |
| verifier `verify` | 27.3M | 28.0M | +3% | 7% |
| verifier `verify_batch` (4) | 61.4M | 63.1M | +3% | 16% |
| swap `commit_swap` | 35.7M | 41.1M | +15% | 10% |
| swap `reveal_and_claim` | 95.9M | 106.4M | +11% | 27% |
| compliance `publish_compliance_proof` | 26.7M | 27.7M | +4% | 7% |
| governance `queue_vk_update` | 81.6K | 420.1K | — | <1% |
| governance `execute_vk_update` | 164.4K | 1.44M | — | <1% |

Governance's two entrypoints do no proof verification (they forward a VK to the verifier and flip a timelock flag), so their native cost is a few hundred thousand instructions; the WASM/native ratio is large in percentage terms purely because the fixed cost of instantiating a WASM contract dominates a near-zero baseline, not because of anything expensive. `cost_parity_governance_queue_and_execute` therefore checks an absolute allowance (native + 2M) instead of a percentage, and both stay under 1% of the 400M budget regardless. Every other row uses the same 25%-over-native, 400M-absolute rule as the token entrypoints.

---

## On-chain evidence, re-verified

`node scripts/verify_onchain_evidence.cjs` queries public Testnet RPC and checks that each cited transaction on the current stack is SUCCESS on the documented contract and function, and that contract state matches:

- 4 shield transactions: https://stellar.expert/explorer/testnet/tx/e51f3be33e335917b663cb1969a7d9812cbac1c5e97e797c7fb2cdcb43abd2b1, https://stellar.expert/explorer/testnet/tx/048f02332f51a0f5efe99c40e01f4b80b073d61755328630c4b28637d150b084, https://stellar.expert/explorer/testnet/tx/0ac97d14310a692a35a7a3c9da71bc84e03e96eed0dcf0c97120a52df2183700, https://stellar.expert/explorer/testnet/tx/244b995070978701a382202355d76c352d63e1b150092541da77d8c9a4910b1c
- `shield_batch` of 8: https://stellar.expert/explorer/testnet/tx/22e3c4e31121a045f319e965a04761edca3d527f3dfb077423aaf0e5eac5964d
- `transfer4`: https://stellar.expert/explorer/testnet/tx/15cbeef9533724df6ea96d3e96152255039664b11dac8640dd3d4c01370678ba
- `unshield`: https://stellar.expert/explorer/testnet/tx/c99b6b23dd068d3c12c77697cb614efa06c805349233716851efc85a22f80891
- a swap-asset shield: https://stellar.expert/explorer/testnet/tx/ff8756d3320ae98a03abf76562e3b7fb980283624ca50700e1979c939e1d527d
- `commit_swap`: https://stellar.expert/explorer/testnet/tx/96ac0a773395a31b36521abe81fa2ff933e6cadf0502399a467e5ec3738b1340
- `execute_swap`: https://stellar.expert/explorer/testnet/tx/994f97fdf6b73dbcb2fd4bf8467a49be63c6d3dd2a5a7483aacfd6c314949797
- `reveal_and_claim`: https://stellar.expert/explorer/testnet/tx/56cf20e1bed210acc1548e32514ccfc59b8f6dc31ffd788d5c609e9054321297
- `publish_compliance_proof`: https://stellar.expert/explorer/testnet/tx/514b9abca55beeb41d56f739f11d83ee9cb8d3a5be90f33cb0736318e3eb5385

State read back at the time of that check: `leaf_count` 18, `min_shield_amount` 1000, five verifying keys registered (Shield, Unshield, NonMembership, Transfer4x4, SwapFairness). The min-shield change (https://stellar.expert/explorer/testnet/tx/09a8a09fc0efe4437b314c38ad6d565480573bc2806416d5a83a70c6a371bbff) and restore (https://stellar.expert/explorer/testnet/tx/fb11dff8c2083c051034707466cf599d32db830f7448d5ea0313194611ca6ae2) transactions, re-run live against the actual current stack's token contract, are also SUCCESS. Public RPC retains only a recent window, so older-stack transactions may return NOT_FOUND there and must be checked on an explorer. (`leaf_count` has since grown well past that snapshot, to 1,402 at the time of writing, from the Deliverable 2.4 scale run and the additional at-scale confirmation transaction above — a live view call, not a discrepancy.)

## Instruction-cost optimisation

The `transfer4` margin (397.9M of 400M) was the roadmap's main risk, so the cost was investigated rather than trimmed at the edges. Findings, each measured on the compiled WASM (full method and numbers in `docs/PERFORMANCE_OPTIMISATION.md` and `docs/TECHNICAL_SPEC.md`, "Where the cost went"):

| Change | transfer4 | shield |
| --- | --- | --- |
| before | 397.9M | 118.6M |
| precomputed empty-subtree roots (an insert re-hashed 32 constants per call) | 210.5M | 81.1M |
| consecutive leaves inserted as one batch (each tree level hashed once) | 83.3M | 79.0M |
| Poseidon input limbs passed to the host directly | 82.0M | 77.5M |
| proof point negated on its bytes instead of a scalar multiplication | 80.8M | 76.4M |

Result: transfer4 -80%, transfer -68% (233.8M to 74.9M), shield -36%, and the maximum `shield_batch` rose from 3 items to 8. The cause was not the verifier: 4 inserts x 64 hashes was most of the cost, half of it recomputing constants.

Correctness of the changed code: the constants are checked against the Poseidon chain (`empty_roots_table_matches_poseidon_chain`); batched insertion is checked node by node against an independently computed tree for 30 batches of every size 1 to 8 crossing power-of-two boundaries, mixed with single inserts (`insert_many_matches_an_independent_tree_for_every_batch_size_and_alignment`, `single_and_batched_inserts_can_be_mixed`), and against the at-scale root; the Poseidon call is checked bit for bit against the pure-Rust implementation and the circuit's own vector; the byte negation is checked against the scalar-multiplication negation on twelve real curve points and against invalid coordinates (`byte_negation_*`, `verify_rejects_a_proof_with_an_invalid_a_point`). It also runs live: the `transfer4` and `unshield` proofs on Testnet are built from Merkle paths read back from the batch-built tree and verify on-chain.

Considered and rejected on measurement: storing each circuit's verifying key separately (registering all six keys changes `verify` by 0.1%); reading empty-subtree roots from storage. Measured but not adopted: `opt-level = 3` instead of the size-optimised `"z"` saves about 5% (transfer4 76.7M, shield 72.8M) and shrinks the token WASM from 88.5KB to 72.2KB; not adopted because at 20% of the limit it changes every artefact hash for no visible benefit, and it is the first thing to try if an entrypoint needs more headroom. Not pursued: a lower-level Poseidon call that skips the sponge's own input checks (estimated single-digit-percent gain for a change to safety-relevant validation).

One behavioural difference: a call that inserts several leaves records one root-history entry (its final root) instead of one per leaf. No transaction can be anchored to an intermediate root, so this only widens the window in calls, not in leaves.

## What is not done, and what is thin

- **`shield_batch` at its maximum** uses 314.4M of the 400M limit on the compiled WASM (335M declared live); that is the largest instruction figure of any entrypoint and the reason the cap is enforced by a test that sweeps every size. `transfer4`, which used to sit at 99.5% of the limit, is now at 20%.
- **Clawback:** proven against the Soroban test environment's asset contract, not by a live clawback on Testnet.
- **Fuzzing:** a modest run and a CI smoke, not a long campaign.
- **Browser check:** a script needing a local Chromium; not in CI.
- **Governance timelock** was exercised on the earlier Testnet stack, not on the current validation stack.
- **Development keys:** the circuit keys come from a single-contributor development ceremony; a real multi-party ceremony is required before mainnet. Nothing here is deployed to mainnet.

## Beyond the roadmap

Two review rounds (`docs/SECURITY_AUDIT_TRANCHE1.md`) found and fixed: the unbound nullifier key, non-canonical public-input aliasing, weak batch challenges, an unsound compliance circuit, and a swap front-running path; accepted risks are listed there.
