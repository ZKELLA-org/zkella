# Tranche 1 deliverables: status and evidence

Status as of the latest commit on `main`. "Done" means the work exists, is tested, and the evidence below can be re-run. Where a criterion is only partly met, that is stated.

Re-run everything: `cd contracts && cargo test --workspace --release`, `npm test`, `npm run check:browser-worker` (needs a local Chromium), `cargo +nightly fuzz run <target>` from `contracts/token`.

## Summary

| # | Deliverable | Status |
| --- | --- | --- |
| 1 | Asset custody and commitment logic | Done |
| 2 | Merkle tree and events | Done, one criterion partly met (at-scale cost) |
| 3 | Shared verifier registry contract | Done |
| 4 | Shield circuit | Done |
| 5 | WASM compilation and proof-generation library | Done, on a validation deployment |
| 6 | Shield flow test suite | Done, with limits on fuzzing and cost coverage |

## 1. Asset custody and commitment logic

- **Custody:** `shield` moves real SEP-41 tokens into the contract as the last step, after all state changes. `unshield` releases them the same way.
- **Asset allowlist:** only governance-approved assets can be shielded (`set_asset_approved`, `is_asset_approved`, error `AssetNotApproved`). Revoking an asset never traps existing notes, because unshield does not check approval.
- **Minimum amount:** governance-settable without a redeploy (`set_min_shield_amount`).
- **Batch deposits:** `shield_batch` deposits several notes with one aggregated transfer. It is capped at 3 items (`MAX_SHIELD_BATCH`) because 3 items already use 347M of the 400M instruction limit.
- **Commitment:** `cm = H(H(H(value, asset), H(rho, rcm)), pk)`, with `pk = H(nk, "zkella_pk")`. Every note commits to its owner's key, so it can only be spent with that owner's nullifier key.
- **Evidence:** `contracts/token/src/lib.rs` (tests `shield_*`, `min_shield_amount_*`, `shield_batch_*`), `docs/TECHNICAL_SPEC.md` section 3.

## 2. Merkle tree and events

- Depth-32 incremental Poseidon tree with a 32-entry root history, so a proof against a slightly old root still verifies.
- A full tree is rejected with `MerkleTreeFull` rather than by panic.
- Note, nullifier and shield events; `merkle_path` view.
- **Cost at scale:** insert cost is measured as the tree grows, up to 150 leaves (`merkle_insert_cost_as_tree_depth_grows`). The network's per-transaction limit of 400 ledger entries makes a single-transaction test at thousands of leaves impossible. This is measured at 150, not thousands.
- **Evidence:** `contracts/token/src/merkle.rs`, tests `merkle_*`, `transfer_*anchor*`.

## 3. Shared verifier registry contract

- One verifying key per circuit; keys are rotated through governance's timelock.
- **Key retention:** a replaced key stays valid for 17,280 ledgers (about a day), so proofs already generated still verify. `revoke_previous_vk` ends that at once; governance exposes it as `revoke_previous_vk`.
- **Batch verification:** `verify_batch` checks K proofs with K+3 pairings. Its challenges hash the circuit and every item's public inputs and proofs.
- **Canonical inputs:** public inputs at or above the field modulus are rejected (`NonCanonicalInput`).
- **Evidence:** `contracts/verifier/src/lib.rs` tests (23), five real-circuit proofs verified, `docs/SECURITY_AUDIT_TRANCHE1.md`.
- **Limit:** `verify_batch` has no measured instruction cost, and no live retention or revoke transaction was run.

## 4. Shield circuit

- **Negative testing:** `tests/unit/shield-circuit-negative.test.ts` runs malformed witnesses against the compiled circuit: forged commitment, forged value commitment, forged public amount or asset, a value of 2^64, a value near the field modulus, and a commitment for a different owner. All are rejected; no under-constrained path was found. `tests/unit/circuit-owner-binding.test.ts` covers the owner-key binding on unshield.
- **Poseidon parameters:** the circuit's Poseidon and the contract's native hash are checked to be identical on the real circuit's own test vector (`poseidon.rs`, `note_commitment_matches_real_shield_circuit_v2_500stroops_vector`) and against circomlibjs (`commitment.test.ts`).
- **Audit finding fixed:** the nullifier key was not bound to the note, so a note could be spent repeatedly. This is now enforced in the spend circuits.
- **Limit:** the keys come from a development ceremony with one local contributor.

## 5. WASM compilation and proof-generation library

- **Provers:** shield, transfer (2 in / 2 out), transfer4 (4 in / 4 out), unshield and swap-fairness generators in `sdk/src/prover/`.
- **Web Worker:** `sdk/src/prover/worker.ts` is a browser Web Worker entry point. In headless Chromium (`scripts/browser_worker_check.mjs`) the page's main thread stalled about 205 ms when proving inline and about 19 ms with the worker, from one run with a real shield proof. Node's `worker_threads` cannot run snarkjs (its `web-worker` dependency crashes on import), which is why the worker targets browsers.
- **Live Testnet transactions:** each proof type was generated by the SDK and verified on-chain in its own transaction (links in `docs/TESTNET_DEPLOYMENT.md`):
  - Four shields: `2a9d480b...`, `2c8594eb...`, `4de68504...`, `4954a5cf...`.
  - `transfer4`: `a7858b39...`.
  - `unshield`: `668fa5bf...`.
  - Swap lifecycle with a real swap-fairness proof: `commit_swap` `a90cb778...`, `execute_swap` `578cf698...`, `reveal_and_claim` `ddc95005...`.
  - Compliance non-membership proof: `87f4a346...`.
- **Limits:** these ran on a validation deployment whose verifier is administered by the deployer, not through governance's timelock. The browser check is not part of CI.

## 6. Shield flow test suite

- **Tests:** contracts have 23 verifier, 54 token, 12 swap, 4 governance, 3 compliance and 2 viewing-key tests; the JS unit suite has 128 tests.
- **Cost against the real network limit:** real-WASM instruction-cost tests for shield (113.2M), transfer (about 228M), transfer4 (396,688,826, 99.17%, thin margin), unshield (33.9M) and shield_batch of 3 (347.2M). They run in CI with the workspace tests.
- **Fuzzing:** three cargo-fuzz targets (`shield_arbitrary`, `transfer_arbitrary`, `verifier_arbitrary`) check that junk input is never accepted and that rejected calls leave state unchanged. A local run of about 85,000 executions found nothing; CI runs a 60-second smoke per target. There is no committed corpus and no swap or governance target. This is a smoke test, not a fuzzing campaign.
- **Coverage:** `docs/COVERAGE.md`. Token crate 99.1% of regions at the time of measurement. Untested pause, admin-transfer and `merkle_path` code was found and covered.
- **Limits:** the cost comparison exists for the token contract only, not for verifier, swap, governance or compliance entrypoints. The coverage review was done by the same team, not independently.

## Beyond the roadmap

Two rounds of security review (`docs/SECURITY_AUDIT_TRANCHE1.md`) found and fixed: the unbound nullifier key, non-canonical public-input aliasing, batch-challenge weakness, an unsound compliance circuit, and a swap front-running path. Accepted risks (front-runnable `initialize`, Merkle sibling TTL, sender-chosen `rho`, development keys) are listed there.

## Not done

- Mainnet: nothing here is deployed to mainnet, and a real trusted-setup ceremony is required first.
- Governance-timelock path on the current validation stack (exercised only on the earlier legacy stack).
- Fuzzing beyond a smoke run; cost tests for non-token contracts; an independent coverage review.
