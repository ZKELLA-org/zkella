# Tranche 3 deliverables: status, evidence and limits

Criterion-by-criterion account of Tranche 3 (Compliance, Governance & Testnet Readiness), in
the same spirit as `docs/TRANCHE1_DELIVERABLES.md` and `docs/TRANCHE2_DELIVERABLES.md`: what
the roadmap's own success criteria (`docs/SCF_REVIEWER_RESPONSE.md`, "Tranche 3") ask for,
what's actually built, and the evidence for each — live Testnet transactions where the
criterion asks for one, real tests otherwise, and an honest note wherever something falls
short of the criterion as literally written.

## How to reproduce

```
cd contracts && cargo build --workspace --target wasm32v1-none --release && cargo test --workspace --release
npm test                                                    # JS unit tests (circuits, SDK, indexer)
cd sdk && npm run typecheck && npm run build
cd contracts/token/fuzz && for t in shield_arbitrary transfer_arbitrary verifier_arbitrary swap_arbitrary \
  governance_arbitrary compliance_arbitrary viewing_keys_arbitrary token_admin_arbitrary verifier_admin_arbitrary; do
    cargo fuzz run "$t" -- -max_total_time=60; done
scripts/testnet_deploy_stack.sh                              # scripted, repeatable six-contract deploy
scripts/testnet_health_check.sh                              # RPC, indexer, contract-state checks
node examples/0{2,3,4,5,6,7}-*.cjs                           # shield, indexer query, audit, transfer, unshield, swap
```

Current Testnet stack (`deployments.json`'s `testnet_final` block): verifier
`CBVZCE42NSL34LGUJNRKBJTU2RSDIKGJSK4WV5NUPCNWAUCYKK56GWC2`, governance
`CA3QHKWLHLVBXFYUGTBWA5GWOX6KWMITIQ6LIJU6LTEZQUW5777XEYST`, token
`CAE63TOLHJDZ3AREF22RA26ASF46TIT4Y5JBTREUWJJQ7EJBOUBHPUIG`, swap
`CDXUPIAJSGXITDYY3OMWSOP2CW4JEMZHHHXLRDVX6WPG5IBOJ3TEVZOV`, compliance
`CCM4NTFPH3D7IYDHVP7HPXHOPSYZMBLB3GRVEXF4YOJ7NJLAHTSEIKW3`, viewing_keys
`CD2TTLHD3EY3QRRALPPDSTPRQDR5G3IKNIIYIQGWQZVW2EKHY67YTUS2` — deployed by
`scripts/testnet_deploy_stack.sh`, not hand-typed CLI invocations. Governance is built with the
`testnet-fast-timelock` feature (`timelock_ledgers() == 60`, not the 7-day production value) so
the full timelock path could be demonstrated live within the session; see
`docs/RUNBOOK.md` §1 for what this does and doesn't mean for a production deployment.

Every transaction hash below was independently confirmed against Horizon's permanent
transaction record (`https://horizon-testnet.stellar.org/transactions/<hash>`), matched to its
specific contract call by decoding its operation parameters, not merely checked against this
repository's own prior documentation.

---

## Deliverable 1: Viewing Keys + Compliance

**1. The viewing key registry and compliance contracts compile and deploy to Testnet. Met.**
Both are part of `testnet_final` above. Compliance's `initialize` ran live at
https://stellar.expert/explorer/testnet/tx/62521977b17c60b46be3d02640d5470b9fb93cf9e1235d787b678761ae898ac8.

**2. Both contracts pass functional tests, including proof verification for the compliance
non-membership check. Met.** `contracts/compliance/src/lib.rs`'s `accepts_and_stores_a_real_sdk_proof`
verifies a genuine `circom`/`snarkjs`-generated non-membership proof, not a mocked one.
`contracts/viewing_keys/src/lib.rs` tests `register`/`revoke`, including that both require the
owner's own authorization. See "Test totals" below for current counts.

**3. Completed technical documentation for both contracts. Met.** `docs/VIEWING_KEYS.md` covers
disclosure, epochs, revocation-by-rotation, and sanctions-list maintenance; `docs/ARCHITECTURE.md`'s
"Viewing keys and compliance" entry covers the contracts' design and current limits.

**4. `publish_compliance_proof` is exercised in a real, published live Testnet transaction, not
only in source review and local tests. Met.** Tx
`e489e10014615fbaaa6f078934479c0fc089ab4175df85a46d6bc6f75674d52d`
(https://horizon-testnet.stellar.org/transactions/e489e10014615fbaaa6f078934479c0fc089ab4175df85a46d6bc6f75674d52d),
submitted through the SDK's `ZKELLACompliance.publishProof()`, not raw CLI.

**5. An account holder can revoke a previously-granted viewing key, confirmed by a test showing
the designated party can no longer decrypt note history after revocation. Met, under a
reworded criterion, which is itself a recorded decision, not a silent narrowing.**
`docs/VIEWING_KEYS.md`'s "Revocation criterion (decision)" section explains why the criterion
as literally written cannot hold for any history-reveal scheme: a note's ciphertext is
published on-chain at the time it's created, and revoking a key cannot reach back and
re-encrypt it. What is built and tested (`tests/unit/viewing-key-rotation.test.ts`):
`wallet.rotateViewingKey()` starts a new epoch, the viewing-key registry's `revoke` withdraws
the advertised commitment for the old one, and a holder of only the earlier epoch's key cannot
decrypt notes received after the rotation — while notes received before it remain readable to
whoever already held that key, which the document states plainly rather than omits.

**6. The decrypt-on-request workflow is implemented end-to-end. Met.** `ZKELLAAuditor.sync()`/
`transactionHistory()` ran live against `testnet_final` with a real exported viewing key and
recovered five genuine receipts (value and ledger for each) from the stack's shield, transfer
and unshield activity, reading nothing it shouldn't: a zero-value padding note from a transfer
is excluded (`tests/unit/auditor.test.ts`), and spends are never visible to a viewing key by
construction (no nullifier key). An unrelated key recovers nothing
(`tests/unit/auditor.test.ts`, "a viewing key for a different wallet recovers nothing").

**7. The sanctions-list maintenance model is specified and documented. Met.**
`docs/VIEWING_KEYS.md`, "Sanctions list maintenance": the maintainer on Testnet is the
compliance admin key (a multisig is called out as needed before mainnet), update cadence is
weekly plus an urgent same-day path, every published root is accompanied by the source list's
hash, and the current root is the empty-list root (sentinels only) — Testnet has no real
sanctions list.

---

## Deliverable 2: Governance

**1. The governance contract compiles and deploys to Testnet. Met.** Address above;
initialized with `timelock_ledgers() == 60` (fast-timelock build) and a distinct guardian key.

**2. It passes functional tests, including the timelocked key-rotation path (both first-time
registration and rotation), the guardian's cancel-only authority over a queued update, and the
pause mechanism newly added across all four contracts that lacked one. Met.** See "Test
totals" below. Representative tests in `contracts/governance/src/lib.rs`:
`guardian_can_cancel_a_queued_update_even_while_paused`,
`guardian_can_cancel_a_queued_min_shield_update`,
`pause_unpause_and_guardian_cancel_require_authorization`; pause coverage spans
`contracts/verifier`, `contracts/governance`, `contracts/compliance` and `contracts/swap`, each
with a test that pause blocks its gated entrypoints and unpause restores them, and a separate
test per contract for each entrypoint that must stay callable while paused (e.g.
`pause_does_not_block_revoke_previous_vk`, `pause_does_not_block_cancel_swap`).

**3. Completed technical documentation, including an explicit note on which governance
features are deferred to a later upgrade. Met.** `docs/GOVERNANCE.md`, "Deferred to a later
governance upgrade": admin multisig, circuit-upgrade authorization beyond VK rotation, and
fine-grained parameter adjustment beyond what this deliverable adds.

**4. `MIN_SHIELD_AMOUNT` and similar constants are governance-settable parameters rather than
hardcoded values, confirmed by changing one through governance without a contract redeploy.
Met, by test and by a live Testnet transaction.** `TokenAdminAction::MinShieldAmount`,
`AssetApproval` and `Relayer` all route through `queue_token_action`/`execute_token_action`,
timelocked the same way as a VK update (`contracts/governance/src/lib.rs`'s
`min_shield_amount_changes_on_the_real_token_only_after_the_timelock` and related tests, run
against a real `token` contract, not a mock). On `testnet_final`: queued at tx
`e71e95957f3817db2a7f1c1754258af6428956e46c38bdad843e20c64b761840`, executed after the timelock
at tx `ce8d642fc8b4db9bb86fe53dcb3b2e2771b26e58a413a73377506a8a4bcefa07`, with `token.min_shield_amount()`
read back as `500` afterward (it was `1000` before) — no contract redeploy involved.

---

## Deliverable 3: Security Tooling & Remediation Pass

**1. Static analysis, dependency vulnerability scanning, and fuzz testing have been run against
every contract and circuit across the entire project. Met for the contracts; circuits have no
direct equivalent of static analysis or dependency scanning (`circom` has no analogue of
`cargo clippy`/`cargo audit`), so they are covered by fuzzing the contract entrypoints that
consume their proof bytes.** `docs/SECURITY_TOOLING_REPORT.md`: `cargo clippy --workspace
--all-targets --release`, `cargo audit` (one fixed, one accepted-risk finding), `npm audit
--omit=dev` (5 high and 2 moderate findings fixed; 15 low accepted-risk, documented with
rationale). Nine `cargo-fuzz` targets, one per contract's state-changing surface
(`shield_arbitrary`, `transfer_arbitrary`, `verifier_arbitrary`, `swap_arbitrary`,
`governance_arbitrary`, `compliance_arbitrary`, `viewing_keys_arbitrary`,
`token_admin_arbitrary`, `verifier_admin_arbitrary`), up from the three this tranche started
with. `compliance_arbitrary` has a committed regression seed
(`contracts/token/fuzz/corpus/compliance_arbitrary/regression_root_equals_authorized`) from a
real crash the harness found during this pass (a fuzzer input whose random root happened to
equal the authorized one) and the fix that followed. CI runs all nine for 60 seconds each on
every push.

**2. Findings are triaged and either remediated or explicitly documented as accepted risk, with
a full findings-and-remediation report published alongside the code changes. Met.**
`docs/SECURITY_TOOLING_REPORT.md` is that report; every accepted-risk line states why it's
accepted, not just that it is.

**3. The real-WASM instruction-budget measurement already applied to shield, transfer, and
transfer4 is extended to every remaining entrypoint across all six contracts, with results
published. Met.** `contracts/budget/tests/instruction_budget.rs` measures every non-proof
state-changing entrypoint (38 of them) against the real compiled `wasm32v1-none` binaries, not
a native-Rust estimate. Results are in `docs/SECURITY_TOOLING_REPORT.md`'s budget table; the
highest is `merkle_root` at 0.55% of the mainnet limit, well clear of the margin concerns the
grant-proposal stage raised for the proof-verifying entrypoints.

---

## Deliverable 4: Full Testnet Deployment & Operational Runbook Drill

**1. Viewing keys deploys to Testnet for the first time, and compliance and governance redeploy
carrying the pause mechanism and guardian role, all three correctly wired to the existing
token, verifier, and swap contracts, and the complete set of all six contract addresses is
published. Met.** `deployments.json`'s `testnet_final` block, deployed and wired by
`scripts/testnet_deploy_stack.sh` in the order verifier → governance → token → swap →
compliance → viewing_keys, with governance set as the verifier's and token's admin.

**2. The indexer is confirmed operating correctly against the token and swap addresses actually
in use after this deployment. Met.** The indexer was pointed at `testnet_final`'s token from
its deploy ledger (5034000) and reached tip with zero lag, confirmed live via
`scripts/testnet_health_check.sh` and by a live shield, transfer, and unshield each being
indexed and later read back correctly (notes, Merkle paths, and viewing-key receipts all
matched what was actually submitted).

**3. An operational runbook exists, covers all four incident categories and the full minimum
operating checklist, and is written against this real Testnet deployment. Met.**
`docs/RUNBOOK.md`, rewritten this pass to describe the pause mechanism and guardian role on the
actual `testnet_final` addresses rather than the superseded stacks it previously described.

**4. A scheduled script automates the runbook's RPC-health, indexer-health, and contract-state
checks and posts a notification on failure. Built, not yet actually scheduled.**
`scripts/testnet_health_check.sh` checks all three and posts to `NOTIFY_WEBHOOK` on failure; a
manual run against `testnet_final` today passed every check. No cron job or equivalent
scheduler is actually installed anywhere, and no webhook destination is configured in this
repository — both are an operational decision for whoever runs the deployed stack day to day,
not something this repository can decide on the team's behalf. See `docs/RUNBOOK.md`'s
"Decisions" section.

**5. At least one incident-response drill has been run against the Testnet indexer and
contracts, including a case surfaced by that automated check, with findings incorporated back
into the runbook. Met, as a tabletop drill; not yet as a drill with people.**
`docs/RUNBOOK.md`'s "Drill record": three faults (RPC down, indexer down, a misconfigured
governance address) were injected into `scripts/testnet_health_check.sh`, one at a time,
against the real `testnet_final` stack on 2026-10-05. All three were caught; the drill found
that the governance failure didn't name the address it tried, and the script was fixed to
include it. A scheduled drill with people walking all four incident categories has not
happened.

**6. This deployment is executed through a scripted, repeatable deployment pipeline rather than
manual, hand-typed CLI invocations. Met.** `scripts/testnet_deploy_stack.sh` deploys,
initializes, queues and executes all six verifying keys, approves the native asset, and sets
the sanctions root, with a `RESUME` mode for continuing after a transient failure (used for
real: a DNS failure interrupted the first deploy attempt).

---

## Deliverable 5: TypeScript SDK, Modules, Stabilization & Release (Testnet)

**1. SDK modules for note/commitment building, proof generation, transaction assembly, and the
indexer client are implemented. Met.** `sdk/src/keys`, `sdk/src/notes`, `sdk/src/prover`,
`sdk/src/wallet`, `sdk/src/compliance`, `sdk/src/indexer`.

**2. The SDK is published on npm with TypeScript typings and a stabilized public API, targeting
Stellar Testnet. Met, as `0.1.0`, deliberately not tagged `1.0.0`.** Published and live:
`npm view @zkella/sdk version` returns `0.1.0`. `docs/SDK_RELEASE.md` records the decision —
semver's own convention reserves `1.0.0` for an API-stability commitment the team isn't yet
making, and a `0.1.0` first release says that honestly rather than overclaiming a `1.0` the
roadmap's original wording assumed. The API itself (typings, documented surface, Testnet
targeting) is exactly what the criterion asks for otherwise.

**3. The SDK's Testnet network configuration includes all six deployed contract addresses and
the correct indexer endpoint. Met.** `sdk/src/config/testnet.ts`'s `TESTNET_CONTRACTS`;
`tests/unit/testnet-config.test.ts` fails the build if it drifts from `deployments.json`.

**4. Example code for shield, transfer, unshield, viewing-key, indexer, and shielded-swap flows
runs successfully against the Testnet deployment. Met — all six ran live this pass.**
- Shield: tx `1126edc56b34bc38836eb9d14a12450fa80c763332604238f8bdc7fc8e62cbc6` (leaf 0, ledger 5038957).
- Indexer query (`examples/03-indexer-query.cjs`): returned 5 notes and a 32-level Merkle path.
- Viewing-key audit (`examples/04-viewing-key-audit.cjs`): recovered 5 real receipts from a granted export.
- Transfer (`examples/05-transfer.cjs`): tx `0b53568b5ade7885f915a23a65bdf053e5acc45b4bb9d3599ddc7e1a6fdaf444`, new leaves 12 and 13.
- Unshield (`examples/06-unshield.cjs`): tx `f81c0016e472d8d8a9d6a15caa2778983ae4ee941fa7f0ae9c1095910fdee795`, change note at leaf 14.
- Swap (`examples/07-swap.cjs`): commit tx `8502c742c674ac87677d90a367ca870f9ac11b372c0bb4d2209f3e521858bb0e`,
  cancelled after expiry at tx `d3d7565947e679959ff9731cfe62bf21dfa70d5abc0193ffd9ab4d63d2b64887`, refunding the escrow.
  A separate, earlier full commit → relayer execute → reveal-and-claim lifecycle also ran live on this
  same stack (swap id `089678d3aba9f4b0837cd8504973d51e828f480db51ab4262e705de11a2810f3`, claimed into leaf 4; see
  `deployments.json`'s `testnet_final._live_checks`).

Two real bugs surfaced and were fixed by these live runs, not found by unit tests alone: the
transfer example didn't pass the recipient's owner key (`toOwnerKey`) and pointed at circuit
artifact paths `circuits/build.sh` doesn't produce; the swap example quoted 99% of the
requested amount instead of the input note's full value, which the fairness check correctly
rejected, since the SDK sets `min_amount_out` from the note's value, not the requested amount.

**5. Developer documentation is available and covers installation, Testnet configuration, API
reference, indexer usage, and troubleshooting. Met.** `docs/SDK_DEVELOPER.md`.

**6. The operational runbook includes an SDK-artifacts section covering version pinning,
upgrade guidance, and client-side integration troubleshooting. Met.** `docs/RUNBOOK.md`, "SDK
artifacts".

**7. The wallet's submission path retries a transient RPC failure and resubmits against a fresh
anchor if the original ages out of the root-history window, confirmed by a test simulating
both failure modes. Met.** `tests/unit/wallet-resilience.test.ts`: a transient failure that
never reached the network is retried and succeeds; an ambiguous failure whose transaction did
land is not resubmitted (checked by hash before retrying — the one way a naive retry could
double-spend); a contract rejection is not retried; an evicted anchor rebuilds the call against
a fresh root; other failures are rethrown without triggering a rebuild.

**8. The swap, auditor, and compliance wrapper classes call the real deployed contracts and
provers rather than returning stub placeholder values, confirmed by each producing a real,
verified live transaction. Met.** `ZKELLASwap` (commit/execute/reveal and commit/cancel, tx
hashes above and in `deployments.json`), `ZKELLAAuditor` (live receipt recovery above),
`ZKELLACompliance` (publish tx above).

---

## Test totals at the time of writing

Full contract workspace (`cd contracts && cargo test --workspace --release`): 229 tests,
all passing — token 124, swap 29, verifier 37, governance 21, compliance 13, viewing_keys 5
(`token-interface` and `verifier-interface` have no tests of their own; they're trait
definitions). JS unit tests (`npm test`, covering circuits, SDK, and indexer): 172 passing, 9
skipped (the skipped tests need a real PostgreSQL instance, not SQLite). SDK typecheck
(`cd sdk && npm run typecheck`) passes with no errors.

## What is left open, honestly

- **Admin multisig** on every contract remains a single key, by design deferred to a mainnet
  deployment decision (`docs/GOVERNANCE.md`).
- **No scheduled health-check cron job or alert webhook is actually running** anywhere; the
  tooling exists, the decision to run it continuously belongs to whoever operates the deployed
  stack.
- **No drill with people**, only a tabletop fault-injection exercise.
- **Fuzzing is smoke-level** (60-to-90-second runs), not a sustained campaign; five of the nine
  targets have no committed regression corpus yet.
- **No independent third-party audit** has been done; `docs/SECURITY_TOOLING_REPORT.md` says
  so explicitly, and that remains the honest status.
- **The SDK is `0.1.0`**, not `1.0.0` — a deliberate first release, not the literal version the
  original roadmap wording named; see `docs/SDK_RELEASE.md`.
- **`swap`'s `reclaim_expired_swap` recovery path** (the post-*execution* unwind, for a relayer
  that fronted liquidity but the claimant never claims) is still unit-tested only. Its claim
  window is `CLAIM_WINDOW_LEDGERS = 17_280` ledgers (~24 hours at 5s/ledger) after the swap's
  original expiry, which makes a live exercise a multi-hour undertaking, not something a single
  session can close inline — unlike `cancel_swap`'s pre-execution path, which was exercised
  live twice this pass.
