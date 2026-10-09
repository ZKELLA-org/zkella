# Tranche 3 deliverables: original description, success criteria, evidence and proofs

Criterion-by-criterion account of Tranche 3 (Compliance, Governance & Testnet Readiness), in
the same spirit as `docs/TRANCHE1_DELIVERABLES.md` and `docs/TRANCHE2_DELIVERABLES.md`, and
going one step further: each deliverable below starts with its **original description and
success criteria, quoted verbatim** from the funding roadmap, followed by the evidence for
each criterion with full, clickable links — a GitHub commit or file link for every piece of
code evidence, and a full `stellar.expert` link for every on-chain transaction. Nothing here
is a paraphrase of what was asked for; it is the literal ask, next to the literal proof.

Repository: https://github.com/ZKELLA-org/zkella. Code links below point at the branch this
work lives on, `compliance-governance-security-testnet-release`
(https://github.com/ZKELLA-org/zkella/tree/compliance-governance-security-testnet-release),
unless a specific commit is named.

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

Current Testnet stack (`deployments.json`'s `testnet_final` block
— https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/deployments.json,
redeployed 2026-10-08 to carry the audit fixes in this document's last section): verifier
`CC2LQPXH3L5YKRP7YJ6UIC57AOGJXBQN4DEKNRU4Y32ABXJZOENCDAX3`, governance
`CDTJLTBEKBXRJJKHVI32A5UMBB4UC6VBMDOF7WR43H2SKCCRAVRJWY5Q`, token
`CA5TFEVODC25SSEZII2XHB2XMCKFNXNLXRNFTKWPKMT5PCWYZUMLPRUZ`, swap
`CBN7JJEPAEA5NCKOECPPGHETAK4CCCUFOBUJCDZ7K7HPIV7Y6ILOC524`, compliance
`CDP5SRSUFDVEYHUCUX53SM4PZVTOIHDZR3Z5C7G4TFFKAQSLX64FOZVJ`, viewing_keys
`CDT776JLXU5GWRIY6WXLZGVKZ5V4TG32HAITNPFZVX5UCJSMMFHNMEEE` (reused unchanged) — deployed by
`scripts/testnet_deploy_stack.sh`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/scripts/testnet_deploy_stack.sh),
not hand-typed CLI invocations. Governance is built with the `testnet-fast-timelock` feature
(`timelock_ledgers() == 60`, not the 7-day production value) so the full timelock path could be
demonstrated live within the funding window.

Every transaction link below was independently confirmed against Horizon/`stellar.expert`'s
permanent transaction record, matched to its specific contract call by decoding its operation
parameters, not merely checked against this repository's own prior documentation.

---

## Deliverable 1: Viewing Keys + Compliance

**Original description:** *"Implement a viewing key registry contract that enables optional,
user-controlled disclosure of transaction details to auditors and regulators via zero-knowledge
proofs, together with a sanctions-list non-membership compliance contract that verifies a
compliance proof against the shared verifying-key registry before storing it."*

**Original success criteria:**
1. The viewing key registry and compliance contracts compile and deploy to Testnet.
2. Both contracts pass functional tests, including proof verification for the compliance non-membership check.
3. Completed technical documentation for both contracts.
4. `publish_compliance_proof` is exercised in a real, published live Testnet transaction, not only in source review and local tests.
5. An account holder can revoke a previously-granted viewing key, confirmed by a test showing the designated party can no longer decrypt note history after revocation.
6. The decrypt-on-request workflow is implemented end-to-end: a designated party uses a granted viewing key to actually decrypt and read note history.
7. The sanctions-list maintenance model, who publishes the list root and how often it updates, is specified and documented.

### Evidence and proof

**1. Compile and deploy. Met.** Both contracts are part of the current stack above.
Compliance's `initialize` ran live at
https://stellar.expert/explorer/testnet/tx/62521977b17c60b46be3d02640d5470b9fb93cf9e1235d787b678761ae898ac8.
Source: `contracts/compliance/src/lib.rs`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/compliance/src/lib.rs)
and `contracts/viewing_keys/src/lib.rs`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/viewing_keys/src/lib.rs).

**2. Functional tests. Met.** 13 compliance tests and 5 viewing-key tests pass, including
`accepts_and_stores_a_real_sdk_proof`, which verifies a genuine `circom`/`snarkjs`-generated
non-membership proof, not a mocked one
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/compliance/src/lib.rs).
Viewing-key ownership, double-register (rotation) and revoke-without-register behavior are
covered in the same file's test module
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/viewing_keys/src/lib.rs).

**3. Technical documentation. Met.** `docs/VIEWING_KEYS.md`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/VIEWING_KEYS.md)
covers disclosure, epochs, revocation-by-rotation and sanctions-list maintenance;
`docs/ARCHITECTURE.md`'s "Viewing keys and compliance" entry
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/ARCHITECTURE.md)
covers the contracts' design and current limits.

**4. `publish_compliance_proof` live on Testnet. Met, twice.** Submitted through the SDK's
`ZKELLACompliance.publishProof()`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/sdk/src/compliance/compliance.ts),
not raw CLI:
- Original stack: https://stellar.expert/explorer/testnet/tx/e489e10014615fbaaa6f078934479c0fc089ab4175df85a46d6bc6f75674d52d
- Current, post-audit stack: https://stellar.expert/explorer/testnet/tx/533837bf63d88ce09578940d4bec9de94d60e54b4babc99b1cd09c5d419d0442

**5. Revocation. Met, under a reworded criterion that is itself a recorded decision, not a
silent narrowing.** `docs/VIEWING_KEYS.md`'s "Revocation criterion (decision)" section
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/VIEWING_KEYS.md)
explains why the criterion as literally written cannot hold for any history-reveal scheme: a
note's ciphertext is published on-chain at the time it's created, and revoking a key cannot
reach back and re-encrypt it. What is built and tested:
`tests/unit/viewing-key-rotation.test.ts`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/tests/unit/viewing-key-rotation.test.ts) —
`wallet.rotateViewingKey()` starts a new epoch, the viewing-key registry's `revoke` withdraws
the advertised commitment for the old one, and a holder of only the earlier epoch's key cannot
decrypt notes received after the rotation, while notes received before it remain readable to
whoever already held that key — stated plainly in the document, not omitted.

**6. Decrypt-on-request workflow. Met, live.** `ZKELLAAuditor.sync()`/`transactionHistory()`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/sdk/src/wallet/auditor.ts)
ran live against the current stack with a real exported viewing key and recovered 4 genuine
receipts, correctly excluding zero-value padding notes from transfers (regression test:
https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/tests/unit/auditor.test.ts).
An unrelated key recovers nothing (same test file, "a viewing key for a different wallet
recovers nothing"). Live run: `examples/04-viewing-key-audit.cjs`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/examples/04-viewing-key-audit.cjs)
against the current stack recovered receipts at ledgers 5090104, 5090106, 5090109, 5090112.

**7. Sanctions-list maintenance. Met, documented.** `docs/VIEWING_KEYS.md`, "Sanctions list
maintenance"
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/VIEWING_KEYS.md):
maintainer on Testnet is the compliance admin key (a multisig is called out as needed before
mainnet), cadence is weekly plus an urgent same-day path, every published root is accompanied
by the source list's hash, and the current root is the empty-list root (sentinels only) —
Testnet has no real sanctions list.

---

## Deliverable 2: Governance

**Original description:** *"Implement a governance contract for protocol evolution: timelocked
verifying-key rotation and an emergency pause mechanism, with event logging for audit trails.
Circuit upgrade authorization beyond key rotation, fine-grained parameter adjustments, and
admin/multisig controls are deferred to a later governance upgrade."*

**Original success criteria:**
1. The governance contract compiles and deploys to Testnet.
2. It passes functional tests, including the timelocked key-rotation path (both first-time registration and rotation), the guardian's cancel-only authority over a queued update, and the pause mechanism newly added across all four contracts that lacked one.
3. Completed technical documentation, including an explicit note on which governance features, admin multisig and beyond, are deferred to a later upgrade.
4. `MIN_SHIELD_AMOUNT` and similar constants are governance-settable parameters rather than hardcoded values, confirmed by changing one through governance without a contract redeploy.

### Evidence and proof

**1. Compile and deploy. Met.** Address above; initialized with `timelock_ledgers() == 60`
(fast-timelock build) and a distinct guardian key. Source:
`contracts/governance/src/lib.rs`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/governance/src/lib.rs).

**2. Functional tests. Met — 25 tests.** Timelocked VK rotation (first-time and rotation),
guardian cancel-only authority, and pause across all four contracts that lacked it
(`contracts/governance/src/lib.rs`, same link as above). The pause mechanism itself also had a
real reachability bug found and fixed this pass — see the audit section below — with its own
regression tests: `pause_verifier_actually_pauses_and_unpause_verifier_restores_it` and
`pause_token_actually_pauses_and_unpause_token_restores_it`.

**3. Technical documentation. Met.** `docs/GOVERNANCE.md`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/GOVERNANCE.md),
"Deferred to a later governance upgrade": admin multisig, circuit-upgrade authorization beyond
VK rotation, and fine-grained parameter adjustment beyond what this deliverable adds.

**4. `MIN_SHIELD_AMOUNT` governance-settable. Met, by test and live, twice.**
`TokenAdminAction::MinShieldAmount`/`AssetApproval`/`Relayer` all route through
`queue_token_action`/`execute_token_action`, timelocked like a VK update
(`min_shield_amount_changes_on_the_real_token_only_after_the_timelock` in
`contracts/governance/src/lib.rs`, same link as criterion 1, run against a real `token`
contract, not a mock). Live, twice:
- Original stack: queue https://stellar.expert/explorer/testnet/tx/e71e95957f3817db2a7f1c1754258af6428956e46c38bdad843e20c64b761840,
  execute https://stellar.expert/explorer/testnet/tx/ce8d642fc8b4db9bb86fe53dcb3b2e2771b26e58a413a73377506a8a4bcefa07.
- Current, post-audit stack: queue https://stellar.expert/explorer/testnet/tx/3f2159168107f2b02b203c3aaf1d9d602b5890e9aed99a381046b86c3a9889d3,
  execute https://stellar.expert/explorer/testnet/tx/f3ef72ee1e4ad65fafb5f5eb4c99d8094f61b696e7517081964cc172f84297b5.

Both runs read `token.min_shield_amount()` back as `500` afterward (it was `1000` before), no
contract redeploy involved.

---

## Deliverable 3: Security Tooling & Remediation Pass

**Original description:** *"Run automated security-analysis tooling against the complete
contract and circuit set built across all three tranches, token, verifier, swap, viewing keys,
compliance, and governance, and every circuit (shield, transfer, unshield, swap fairness,
compliance non-membership): static analysis, dependency vulnerability scanning, and fuzz
testing of every contract entrypoint, and remediate any findings. [...] This is tooling-based
self-review, not a substitute for an independent third-party audit, and this grant does not
fund an independent third-party audit; it funds the team's own tooling-based hardening pass,
which produces the evidence a later, separately-funded audit would build on."*

**Original success criteria:**
1. Static analysis, dependency vulnerability scanning, and fuzz testing have been run against every contract and circuit across the entire project, Tranches 1, 2, and 3.
2. Findings are triaged and either remediated or explicitly documented as accepted risk, with a full findings-and-remediation report published alongside the code changes.
3. The real-WASM instruction-budget measurement already applied to shield, transfer, and transfer4 is extended to every remaining entrypoint across all six contracts, with results published.

### Evidence and proof

**1. Tooling run. Met for the contracts; circuits have no direct equivalent of static analysis
or dependency scanning (`circom` has no analogue of `cargo clippy`/`cargo audit`), so they are
covered by fuzzing the contract entrypoints that consume their proof bytes.**
`docs/SECURITY_TOOLING_REPORT.md`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/SECURITY_TOOLING_REPORT.md):
`cargo clippy --workspace --all-targets --release`, `cargo audit`
(https://github.com/ZKELLA-org/zkella/commit/813346819d8c7737d054c15072d5d7ede8f08fac — spin update;
see full report for details), `npm audit --omit=dev`
(https://github.com/ZKELLA-org/zkella/commit/d38087ee73432c35eb6058f640862449036905c4).
Nine `cargo-fuzz` targets covering every contract's state-changing surface:
`shield_arbitrary`, `transfer_arbitrary`, `verifier_arbitrary` (pre-existing), plus
`swap_arbitrary` (https://github.com/ZKELLA-org/zkella/commit/2063efb8c6a4d55a6c45434601b004aaebd98a3a),
`governance_arbitrary`/`compliance_arbitrary`/`viewing_keys_arbitrary`
(https://github.com/ZKELLA-org/zkella/commit/846b0ec9b701ff9862170c0dbb3d776ef1927a73), and
`token_admin_arbitrary`/`verifier_admin_arbitrary`
(https://github.com/ZKELLA-org/zkella/commit/e06e85e8fa2278ee40344a86034fa2fa4b5cf7fd). All
nine now have a committed, minimized corpus (`cargo +nightly fuzz cmin`), including a
regression seed in `compliance_arbitrary` from a real crash the harness found
(https://github.com/ZKELLA-org/zkella/commit/04b52496fbcb33952b76d827f820bcf1532704ac) and the
fix that followed. CI runs all nine for 60 seconds each on every push.

**2. Findings report. Met.** `docs/SECURITY_TOOLING_REPORT.md` (link above) is the
findings-and-remediation report; every accepted-risk line states why it's accepted, not just
that it is.

**3. Real-WASM budget for every entrypoint. Met.** `contracts/budget/tests/instruction_budget.rs`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/budget/tests/instruction_budget.rs),
added at https://github.com/ZKELLA-org/zkella/commit/dc84c450d49de8bef007d7e35c4537724cd019b3,
measures every non-proof state-changing entrypoint (38 of them) against the real compiled
`wasm32v1-none` binaries. Results in `docs/SECURITY_TOOLING_REPORT.md`'s budget table; the
highest is `merkle_root` at 0.55% of the mainnet limit.

---

## Deliverable 4: Full Testnet Deployment & Operational Runbook Drill

**Original description:** *"Deploy the viewing keys contract, which has no live deployment at
all today, and redeploy compliance and governance to carry the pause mechanism, and for
governance the guardian role, added earlier in this tranche [...]. This deliverable also
develops an operational runbook and incident-response plan covering deployment and
configuration of contracts and the indexer; monitoring of RPC health, indexer sync lag,
contract state, and transaction failures; key-management and secret-rotation procedures; and
escalation paths and rollback steps for suspected misconfiguration or degradation."*

**Original success criteria:**
1. Viewing keys deploys to Testnet for the first time, and compliance and governance redeploy carrying the pause mechanism, and for governance the guardian role, all three correctly wired to the existing token, verifier, and swap contracts from Tranches 1–2 (governance as verifier's admin, compliance referencing the shared verifier registry), and the complete set of all six contract addresses is published.
2. The Tranche 2 indexer is confirmed operating correctly against the token and swap addresses actually in use after this deployment, unchanged from Tranches 1-2 in the expected case, or repointed here if either contract was redeployed due to a Deliverable 3 finding.
3. An operational runbook exists, covers all four incident categories and the full minimum operating checklist, and is written against this real Testnet deployment.
4. A scheduled script automates the runbook's RPC-health, indexer-health, and contract-state checks and posts a notification on failure.
5. At least one incident-response drill has been run against the Testnet indexer and contracts, including a case surfaced by that automated check rather than a manual read of the checklist, with findings incorporated back into the runbook.
6. This deployment is executed through a scripted, repeatable deployment pipeline rather than manual, hand-typed CLI invocations.

### Evidence and proof

**1. Six-contract stack, fully wired. Met.** `deployments.json`'s `testnet_final` block
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/deployments.json),
addresses published above. Deployed and wired by `scripts/testnet_deploy_stack.sh`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/scripts/testnet_deploy_stack.sh),
in order verifier → governance → token → swap → compliance → viewing_keys, governance set as
verifier's and token's admin.

**2. Indexer confirmed against the live stack. Met.** The indexer was pointed at the current
token from its deploy ledger and reached tip with zero lag, confirmed live via
`scripts/testnet_health_check.sh`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/scripts/testnet_health_check.sh)
and by a live shield, transfer, unshield and compliance publish each being indexed and read
back correctly.

**3. Operational runbook. Met.** `docs/RUNBOOK.md`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/RUNBOOK.md),
covering all four incident categories and the full minimum operating checklist, written
against the addresses actually deployed.

**4. Scheduled, automated check with a real notification. Met.** `scripts/testnet_health_check.sh`
(link above) checks RPC, indexer and contract state and posts on failure. It is actually
installed and running — `*/15 * * * *` via cron on the operating host, confirmed active — not
just documented as possible. Failures post to a real `ntfy.sh` channel (plain-text payload,
fixed at https://github.com/ZKELLA-org/zkella/commit/55b51e9c829b56633575dcdcca63c3157bdeeea7
after the original JSON payload turned out not to be parsed by that provider).

**5. A real incident-response drill, with a case surfaced by the automated check. Met.**
`docs/RUNBOOK.md`'s "Drill record"
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/RUNBOOK.md):
- First drill (2026-10-05): three faults injected into the health check's own inputs (RPC
  unreachable, indexer unreachable, a misconfigured governance address), all caught; found and
  fixed a diagnostic gap (failure messages didn't name the contract address).
- Second drill (2026-10-08, after the audit redeploy): a **real** Category 2 incident — the
  indexer process was genuinely stopped, not simulated. The scheduled check's own command
  caught it for real and posted a real alert to the team's `ntfy.sh` channel, confirmed
  received by a person before any recovery step was taken. `RUNBOOK.md` Category 2's own steps
  were then followed for real: process checked, restarted against the existing database,
  confirmed it resumed from its persisted cursor (`sync_state.last_synced_ledger`), not from the
  configured start ledger — verified directly in the database, not inferred. `token.merkle_root()`
  was confirmed still readable straight from the contract while the indexer was down. Found and
  fixed a real diagnostic gap: the indexer's own startup log always printed the configured start
  ledger even when actually resuming from a far later persisted cursor, misleading during a real
  incident — fixed at
  https://github.com/ZKELLA-org/zkella/commit/7ab5939efab3f4b96c4d548610c8d4f2aab5121e.

**6. Scripted, repeatable deployment. Met.** `scripts/testnet_deploy_stack.sh` (link above)
deploys, initializes, queues and executes all six verifying keys, approves the native asset,
and sets the sanctions root, with a `RESUME` mode for continuing after a transient failure
(used for real: a DNS failure interrupted the first deploy attempt, and the audit-fix redeploy
on 2026-10-08 used this same script end to end, not a one-off hand-typed sequence).

---

## Deliverable 5: TypeScript SDK, Modules, Stabilization & Release (Testnet)

**Original description:** *"Create TypeScript SDK modules providing developers with: (1) note
and commitment builders; (2) proof generation helpers wrapping the WASM circuits [...]; (3)
transaction assembly and submission utilities; (4) an indexer client for querying notes, Merkle
paths, and nullifier state [...]; (5) complete example implementations. Stabilize this API
surface [...] and release it as a versioned v1.0 npm package with TypeScript typings and full
API documentation, plus developer documentation covering installation, Testnet configuration,
API reference, example code, indexer usage, and troubleshooting. [...] Scoped to Stellar
Testnet only; mainnet network configuration is out of scope for this release."*

**Original success criteria:**
1. SDK modules for note/commitment building, proof generation, transaction assembly, and the indexer client are implemented.
2. ZKELLA SDK v1.0 is published on npm with TypeScript typings and a stabilized public API, targeting Stellar Testnet.
3. The SDK's Testnet network configuration includes all six deployed contract addresses and the correct indexer endpoint.
4. Example code for shield, transfer, unshield, viewing-key, indexer, and shielded-swap flows runs successfully against the Testnet deployment.
5. Developer documentation is available and covers installation, Testnet configuration, API reference, indexer usage, and troubleshooting.
6. Deliverable 4's operational runbook now includes an SDK-artifacts section covering version pinning, upgrade guidance, and client-side integration troubleshooting.
7. The wallet's submission path retries a transient RPC failure and resubmits against a fresh anchor if the original ages out of the root-history window, confirmed by a test simulating both failure modes.
8. The swap, auditor, and compliance wrapper classes call the real deployed contracts and provers rather than returning stub placeholder values, confirmed by each producing a real, verified live transaction.

### Evidence and proof

**1. SDK modules. Met.** `sdk/src/keys`, `sdk/src/notes`, `sdk/src/prover`, `sdk/src/wallet`,
`sdk/src/compliance`, `sdk/src/indexer`
(https://github.com/ZKELLA-org/zkella/tree/compliance-governance-security-testnet-release/sdk/src).

**2. Published on npm. Met, as `0.1.1`, deliberately not `1.0.0`.** `npm view @zkella/sdk
version` returns `0.1.1`: https://www.npmjs.com/package/@zkella/sdk. (First published as `0.1.0`
on 2026-10-05; bumped to `0.1.1` to add the package's README, a bundled `LICENSE`, and complete
`package.json` metadata — no dependency or API change.) `docs/SDK_RELEASE.md`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/SDK_RELEASE.md)
records the decision — semver reserves `1.0.0` for an API-stability commitment, and the team
chose to say that honestly with a pre-1.0 first release rather than overclaim a `1.0` the
original roadmap wording assumed. The API itself (typings, documented surface, Testnet
targeting) is exactly what the criterion asks for otherwise.

**3. Testnet network configuration. Met.** `sdk/src/config/testnet.ts`'s `TESTNET_CONTRACTS`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/sdk/src/config/testnet.ts);
`tests/unit/testnet-config.test.ts`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/tests/unit/testnet-config.test.ts)
fails the build if it drifts from `deployments.json`.

**4. Example code, all six, live — and re-verified against the current stack specifically.
Met.**

*First run, original stack (now `testnet_final_superseded_2026_10_08`):*
- Shield: https://stellar.expert/explorer/testnet/tx/1126edc56b34bc38836eb9d14a12450fa80c763332604238f8bdc7fc8e62cbc6 (leaf 0).
- Indexer query, viewing-key audit (5 receipts), transfer:
  https://stellar.expert/explorer/testnet/tx/0b53568b5ade7885f915a23a65bdf053e5acc45b4bb9d3599ddc7e1a6fdaf444,
  unshield: https://stellar.expert/explorer/testnet/tx/f81c0016e472d8d8a9d6a15caa2778983ae4ee941fa7f0ae9c1095910fdee795,
  swap commit: https://stellar.expert/explorer/testnet/tx/8502c742c674ac87677d90a367ca870f9ac11b372c0bb4d2209f3e521858bb0e,
  swap cancel: https://stellar.expert/explorer/testnet/tx/d3d7565947e679959ff9731cfe62bf21dfa70d5abc0193ffd9ab4d63d2b64887.
  A separate, earlier full commit → execute → reveal lifecycle also ran on this stack (claimed
  into leaf 4; see `deployments.json`'s `testnet_final_superseded_2026_10_08._live_checks`).

*Second run, current post-audit stack:*
- Shield (leaves 4–5), indexer query (5 notes, 32-level Merkle path), viewing-key audit (4
  receipts, zero-value padding notes correctly excluded), transfer (new leaves 6–7), unshield
  (change note at leaf 8).
- Compliance publish: https://stellar.expert/explorer/testnet/tx/533837bf63d88ce09578940d4bec9de94d60e54b4babc99b1cd09c5d419d0442.
- Full swap lifecycle through the fixed `ZKELLASwap` wrapper:
  shield https://stellar.expert/explorer/testnet/tx/d8b3c78e0614764f695a43dae0ca6da2801a68bded4b709122d134069cdfd973,
  commit_swap https://stellar.expert/explorer/testnet/tx/d84ff18f304765f1fb9a1f93f8b41a24dbad95116705256b711e4f5a27676ec6,
  execute_swap https://stellar.expert/explorer/testnet/tx/9aa8b8cd7cb0f335f109d6387334fcaac17aad9cc033518c415f57eebe023120,
  reveal_and_claim https://stellar.expert/explorer/testnet/tx/1320f2a101e0adaf9475d0d00b76082cbc59105b99885ad66e090b6f6cf1e1fc.

Two real bugs surfaced by the first run's live testing, not found by unit tests alone: the
transfer example didn't pass the recipient's owner key and pointed at circuit artifact paths
`circuits/build.sh` doesn't produce
(https://github.com/ZKELLA-org/zkella/commit/d471c5c7381c76a44abd57cc623c635e0b50a1fe); the
swap example quoted 99% of the requested amount instead of the input note's full value, which
the fairness check correctly rejected
(https://github.com/ZKELLA-org/zkella/commit/7df8b8b417c92406c14174253085d1127d2f1f44).

**5. Developer documentation. Met.** `docs/SDK_DEVELOPER.md`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/SDK_DEVELOPER.md).

**6. Runbook SDK-artifacts section. Met.** `docs/RUNBOOK.md`, "SDK artifacts"
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/RUNBOOK.md),
added at https://github.com/ZKELLA-org/zkella/commit/13756e0b8a8af2cb444f21d9077be0d05620eca4.

**7. Retry and resubmit logic, tested. Met.** `tests/unit/wallet-resilience.test.ts`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/tests/unit/wallet-resilience.test.ts),
added at https://github.com/ZKELLA-org/zkella/commit/241556203710dabd2b21d7c4da29adfeb29c7700:
a transient failure that never reached the network is retried and succeeds; an ambiguous
failure whose transaction did land is not resubmitted (checked by hash first — the one way a
naive retry could double-spend); a contract rejection is not retried; an evicted anchor
rebuilds the call against a fresh root.

**8. Wrapper classes call real contracts, each with a real transaction. Met.**
`ZKELLASwap` (https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/sdk/src/wallet/swap.ts,
replaced from a stub at https://github.com/ZKELLA-org/zkella/commit/f49797bb3a47a6a50da4ca8dde05f27ce08b7bb3),
`ZKELLAAuditor` (https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/sdk/src/wallet/auditor.ts),
`ZKELLACompliance` (https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/sdk/src/compliance/compliance.ts,
replaced from a stub at https://github.com/ZKELLA-org/zkella/commit/114cb12ddc26048b9a3247468c865412a72c2d83) —
each confirmed by the live transactions cited in criterion 4 above.

---

## Independent audit pass and fixes (2026-10-08)

A second, independent review — `token`, `verifier`+`governance`, `swap`, `compliance`+`viewing_keys`,
the SDK, and the indexer each reviewed separately — found and fixed five real defects, the most
severe since the swap incidents `contracts/swap/src/lib.rs`'s own doc comments already
reference:

- **Critical, `swap`.** `reveal_and_claim` never bound the output note's own randomness to
  anything fixed at `commit_swap` time, only its owner key. Anyone who observed a pending reveal
  could resubmit the same fairness proof with their own `out_rho`/`out_rcm`, permanently
  destroying the escrowed `amount_out` at zero cost. Fixed at
  https://github.com/ZKELLA-org/zkella/commit/a09e48b3db4d69f1b7efccfe26a99dd8b7d2b6c3
  with `out_note_binding_hash`, committed at `commit_swap` and re-checked at reveal; the SDK's
  `ZKELLASwap` wrapper now builds and commits to the output note at `commitSwap` time instead of
  generating a fresh one at reveal.
- **High, `governance`/`verifier`/`token`.** `verifier.pause()` and `token.pause()`, both
  documented in `docs/RUNBOOK.md` as primary incident-response levers, were completely
  unreachable — governance is each one's admin, and neither had a function that actually calls
  through. Fixed at
  https://github.com/ZKELLA-org/zkella/commit/1ff6a13002e1cc7ac4c3318bad7d14bb15b7e5e1
  with `pause_verifier`/`unpause_verifier` and `pause_token`/`unpause_token` — both live-confirmed
  to actually block their target (`Error(Contract, #10)`/`#3` == `Paused`) and restore on
  unpause.
- **Medium, `token`.** `transfer()`/`transfer4()`/`unshield()` never validated `encrypted_note`
  length, unlike `shield()`. A malformed ciphertext could pass proof verification (the proof
  only binds the commitment) while permanently stranding the recipient's ability to decrypt and
  spend that note. Fixed at
  https://github.com/ZKELLA-org/zkella/commit/219cfe28e528c1a08aff58be7de9b77853a9a6fd.
- **Medium, SDK.** `ZKELLAAuditor.sync()` had no dedup; a second call — the normal way to keep
  an audit view current — would double-count every receipt. Fixed at
  https://github.com/ZKELLA-org/zkella/commit/afe5290a9669d2881a0e62f1eff11f8d8cc6883b.
- **Medium + Low, indexer.** `/merkle/root` and `/merkle/path` could serve a root/path pair
  computed against two different ledgers (two independent, unsynchronized RPC simulations), and
  `RateLimiter` never evicted an identity once seen (unbounded memory growth). Fixed at
  https://github.com/ZKELLA-org/zkella/commit/45eb39de7816f8c78bfb1a4e21c244747b1f3baf.

`contracts/compliance`/`contracts/viewing_keys` held up with no fixable finding.

**A sixth finding, caught by CI rather than by review.** This branch's push trigger was only
added to CI after the fixes above
(https://github.com/ZKELLA-org/zkella/commit/26f80e5048451a844c169b2d04b74e73330769ec), and its
first real run failed: the swap fix's new `out_note_binding` parameter on `commit_swap` broke
`contracts/token/fuzz/fuzz_targets/swap_arbitrary.rs`'s own call to it — a straight compile
error in the fuzz crate, not a runtime crash, since the harness still passed the old
14-argument list. Fixed at
https://github.com/ZKELLA-org/zkella/commit/324885b747f3cf39d8e8e6ff19d9fa1d0745e268
by adding the missing argument and re-minimizing the corpus; confirmed locally by running all
nine targets with the exact flags CI uses before pushing. Worth recording plainly: the audit
pass's own test suite (`cargo test --workspace`) never exercises the separate fuzz crate, so
this kind of drift between a contract's signature and its fuzz harness is exactly the gap CI's
dedicated fuzz-build step exists to catch — and, once wired up on this branch, did.

All fixes shipped with regression tests, and the full stack was redeployed
(https://github.com/ZKELLA-org/zkella/commit/4661d5ca4b00ec5248c141b0903694c11690e7ab) — the
prior stack predated every one of these fixes, so none of them protected anything until
redeployed. See Deliverable 5's criterion 4 above for the full live re-verification this
redeploy received.

## Test totals at the time of writing

Full contract workspace (`cd contracts && cargo test --workspace --release`): 237 tests,
all passing — token 127, swap 30, verifier 37, governance 25, compliance 13, viewing_keys 5
(`token-interface` and `verifier-interface` have no tests of their own; they're trait
definitions). JS unit tests (`DATABASE_URL=postgres://... npx jest tests/unit`): 188 passing, 0
skipped, including `tests/unit/indexer-db-postgres.test.ts` against a real PostgreSQL server
(without `DATABASE_URL` set, that one file's 9 tests are skipped rather than failed, so a local
`npm test` run doesn't require standing up Postgres just to pass). SDK typecheck (`cd sdk && npx tsc --noEmit`)
passes with no errors.

## What is left open, honestly

- **Admin multisig** on every contract remains a single key, by design deferred to a mainnet
  deployment decision (`docs/GOVERNANCE.md`).
- **The live drill so far involved two people**, not a full team walkthrough — the first,
  tabletop drill was run solo.
- **Fuzzing is smoke-level** (60-to-90-second runs), not a sustained campaign, even though all
  nine targets now have a committed, minimized corpus.
- **No independent third-party audit** has been done; `docs/SECURITY_TOOLING_REPORT.md` says
  so explicitly, and that remains the honest status.
- **The SDK is `0.1.1`**, not `1.0.0` — a deliberate pre-1.0 release, not the literal version the
  original roadmap wording named; see `docs/SDK_RELEASE.md`.
- **`swap`'s `reclaim_expired_swap` recovery path** (the post-*execution* unwind, for a relayer
  that fronted liquidity but the claimant never claims) is still unit-tested only. Its claim
  window is `CLAIM_WINDOW_LEDGERS = 17_280` ledgers (~24 hours at 5s/ledger) after the swap's
  original expiry, which makes a live exercise a multi-hour undertaking, not something a single
  session can close inline — unlike `cancel_swap`'s pre-execution path, which was exercised
  live twice.
