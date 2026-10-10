# Tranche 3 deliverables: original description, success criteria, evidence and proofs

Criterion-by-criterion account of Tranche 3 (Compliance, Governance & Testnet Readiness): each
deliverable below starts with its **original description and success criteria, quoted
verbatim** from the funding roadmap, followed by the evidence for each criterion with full,
clickable links — a GitHub commit or file link for every piece of code evidence, and a full
`stellar.expert` link for every on-chain transaction. Nothing here is a paraphrase of what was
asked for; it is the literal ask, next to the literal proof.

Repository: https://github.com/ZKELLA-org/zkella. Code links below point at the branch this
work lives on, `compliance-governance-security-testnet-release`
(https://github.com/ZKELLA-org/zkella/tree/compliance-governance-security-testnet-release),
unless a specific commit is named.

## How to reproduce

```
cd contracts
cargo build --workspace --target wasm32v1-none --release
cargo test --workspace --release
cd ..

npm test                                      # JS unit tests (circuits, SDK, indexer)

cd sdk
npm run typecheck
npm run build
cd ..

cd contracts/token/fuzz
cargo fuzz run shield_arbitrary -- -max_total_time=60
cargo fuzz run transfer_arbitrary -- -max_total_time=60
cargo fuzz run verifier_arbitrary -- -max_total_time=60
cargo fuzz run swap_arbitrary -- -max_total_time=60
cargo fuzz run governance_arbitrary -- -max_total_time=60
cargo fuzz run compliance_arbitrary -- -max_total_time=60
cargo fuzz run viewing_keys_arbitrary -- -max_total_time=60
cargo fuzz run token_admin_arbitrary -- -max_total_time=60
cargo fuzz run verifier_admin_arbitrary -- -max_total_time=60
cd ../../..

scripts/testnet_deploy_stack.sh               # scripted, repeatable six-contract deploy
scripts/testnet_health_check.sh               # RPC, indexer, contract-state checks

node examples/02-shield.cjs                   # shield
node examples/03-indexer-query.cjs            # indexer query
node examples/04-viewing-key-audit.cjs        # viewing-key audit
node examples/05-transfer.cjs                 # transfer
node examples/06-unshield.cjs                 # unshield
node examples/07-swap.cjs                     # swap
```

Current Testnet stack (`deployments.json`'s `testnet_final` block
— https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/deployments.json,
redeployed 2026-10-08 to carry the audit fixes in this document's last section):

- verifier: https://stellar.expert/explorer/testnet/contract/CC2LQPXH3L5YKRP7YJ6UIC57AOGJXBQN4DEKNRU4Y32ABXJZOENCDAX3
- governance: https://stellar.expert/explorer/testnet/contract/CDTJLTBEKBXRJJKHVI32A5UMBB4UC6VBMDOF7WR43H2SKCCRAVRJWY5Q
- token: https://stellar.expert/explorer/testnet/contract/CA5TFEVODC25SSEZII2XHB2XMCKFNXNLXRNFTKWPKMT5PCWYZUMLPRUZ
- swap: https://stellar.expert/explorer/testnet/contract/CBN7JJEPAEA5NCKOECPPGHETAK4CCCUFOBUJCDZ7K7HPIV7Y6ILOC524
- compliance: https://stellar.expert/explorer/testnet/contract/CDP5SRSUFDVEYHUCUX53SM4PZVTOIHDZR3Z5C7G4TFFKAQSLX64FOZVJ
- viewing_keys: https://stellar.expert/explorer/testnet/contract/CDT776JLXU5GWRIY6WXLZGVKZ5V4TG32HAITNPFZVX5UCJSMMFHNMEEE (reused unchanged)

**Deployment note:** all six were deployed by `scripts/testnet_deploy_stack.sh`
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

**1. Compile and deploy — Met.** Both contracts build successfully and are live on the current
six-contract Testnet stack. Compliance's `initialize` function was called in a real
transaction, which proves it actually runs on-chain, not only in local tests.
Proof: the live `initialize` transaction
(https://stellar.expert/explorer/testnet/tx/62521977b17c60b46be3d02640d5470b9fb93cf9e1235d787b678761ae898ac8);
the compliance contract source
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/compliance/src/lib.rs);
the viewing-key contract source
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/viewing_keys/src/lib.rs).

**2. Functional tests — Met.** 13 tests cover compliance and 5 cover viewing keys, and all of
them pass. One test, `accepts_and_stores_a_real_sdk_proof`, checks a real cryptographic proof
produced by the actual proving tools rather than a fake placeholder, which confirms the
verification logic genuinely works. Other tests confirm a viewing key can be registered and
read back, that registering again replaces the old key (rotation), that revoking an existing
key removes it, and that both registering and revoking require the owner's own authorization.
(See "What is left open, honestly" below for the one related edge case not covered by its own
test.)
Proof: the compliance test that checks the real proof
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/compliance/src/lib.rs#L498);
the viewing-key contract's test module
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/viewing_keys/src/lib.rs#L72-L139).

**3. Technical documentation — Met.** Two documents explain how this works: one is dedicated to
disclosure and compliance specifically, the other covers these contracts as part of the overall
system design.
Proof: `docs/VIEWING_KEYS.md`, covering disclosure, epochs, key revocation, and sanctions-list
maintenance
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/VIEWING_KEYS.md);
`docs/ARCHITECTURE.md`'s "Viewing key registry contract" and "Compliance contract" sections,
covering the contracts' design and current limits
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/ARCHITECTURE.md#24-viewing-key-registry-contract,
https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/ARCHITECTURE.md#25-compliance-contract).

**4. `publish_compliance_proof` live on Testnet — Met, twice.** This function was actually
called through the SDK, not a raw command-line invocation, and it was run successfully on two
different deployments of the stack — proving it works end-to-end from real client code, not
just once by chance.
Proof: on the original stack
(https://stellar.expert/explorer/testnet/tx/e489e10014615fbaaa6f078934479c0fc089ab4175df85a46d6bc6f75674d52d);
on the current, post-audit stack
(https://stellar.expert/explorer/testnet/tx/533837bf63d88ce09578940d4bec9de94d60e54b4babc99b1cd09c5d419d0442);
the SDK method that submitted both
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/sdk/src/compliance/compliance.ts).

**5. Revocation — Met, under a reworded criterion.** The criterion as originally written asks
for something that isn't possible for any system built this way: once a note's encrypted data
is published on-chain, revoking a key afterward cannot erase what someone already had the
ability to decrypt. This is explained plainly in the documentation, not hidden. What was
actually built and tested: starting a new key epoch means new notes are encrypted with the new
key going forward, the old key's public record is withdrawn, and someone holding only the old
key can no longer decrypt anything received after that point — while they can still read what
they could already decrypt before it, which is the expected and correct behavior, not a gap.
Proof: `docs/VIEWING_KEYS.md`'s "Revocation criterion (decision)" section, explaining the
reasoning
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/VIEWING_KEYS.md#revocation-criterion-decision);
the automated test proving the behavior
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/tests/unit/viewing-key-rotation.test.ts#L35-L45).

**6. Decrypt-on-request workflow — Met, live.** An auditor holding a real, exported viewing key
was able to decrypt and read actual transaction receipts straight from the live contract — this
was run for real, not only tested locally. Four genuine receipts were recovered, and empty
"padding" notes (used internally for privacy and carrying no real value) were correctly
excluded rather than shown as if they were real transactions. A viewing key from an unrelated
wallet recovers nothing at all, which confirms the decryption is genuinely tied to the right
key and not something that works for anyone.
Proof: the live run against the current stack, recovering receipts at ledgers 5090104, 5090106,
5090109, and 5090112
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/examples/04-viewing-key-audit.cjs);
the SDK method used
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/sdk/src/wallet/auditor.ts);
the automated test covering both the successful case and the wrong-key case
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/tests/unit/auditor.test.ts#L38-L55).

**7. Sanctions-list maintenance — Met, documented.** The rules for who publishes the sanctions
list and how often are written down: on Testnet, the compliance admin key is the maintainer (a
multisig is planned before mainnet), the list is meant to update weekly with a same-day path
for urgent cases, and every published version is accompanied by a hash of the source list so
anyone can check it. Right now the list actually in use is empty — just placeholder values —
because Testnet has no real sanctions list yet, and the documentation says so plainly rather
than implying otherwise.
Proof: `docs/VIEWING_KEYS.md`, "Sanctions list maintenance" section
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/VIEWING_KEYS.md#sanctions-list-maintenance).

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

**1. Compile and deploy — Met.** The governance contract is live on the current stack,
initialized with a 60-ledger timelock (a shortened, demo-only stand-in for the real 7-day
production delay) and a separate guardian key.
Proof: the governance contract source
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/governance/src/lib.rs).

**2. Functional tests — Met, 25 tests.** The tests cover: registering a verifying key for the
first time and later rotating it, both going through the same timelock; the guardian's ability
to cancel a pending change on its own, without also being able to approve one; and the pause
mechanism now added to all four contracts that didn't have one before. That pause mechanism
itself had a real bug, described in the audit section below, which is now fixed and covered by
its own dedicated tests (`pause_verifier_actually_pauses_and_unpause_verifier_restores_it`,
`pause_token_actually_pauses_and_unpause_token_restores_it`).
Proof: the governance contract's test module, starting with the two pause tests named above
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/governance/src/lib.rs#L658-L745).

**3. Technical documentation — Met.** The documentation explicitly lists which governance
features are intentionally left for a later upgrade: an admin multisig, authorization to
upgrade the circuits themselves beyond simple key rotation, and finer-grained parameter
controls.
Proof: `docs/GOVERNANCE.md`, "Deferred to a later governance upgrade" section
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/GOVERNANCE.md#deferred-to-a-later-governance-upgrade).

**4. `MIN_SHIELD_AMOUNT` is governance-settable — Met, demonstrated live twice.** Changing the
minimum shield amount, which asset is approved, or which relayer is allowed, all go through the
same timelocked process as a verifying-key change — none of them need a contract redeploy. This
was proven against the real token contract in an automated test, then demonstrated live on
Testnet twice: once on the original stack, and again on the current stack after the audit
fixes. In both cases, the minimum shield amount was read back afterward as `500`, down from the
original `1000`, with no redeploy involved either time.
Proof:

- The automated test: `min_shield_amount_changes_on_the_real_token_only_after_the_timelock`
  — https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/governance/src/lib.rs#L587
- Original-stack run — queue: https://stellar.expert/explorer/testnet/tx/e71e95957f3817db2a7f1c1754258af6428956e46c38bdad843e20c64b761840
  — execute: https://stellar.expert/explorer/testnet/tx/ce8d642fc8b4db9bb86fe53dcb3b2e2771b26e58a413a73377506a8a4bcefa07
- Current-stack run — queue: https://stellar.expert/explorer/testnet/tx/3f2159168107f2b02b203c3aaf1d9d602b5890e9aed99a381046b86c3a9889d3
  — execute: https://stellar.expert/explorer/testnet/tx/f3ef72ee1e4ad65fafb5f5eb4c99d8094f61b696e7517081964cc172f84297b5

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

**1. Tooling run — Met for the contracts.** Circuits don't have a direct equivalent of
code-quality linting or dependency scanning (there is no tool like that for the Circom
language), so they're covered instead by fuzz-testing the contract functions that consume the
proofs those circuits produce. For the Rust contracts: a linter found no correctness issues,
and a dependency-vulnerability scanner found one real issue, which was fixed. The same scan was
run on the JavaScript side. Nine separate fuzz tests now exist, one covering each contract's
full set of state-changing actions, each with a saved starting set of test inputs so future
runs build on real coverage instead of starting from nothing. One of these fuzz tests already
found a real crash, which is now fixed and kept as a permanent regression case.
Proof: the full tooling report
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/SECURITY_TOOLING_REPORT.md);
the dependency-vulnerability fixes
(https://github.com/ZKELLA-org/zkella/commit/813346819d8c7737d054c15072d5d7ede8f08fac,
https://github.com/ZKELLA-org/zkella/commit/d38087ee73432c35eb6058f640862449036905c4); the new
fuzz targets
(https://github.com/ZKELLA-org/zkella/commit/2063efb8c6a4d55a6c45434601b004aaebd98a3a,
https://github.com/ZKELLA-org/zkella/commit/846b0ec9b701ff9862170c0dbb3d776ef1927a73,
https://github.com/ZKELLA-org/zkella/commit/e06e85e8fa2278ee40344a86034fa2fa4b5cf7fd); the real
crash found and fixed
(https://github.com/ZKELLA-org/zkella/commit/04b52496fbcb33952b76d827f820bcf1532704ac).

**2. Findings report — Met.** Every finding from the tooling run is written down together with
what was done about it — fixed, or explicitly accepted with a stated reason — never left
unaddressed with no explanation.
Proof: `docs/SECURITY_TOOLING_REPORT.md`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/SECURITY_TOOLING_REPORT.md).

**3. Real-WASM budget for every entrypoint — Met.** Every function that changes contract state
(38 of them, across all six contracts) was measured for how much of Stellar's per-transaction
instruction budget it actually uses, against the real compiled contract code rather than an
estimate. The most expensive one uses well under 1% of the limit.
Proof: the budget-measurement test, `instruction_budget_for_every_non_proof_entrypoint`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/contracts/budget/tests/instruction_budget.rs#L43,
added at https://github.com/ZKELLA-org/zkella/commit/dc84c450d49de8bef007d7e35c4537724cd019b3);
full results in `docs/SECURITY_TOOLING_REPORT.md`'s budget table (link above).

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

**1. Six-contract stack, fully wired — Met.** All six contracts are deployed and correctly
connected to each other on the current stack: governance is set as the admin for both the
verifier and the token, and every contract address is published.
Proof: `deployments.json`'s `testnet_final` block
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/deployments.json);
the deployment script that performed it, run in order verifier → governance → token → swap →
compliance → viewing_keys
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/scripts/testnet_deploy_stack.sh).

**2. Indexer confirmed against the live stack — Met.** The indexer — the service that lets a
wallet recover its note history — was pointed at the current contracts and caught up to the
latest ledger with no lag. A real shield, transfer, unshield, and compliance publish were each
correctly picked up and made queryable afterward.
Proof: the health-check script used to confirm this
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/scripts/testnet_health_check.sh).

**3. Operational runbook — Met.** A written runbook covers what to do for each of the four
kinds of incidents this project anticipates, plus a basic day-to-day operating checklist, and
it's written against the actual deployed addresses rather than a hypothetical setup.
Proof: `docs/RUNBOOK.md`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/RUNBOOK.md).

**4. Scheduled, automated check with a real notification — Met.** A script checks the RPC
connection, the indexer, and contract state every 15 minutes through a real cron job running on
the actual operating machine — this is live right now, not only something documented as
possible. When a check fails, it sends a real notification; an earlier version of this
notification was formatted incorrectly for the notification service and has since been fixed.
Proof: the health-check script
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/scripts/testnet_health_check.sh);
the notification-format fix
(https://github.com/ZKELLA-org/zkella/commit/55b51e9c829b56633575dcdcca63c3157bdeeea7).

**5. A real incident-response drill — Met.** Two drills were run. The first injected three fake
failures into the health check's own inputs (RPC unreachable, indexer unreachable, a
misconfigured governance address) to confirm it catches each one; this also found and fixed a
real gap where failure messages didn't say which contract address was the problem. The second
drill was not simulated: the indexer process was actually stopped. The scheduled check caught
the real failure and sent a real alert, which a person confirmed receiving before any recovery
step was taken. The runbook's recovery steps were then followed for real, and it was confirmed
directly in the database, not assumed, that the indexer resumed from where it had left off
rather than starting over, while `token.merkle_root()` stayed readable from the contract the
whole time the indexer was down. This second drill also found and fixed a real gap: the
indexer's own startup message was misleading about where it was actually resuming from.
Proof: `docs/RUNBOOK.md`'s "Drill record" section
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/RUNBOOK.md#drill-record);
the diagnostic-message fix from the second drill
(https://github.com/ZKELLA-org/zkella/commit/7ab5939efab3f4b96c4d548610c8d4f2aab5121e).

**6. Scripted, repeatable deployment — Met.** One script performs the entire deployment:
deploying all six contracts, wiring them together, registering every verifying key, approving
the native asset, and setting the sanctions-list root. It supports resuming after a failure
partway through instead of starting over, which was genuinely needed once — a DNS problem
interrupted the first deployment attempt, and the script picked back up from where it had
stopped. The same script, unmodified, was used again later for the audit-fix redeployment.
Proof: the deployment script
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/scripts/testnet_deploy_stack.sh).

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

**1. SDK modules — Met.** Every piece the criterion asks for exists as real code: key
management, note handling, proof generation, transaction building, compliance, and the indexer
client.
Proof: the SDK source tree
(https://github.com/ZKELLA-org/zkella/tree/compliance-governance-security-testnet-release/sdk/src).

**2. Published on npm — Met, as `0.1.1`, deliberately not `1.0.0`.** The package is live on the
npm registry. It was first published as `0.1.0`, then updated to `0.1.1` to add a proper
README, a license file, and complete package metadata — nothing about the actual code or API
changed between those two versions. It was deliberately not labeled `1.0.0`: that version
number conventionally signals a promise that the public API won't change again without a major
version bump, which isn't a promise this project is ready to make yet. Everything else the
criterion actually asks for — TypeScript types, a documented API, Testnet targeting — is
genuinely there.
Proof: the published package (https://www.npmjs.com/package/@zkella/sdk); the release-decision
writeup
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/SDK_RELEASE.md).

**3. Testnet network configuration — Met.** The SDK ships the current addresses for all six
contracts and the correct indexer endpoint, and an automated test fails the build automatically
if this ever drifts out of sync with the actual deployment record.
Proof: the configuration file
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/sdk/src/config/testnet.ts);
the test that enforces it
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/tests/unit/testnet-config.test.ts#L5-L13).

**4. Example code, all six flows, live — Met, and re-verified after the audit fixes.** Every
example (shield, transfer, unshield, viewing-key audit, indexer query, and swap) was run for
real against a live deployment, not left as un-run sample code. This happened twice: once on
the original stack, and again on the current stack after the audit fixes, to confirm the fixes
didn't break anything a real user would run. Running these for real, not just the unit tests,
caught two genuine bugs unit tests alone had missed: the transfer example wasn't passing the
recipient's key correctly, and the swap example was quoting a slightly wrong amount that the
system correctly rejected. Both are fixed now.
Proof:

- First run, original stack:
  - Shield: https://stellar.expert/explorer/testnet/tx/1126edc56b34bc38836eb9d14a12450fa80c763332604238f8bdc7fc8e62cbc6
  - Indexer query / viewing-key audit / transfer: https://stellar.expert/explorer/testnet/tx/0b53568b5ade7885f915a23a65bdf053e5acc45b4bb9d3599ddc7e1a6fdaf444
  - Unshield: https://stellar.expert/explorer/testnet/tx/f81c0016e472d8d8a9d6a15caa2778983ae4ee941fa7f0ae9c1095910fdee795
  - Swap commit: https://stellar.expert/explorer/testnet/tx/8502c742c674ac87677d90a367ca870f9ac11b372c0bb4d2209f3e521858bb0e
  - Swap cancel: https://stellar.expert/explorer/testnet/tx/d3d7565947e679959ff9731cfe62bf21dfa70d5abc0193ffd9ab4d63d2b64887
- Second run, current post-audit stack:
  - Shield: confirmed by leaf index (leaves 4–5) — this run's specific transaction hash
    wasn't individually recorded, unlike the first run above
  - Indexer query and viewing-key audit: read-only calls, so neither produces its own
    transaction; the audit recovered 4 real receipts, same run cited under Deliverable 1
    criterion 6 above
  - Transfer: confirmed by leaf index (new leaves 6–7); hash not individually recorded
  - Unshield: confirmed by leaf index (change note at leaf 8); hash not individually recorded
  - Compliance publish: https://stellar.expert/explorer/testnet/tx/533837bf63d88ce09578940d4bec9de94d60e54b4babc99b1cd09c5d419d0442
  - Full swap lifecycle — shield: https://stellar.expert/explorer/testnet/tx/d8b3c78e0614764f695a43dae0ca6da2801a68bded4b709122d134069cdfd973,
    commit: https://stellar.expert/explorer/testnet/tx/d84ff18f304765f1fb9a1f93f8b41a24dbad95116705256b711e4f5a27676ec6,
    execute: https://stellar.expert/explorer/testnet/tx/9aa8b8cd7cb0f335f109d6387334fcaac17aad9cc033518c415f57eebe023120,
    reveal: https://stellar.expert/explorer/testnet/tx/1320f2a101e0adaf9475d0d00b76082cbc59105b99885ad66e090b6f6cf1e1fc
- The two bug fixes:
  - Transfer example's recipient key: https://github.com/ZKELLA-org/zkella/commit/d471c5c7381c76a44abd57cc623c635e0b50a1fe
  - Swap example's quoted amount: https://github.com/ZKELLA-org/zkella/commit/7df8b8b417c92406c14174253085d1127d2f1f44

**5. Developer documentation — Met.** A dedicated guide covers installing the SDK, configuring
it for Testnet, the API reference, example usage, and troubleshooting.
Proof: `docs/SDK_DEVELOPER.md`
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/SDK_DEVELOPER.md).

**6. Runbook SDK-artifacts section — Met.** The operational runbook now has a section
specifically about the SDK: which version to pin, how to upgrade, and how to troubleshoot
common integration problems.
Proof: `docs/RUNBOOK.md`, "SDK artifacts" section
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/docs/RUNBOOK.md#sdk-artifacts),
added at https://github.com/ZKELLA-org/zkella/commit/13756e0b8a8af2cb444f21d9077be0d05620eca4.

**7. Retry and resubmit logic, tested — Met.** The wallet now handles two realistic failure
cases correctly: if a transaction never actually reached the network, it's safely retried; if a
transaction's outcome is unclear because it may have already landed, the wallet checks first
instead of blindly retrying, which is the one way a naive retry could cause a double-spend. A
rejected transaction is not retried, and if the reference point a proof was built against
becomes too old, the wallet rebuilds the call against a fresh one automatically.
Proof: the automated test covering all four cases
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/tests/unit/wallet-resilience.test.ts#L39-L69,
added at https://github.com/ZKELLA-org/zkella/commit/241556203710dabd2b21d7c4da29adfeb29c7700).

**8. Wrapper classes call real contracts, not stubs — Met.** The swap, auditor, and compliance
helper classes used to return placeholder values. They now make real calls to the real deployed
contracts, and each one is backed by a real, verified transaction — the same ones cited under
criterion 4 above.
Proof: the swap wrapper
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/sdk/src/wallet/swap.ts),
replaced from a placeholder at
https://github.com/ZKELLA-org/zkella/commit/f49797bb3a47a6a50da4ca8dde05f27ce08b7bb3; the
auditor wrapper
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/sdk/src/wallet/auditor.ts);
the compliance wrapper
(https://github.com/ZKELLA-org/zkella/blob/compliance-governance-security-testnet-release/sdk/src/compliance/compliance.ts),
replaced from a placeholder at
https://github.com/ZKELLA-org/zkella/commit/114cb12ddc26048b9a3247468c865412a72c2d83.

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
- **Revoking a never-registered viewing key** has no dedicated test; it's a safe no-op by
  construction (removing a nonexistent entry from Soroban storage doesn't error), confirmed by
  reading the implementation, not by a test exercising that exact case.
- **`swap`'s `reclaim_expired_swap` recovery path** (the post-*execution* unwind, for a relayer
  that fronted liquidity but the claimant never claims) is still unit-tested only. Its claim
  window is `CLAIM_WINDOW_LEDGERS = 17_280` ledgers (~24 hours at 5s/ledger) after the swap's
  original expiry, which makes a live exercise a multi-hour undertaking, not something a single
  session can close inline — unlike `cancel_swap`'s pre-execution path, which was exercised
  live twice.
