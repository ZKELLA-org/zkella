# ZKELLA — Operational Runbook and Incident Response

This is the operational runbook referenced throughout `docs/ARCHITECTURE.md`, `README.md`, and the roadmap. It exists to make deployment, monitoring, key handling, and incident response concrete rather than aspirational.

**Status of this document itself:** written against the current Testnet deployment (single team, single indexer operator, Stellar Testnet only), and exercised in two drills — one with injected faults, one a real indexer outage with a real alert delivered and confirmed by a person (see "Drill record" below). Treat it as a working operational baseline, proven once at small scale, not yet a mature process proven at production scale — see "Known limitations" at the end.

---

## 1. System components at a glance

Every admin-gated action below refers to the real contract entrypoints in this repository, not a hypothetical interface. The current stack is `testnet_final` in `deployments.json`, built from the current source and deployed by `scripts/testnet_deploy_stack.sh`:

| Component | Address | Admin model | Has `pause()` |
|---|---|---|---|
| `token` | `CA5TFEVODC25SSEZII2XHB2XMCKFNXNLXRNFTKWPKMT5PCWYZUMLPRUZ` | admin = `governance` contract address | **yes**, via `governance.pause_token()` |
| `verifier` | `CC2LQPXH3L5YKRP7YJ6UIC57AOGJXBQN4DEKNRU4Y32ABXJZOENCDAX3` | admin = `governance` contract address (cross-call auth) | **yes**, via `governance.pause_verifier()` |
| `governance` | `CDTJLTBEKBXRJJKHVI32A5UMBB4UC6VBMDOF7WR43H2SKCCRAVRJWY5Q` | single admin key (`zkella-testnet-deployer`), two-step transfer, plus a separate guardian key (`zkella-testnet-guardian`) that can only cancel a queued update; VK updates and token-admin actions are additionally timelocked — production value is 7 days (`VK_TIMELOCK_LEDGERS = 120_960`), **but this instance was built with the `testnet-fast-timelock` feature, so `timelock_ledgers()` returns 60 (about 5 minutes) — never assume a 5-minute timelock for a production deployment** | **yes** |
| `compliance` | `CDP5SRSUFDVEYHUCUX53SM4PZVTOIHDZR3Z5C7G4TFFKAQSLX64FOZVJ` | single admin key (`zkella-testnet-deployer`) | **yes** |
| `swap` | `CBN7JJEPAEA5NCKOECPPGHETAK4CCCUFOBUJCDZ7K7HPIV7Y6ILOC524` | single admin key (`zkella-testnet-deployer`); per-relayer allowlist via `set_relayer` | **yes** |
| `viewing_keys` | `CDT776JLXU5GWRIY6WXLZGVKZ5V4TG32HAITNPFZVX5UCJSMMFHNMEEE` | no admin — `register`/`revoke` each require the calling owner's own authorization | no (nothing to halt: a holder can only touch their own entry) |
| `indexer/` | self-hosted, `http://localhost:8787` by default | single process, single SQLite file | n/a (`/health` endpoint) |

Earlier Testnet stacks (`testnet_tranche1`, `testnet_tranche2`, and the legacy `testnet` entry in `deployments.json`) predate pause entirely — none of their contracts can be halted. They are not source-equivalent to current code and should not be used as a reference for what the current contracts can do.

**All five contracts with state can be paused today**, admin-gated in every case (`pause()`/`unpause()` each call `require_auth()` on that contract's own admin — see the table above for who that is per contract). Each pause is scoped to the calls that change state or decide something:

- **`token.pause()`** stops `shield()`, `shield_batch()`, `transfer()`, `transfer4()` and `unshield()` directly, and *indirectly* blocks `swap.commit_swap()` and `swap.reveal_and_claim()` (both cross-call into `token`'s `unshield`/`shield`, which check `assert_not_paused()` on entry). `token`'s admin is `governance`'s own contract address, so this is only reachable by calling `governance.pause_token()` — not `token.pause()` directly, which would fail its `admin.require_auth()`.
- **`verifier.pause()`** stops `register_verifying_key()`, `update_verifying_key()`, `verify()` and `verify_batch()` (the last two fail closed: a paused verifier rejects every proof rather than skip checking it). It does **not** block `revoke_previous_vk()` or any read — a pause caused by a bad key must not also block the one call that lets you kill that key's retention window. Same reachability note as `token`: call `governance.pause_verifier()`, not `verifier.pause()` directly.
- **`governance.pause()`** stops `queue_vk_update()`, `execute_vk_update()`, `queue_token_action()`, `execute_token_action()`, `transfer_admin()` and `accept_admin()`. It does **not** block `cancel_vk_update()`, `guardian_cancel_vk_update()`, `guardian_cancel_token_action()` or `revoke_previous_vk()` — cancelling a bad queued update, or the guardian's cancel-only path, must keep working precisely when a pause is in force.
- **`compliance.pause()`** stops `publish_compliance_proof()`. It does not block reads (`get_compliance_proof`, `sanctions_root`) or `set_sanctions_root()`.
- **`swap.pause()`** stops `commit_swap()`, `execute_swap()`, `reveal_and_claim()` and `set_relayer()`. It does **not** stop `cancel_swap()` or `reclaim_expired_swap()` (the refund paths) — those must stay callable under a pause so escrowed funds are never stuck.

---

## 2. Monitoring

No automated alerting exists yet (see "Known limitations") — this section defines what to check and how, for manual or soon-to-be-automated monitoring.

### RPC health

- `curl https://soroban-testnet.stellar.org` (or the configured `SOROBAN_RPC_URL`) reachable and returning current ledger info via `getLatestLedger`.
- Watch for elevated latency or error rates on `getEvents`/`getTransaction`/`simulateTransaction` calls — the indexer's sync loop and every SDK proof submission depend on these.

### Indexer health

- `GET {INDEXER_HTTP_PORT:-8787}/health` — the indexer's own liveness endpoint (`indexer/src/http.ts`).
- **Sync lag**: compare the indexer's most recently persisted leaf index against `token`'s real `leaf_count()` (read via `stellar contract invoke --id <token> -- leaf_count` or the SDK). A growing gap means the sync loop (`indexer/src/sync.ts`, polling on `INDEXER_POLL_MS`, default 5000ms) has stalled or is falling behind — check process logs first, RPC health second.
- **Data correctness spot-check**: pick a recent leaf index, fetch its commitment from the indexer, and confirm it matches the same leaf read directly from `token.merkle_path(leaf_index)` — the indexer proxies `merkle_root`/`merkle_path` live rather than caching them, so a mismatch here points at the note/nullifier sync path specifically, not tree state.

### Contract state

- `token.merkle_root()`, `token.leaf_count()`, `token.shielded_supply(asset)` — read via `stellar contract invoke` or the SDK's view calls. A `shielded_supply` that doesn't reconcile with the sum of real `shield()` deposits minus `unshield()` withdrawals for that asset is a signal worth investigating immediately (see Incident Category 1).
- `swap`'s per-`swap_id` `SwapState` (status, expiry_ledger) for any swap that's been `Committed` or `Executed` for an unusually long time without progressing — a candidate for `cancel_swap`/`reclaim_expired_swap` once its window passes (see §4).

### Transaction failure patterns

Watch simulation/submission failures for these specific error shapes, each pointing at a different root cause:

- `HostError: Error(Budget, ExceededLimit)` — instruction budget exhaustion. Compare against the measured baseline (~76.4M/400M for `shield()`, `contracts/token/src/lib.rs`'s `shield_fits_within_mainnet_instruction_budget` test) — a large deviation suggests a regression, not normal variance.
- `Error(Contract, #5)` on `token` (`Error::InvalidAnchor`) — the caller's proof anchor fell outside the 32-root history window (`contracts/token/src/merkle.rs`'s `ROOT_HISTORY_SIZE`). Expected occasionally under concurrent load; a sudden spike means proofs are being generated much slower than the tree is advancing, or the window needs re-tuning. The window counts calls that inserted leaves: a `transfer`, `transfer4` or `shield_batch` adds one root history entry however many leaves it inserts (only its final root is recorded).
- `Error(Auth, InvalidAction)` on any cross-contract call — a missing `authorize_as_current_contract` entry (see the real incident this exact error caused in `docs/POC_IMPLEMENTATION.md`'s swap audit). Treat as a code-level bug, not an operational issue, unless it appears on a code path that was previously working.
- `Error(Contract, #3)` on `verifier` (`Error::VkAlreadyRegistered`) — an attempted `register_verifying_key` for a circuit that already has one; use `governance.queue_vk_update`/`execute_vk_update` instead (see §3).

---

## 3. Key management

### Admin key rotation (token, governance, compliance, swap)

`token` and `governance` implement a two-step transfer — `transfer_admin(new_admin)` (current admin proposes) then `accept_admin()` (new admin, from their own address, confirms). This prevents an admin key rotation from bricking the contract by transferring to an unreachable or mistyped address. `compliance` and `swap` currently use a single-step admin model — rotate with direct care, there is no confirmation step to catch a mistake.

Procedure:
1. Generate the new admin keypair out of band (hardware wallet or equivalent — never generate or transmit a production admin key through this runbook's own tooling).
2. `transfer_admin(new_admin)` from the current admin key.
3. `accept_admin()` from the new admin key (for `token`/`governance` only).
4. Confirm via `stellar contract invoke -- admin` (or equivalent read) that the new address is live before decommissioning the old key.

### Verifying-key rotation (soundness-critical)

`verifier.update_verifying_key()` requires `verifier`'s admin, which is `governance`'s own contract address — so a VK rotation is always initiated through `governance`, never by calling `verifier` directly:

1. `governance.queue_vk_update(circuit, new_vk)` — starts the timelock: 7 days (`VK_TIMELOCK_LEDGERS = 120_960` ledgers at ~5s/ledger) for a real production build. **The `governance` instance currently live on Testnet (§1's table) uses the `testnet-fast-timelock` build instead — ~5 minutes — so check which binary is actually deployed before relying on either number operationally.**
2. Wait for `eta` (the queued ledger sequence) to pass. This window exists specifically so users can exit before an untrusted or malicious VK takes effect — do not shorten it operationally even under incident pressure; if a VK is actively being exploited, `verifier.pause()` (which fails every `verify()`/`verify_batch()` closed) or `token.pause()` is the correct immediate lever, not rushing a VK swap.
3. `governance.execute_vk_update(circuit)` — cross-calls `verifier.update_verifying_key()`.
4. If the update should be aborted before `eta`, `governance.cancel_vk_update(circuit)`.

**Retention window and revocation.** When `verifier.update_verifying_key` replaces a key, the outgoing key stays acceptable for `VK_RETENTION_WINDOW_LEDGERS = 17_280` ledgers (about one day) so proofs generated against it but not yet submitted still verify. If the outgoing key is compromised or known to be unsound, the verifier admin calls `verifier.revoke_previous_vk(circuit)` to end the window immediately (the verifier's admin is the `governance` contract, so use `governance.revoke_previous_vk(circuit)`, which forwards the call; `governance.timelock_ledgers()` returns the timelock the deployed binary was built with; on a deployer-administered verifier the deployer calls it directly). Expiry arithmetic saturates. Do not rely on the window to protect users from an unsound old key: revoke it in the same step as the rotation.

### Deployment: initialize immediately

`initialize` on every contract can be called by anyone between deploy and the first initialize call, so it can be front-run. Deploy and initialize in the same step (same script, back to back), then confirm the admin and the stored verifier/token addresses are the intended values before using the contract. If the admin is wrong, redeploy.

### Merkle sibling TTL

Sibling nodes read during an insert are not TTL-bumped. After roughly a year with no inserts, an insert can fail until the expired persistent entry is restored. Restoring an archived entry is permissionless (any account can submit the restore footprint), so this is a liveness risk, not a loss of funds. If shield/transfer starts failing with an archived-entry error after a long idle period, restore the entries named in the simulation footprint and retry.

### Note format and shield limits

- Notes are owner-key notes: `cm = H(H(H(value, asset), H(rho, rcm)), pk)` with `pk = H(nk, DOMAIN_PK)`, `DOMAIN_PK = int("zkella_pk") = 2258241487740017274987`. A note can only be spent with the `nk` that derives its `pk`. `shield` takes the recipient's `owner_pk` explicitly, and `ShieldBatchItem` carries an `owner_pk` after `rcm`.
- `shield_batch` accepts at most `MAX_SHIELD_BATCH = 8` items (measured about 314M instructions for 8, 79% of the 400M limit, about 34M per item after a shared tree-hashing cost of about 42M; the live 8-item transaction declared 335M). The build fails if the constant is raised past 85% of the limit (`shield_batch_size_sweep_on_real_wasm`).
- Shielding an asset requires it to be approved: `set_asset_approved(asset, approved)` / `is_asset_approved(asset)`. `set_min_shield_amount` / `min_shield_amount` set the spam floor (default 1,000 base units).
- Token errors added: `AssetNotApproved = 18`, `BatchLengthMismatch = 19`, `EmptyBatch = 20`, `BatchTooLarge = 21`. Verifier error `NonCanonicalInput = 9`: a public input at or above the BN254 scalar modulus was submitted (asset fields are reduced mod r before use as inputs). The token also rejects a negative transfer fee.

### Relayer key management (swap)

`swap.set_relayer(relayer_address, approved: bool)`, admin-gated. To revoke a compromised or misbehaving relayer: `set_relayer(relayer, false)` immediately — this only blocks *future* `execute_swap` calls from that address; it does not affect swaps that address already executed (those still need `reveal_and_claim` or the unwind paths in §4 to resolve).

### Indexer operational keypair

The indexer uses one Stellar keypair internally for read-only simulation calls against `token` (see `docs/POC_IMPLEMENTATION.md`'s account on the indexer's own address-generation bug fix). This key holds no funds and no admin privilege — rotating it is just restarting the process with a freshly generated keypair, no on-chain action required.

---

## 4. Incident response

### Category 1 — Contract or proof-verification failure

**Symptoms:** budget-exceeded errors on previously-working calls, `shielded_supply` not reconciling, unexpected proof-verification failures at a rate inconsistent with normal user error.

1. Determine whether the failure is circuit-side (VK mismatch — check `verifier.get_verifying_key(circuit)` against the expected artifact hash) or budget-side (compare instruction cost against the measured baselines in `docs/CIRCUIT_SPEC.md` §8 and `docs/POC_IMPLEMENTATION.md`).
2. If the issue is exploitable (a proof that shouldn't verify is being accepted, or vice versa for legitimate proofs): `verifier.pause()` stops `verify()`/`verify_batch()` directly and fails closed, which is the most targeted lever for a verification bug specifically. `token.pause()` additionally stops new proofs from being submitted at all (`shield`/`transfer`/`transfer4`/`unshield`). Neither stops `swap.execute_swap()`, `cancel_swap()` or `reclaim_expired_swap()` (plain transfers, no verifier/token call) — pause `swap` itself too if the exploit reaches those paths.
3. Preserve the failing transaction hash(es), the exact `pub_inputs` and proof bytes submitted, and simulation output before anything is retried or the contract state changes further.
4. If a VK fix is needed, follow §3's verifying-key rotation procedure — there is no fast path around the 7-day timelock by design.
5. Root-cause before `unpause()`. Do not unpause on a timer; unpause when the specific failure mode is understood and, if code changed, redeployed and tested.

### Category 2 — Indexer outage or data inconsistency

**Symptoms:** `/health` failing, sync lag growing, a served note/path not matching on-chain state.

1. Check process status and logs first; the sync loop is a single Node process today (no supervisor/restart-on-crash configured by default — add one, e.g. systemd or a process manager, before relying on this in anything beyond a demo).
2. If the process is alive but stuck: restart it. `INDEXER_START_LEDGER` only matters for a fresh database; a restart against an existing `INDEXER_DB_PATH` resumes from the last persisted cursor, not from scratch.
3. If the SQLite file itself is suspected corrupt: there is no secondary indexer instance to fail over to today (see "Known limitations") — the recovery path is re-syncing from `INDEXER_START_LEDGER` (or from genesis of the currently-deployed `token` instance) into a fresh database file, which is safe but not instant, since `getEvents` retention limits how far back a single query can reach — chunk the backfill accordingly.
4. Regardless of cause: `merkle_root`/`merkle_path` are proxied live to `token`, not cached, so wallets performing those specific reads are unaffected by an indexer outage — only note/nullifier history recovery is impacted. Communicate that distinction; it materially changes user impact.

### Category 3 — Key or secret exposure

**Symptoms:** suspected leak of an admin key, relayer key, or indexer operational key.

1. **Admin key (any contract):** follow §3's admin rotation procedure immediately. Until rotation completes, treat every admin-gated function on that contract as potentially attacker-controlled — on every contract this now includes that contract's own `pause()`/`unpause()` (see §1), plus `swap.set_relayer()` and, on `governance`, `queue_vk_update()`/`queue_token_action()`. A compromised admin *cannot* forge proofs or steal shielded funds directly (that requires breaking Groth16/BN254, not the admin key), but *can* pause a contract or queue a malicious `governance` update — the latter is blocked by the 7-day timelock, and the separate guardian key (not the compromised admin key) can cancel the queued update during that window via `guardian_cancel_vk_update`/`guardian_cancel_token_action`, which is exactly the scenario that guardian role exists for.
2. **Relayer key:** `set_relayer(compromised_relayer, false)` immediately. Audit any `swap`s that relayer executed but weren't yet claimed — they may need to go through `reclaim_expired_swap` once their claim window passes rather than a legitimate `reveal_and_claim`.
3. **Indexer key:** no funds or privilege at risk (see §3) — rotate by restart, no urgency beyond routine hygiene.
4. In every case: preserve evidence of the exposure (how it was discovered, what if anything was accessed) before rotating, if it's safe to take the time to do so — rotation is more urgent than forensics, but don't discard the latter unnecessarily.

### Category 4 — Misconfigured verifier or admin flow

**Symptoms:** a deployment or configuration step left a contract pointed at the wrong dependency (e.g. `token` initialized with the wrong `verifier` address, `swap` initialized with the wrong `token` address) — most likely right after a redeployment.

1. Halt new operations: pause the misconfigured contract directly (`token.pause()`, `verifier.pause()`, `compliance.pause()` or `swap.pause()` — see §1 for exactly what each one stops), plus `token.pause()` for anything that cross-calls into it. `governance.pause()` additionally stops VK and token-admin actions from being queued or executed while the misconfiguration is being diagnosed.
2. Read back every cross-contract address each contract actually stores (`token`'s configured verifier, `swap`'s configured token/verifier, `compliance`'s configured verifier) and diff against the intended `deployments.json` set.
3. Soroban contracts can't have their constructor-set addresses changed in place — a genuine misconfiguration at `initialize()` time means redeploying the affected contract(s), not patching state. Follow the same redeployment + re-wiring process documented in `docs/POC_IMPLEMENTATION.md`'s "senior audit, contract-stack redeployment" update, and update `deployments.json`/`docs/TESTNET_DEPLOYMENT.md` immediately afterward so they don't go stale.
4. Any swap or note state stranded in a superseded contract instance follows the same recovery path already documented for the superseded `swap` instance in `docs/TESTNET_DEPLOYMENT.md` ("Swap redeployment") — `reclaim_expired_swap` once the window passes; there is no equivalent unwind for `token`, which is why getting `token`'s configuration right the first time matters more than any other single deployment step.

---

### Category 5 — Issuer clawback of a custodied asset

**Symptoms:** `token.custody_shortfall(asset)` returns a positive value; unshield payouts for that asset are lower than the withdrawn amount.

1. Confirm: compare `shielded_supply(asset)` with the token contract's balance of `asset` (the difference is `custody_shortfall`).
2. Stop new deposits: `set_asset_approved(asset, false)`. Existing notes stay withdrawable; nothing is trapped.
3. Do not pause unless withdrawals themselves are at risk. Withdrawals are already paid pro rata (`value * balance / supply`), so the loss is shared by all holders and early withdrawers cannot drain the pool.
4. `contracts/swap::commit_swap` will refuse to escrow this asset while a shortfall exists (the escrow would be short); this is expected.
5. Communicate the loss ratio (`balance / supply`) to holders and file the issuer action for follow-up. Only re-approve the asset once the issuer's clawback authority is resolved.

## 5. Minimum operating checklist

- [ ] RPC health check (manual or automated) at a cadence matched to how quickly a stall would be noticed by users — no automated schedule exists yet, see "Known limitations."
- [ ] Indexer `/health` + sync-lag check against `token.leaf_count()`.
- [ ] Weekly reconciliation: `shielded_supply(asset)` against the running sum of real shield/unshield events for that asset, per asset currently wrapped.
- [ ] Every contract invocation and state-changing event logged somewhere durable outside the ledger itself (today: whatever the operator's own `stellar contract invoke`/SDK client logs — no centralized log aggregation exists yet).
- [ ] Documented, current owner for each component (today: the same small team for all of them — see "Known limitations").
- [ ] Periodic backup of `indexer.db` (SQLite file) and the `deployments.json`/`docs/TESTNET_DEPLOYMENT.md` address record.
- [ ] Weekly review of any Testnet incidents, near-misses, or anomalies, however minor.

---

## Known limitations

This runbook describes a real but early operational posture, not a mature one. Specifically:

- **Single admin key per contract**, not multi-sig, except for `governance`'s VK-update timelock. A single admin key compromise is a real, unmitigated risk for every contract except the specific VK-rotation path — see Category 3 above for exactly what is and isn't exposed by that.
- **Alerting exists but is minimal.** The scheduled health check (below) posts a plain-text failure message to a single `ntfy.sh` channel — real and running, not a placeholder — but there is no paging, escalation, or on-call rotation behind it; a dropped notification has no backstop beyond `LOG_FILE`.
- **No indexer failover.** One process, one SQLite file, no secondary instance, and a single RPC provider for event ingestion. `docs/ARCHITECTURE.md` and `docs/POC_IMPLEMENTATION.md` describe multi-operator indexing as target architecture; `docs/TECHNICAL_SPEC.md` §13.3 sets out the planned production design (dual-provider RPC failover, managed Postgres with Multi-AZ, a second operator in a different region or cloud provider) — none of it is built yet.
- **A compromised admin key is still a real, largely unmitigated risk**, except for `governance`'s own VK/token-admin timelock (where a separate guardian key can cancel a malicious queued update without needing the admin key at all). None of the five contracts use a multisig admin — see Category 3 for what each contract's pause does and does not protect against.
- **Exercised twice, not battle-tested.** This document has been run through two drills (see "Drill record" below), one of them a real incident, not simulated. That is still a small sample — treat the procedures above as validated once at small scale, not a playbook proven across many incidents or at production scale.

## Testnet stack health check

`scripts/testnet_health_check.sh` checks the Soroban RPC, the indexer (when `INDEXER_URL` is set), and the deployed contract state recorded under `testnet_final` in `deployments.json`: governance's timelock value and the token's approval of the native asset. Any failure is appended to `LOG_FILE` and posted to `NOTIFY_WEBHOOK` if that is set; the script exits non-zero.

Scheduled with cron, every 15 minutes, on the operating host for the current stack:

```
*/15 * * * * cd /path/to/zkella && NOTIFY_WEBHOOK=... INDEXER_URL=... scripts/testnet_health_check.sh
```

This is live today, not a suggested setup — it posts to a real `ntfy.sh` channel on failure (see the second drill below for a real alert delivered through it) and appends every failure to `LOG_FILE`. No notification destination is committed to this repository, since `NOTIFY_WEBHOOK` is operator-specific; set your own before relying on alerts for a different deployment.

## Drill record

Run on 2026-10-05 against the Testnet stack in `deployments.json` (`testnet_final`). Three faults were injected into `scripts/testnet_health_check.sh`, one at a time:

| Fault | Expected | Observed |
| --- | --- | --- |
| Soroban RPC unreachable | RPC check fails, exit 1 | `FAIL rpc status='unreachable'`, exit 1 |
| Indexer unreachable (`INDEXER_URL`) | Indexer lag check fails, exit 1 | `FAIL indexer lag='unreachable'`, exit 1 |
| Deployment record points governance at a non-contract address | Governance check fails and names the address, exit 1 | `FAIL governance <address> timelock_ledgers unreadable`, exit 1 |

Findings:

- The third fault exposed a diagnostic gap: the governance failure did not name the address it tried. The messages now include the contract address for governance and token.
- All three faults reached the failure path and the script exited non-zero. Delivery to a webhook was verified separately against a local receiver.

### Second drill: a real fault, with a person, after the 2026-10-08 redeploy

Run on 2026-10-08 against the current `testnet_final` stack, after the audit-fix redeploy. Unlike
the first drill (which injected faults into the health check's own inputs), this one caused a
genuine Category 2 incident — the indexer process was actually stopped, not simulated — and a
person watched the real alert arrive and confirmed the response, not just a script's exit code.

1. The indexer was stopped (`SIGTERM`). The scheduled check (the same command the cron job
   runs) was run against the live, genuinely-down indexer, which failed:
   `FAIL indexer lag='unreachable' (limit 100)`, exit 1. The failure POSTed a real notification to
   the team's `ntfy.sh` channel, which a person subscribed to and confirmed receiving before any
   further step was taken — the drill's success condition from the roadmap ("including a case
   surfaced by that automated check, rather than only a manual read of the checklist") is met by
   this run specifically, not by the first drill's manual fault injection.
2. `RUNBOOK.md` Category 2's own first two steps were followed for real: process status checked
   (confirmed down), then restarted against the existing `INDEXER_DB_PATH`. The restart resumed
   from ledger 5090457 (the persisted cursor), not from `INDEXER_START_LEDGER` — confirmed
   directly against the database's `sync_state` table, not inferred from how fast it caught up.
3. Category 2's claim that `merkle_root`/`merkle_path` stay available during an indexer outage
   was checked directly while the indexer was still down: `token.merkle_root()` read correctly
   straight from the contract.
4. A second health-check run after the restart passed clean.

Findings:

- **Real diagnostic gap, fixed:** the indexer's own startup log always printed
  `syncing ... from ledger <INDEXER_START_LEDGER>`, even when a persisted cursor meant it was
  about to resume from a much later ledger — during a real incident this reads as "about to
  re-sync from scratch," which it isn't. `indexer/src/main.ts` now reads the actual resume
  point before logging and says so explicitly when it differs from the configured start.
  The first drill's fix (naming the contract address) was a diagnostic gap in the health check
  itself; this one was a diagnostic gap in the indexer being checked.
- **The webhook payload format was wrong for the configured destination:** `testnet_health_check.sh`
  POSTed `{"text": ...}` as a raw body, which matches Slack's incoming-webhook format but is not
  parsed by `ntfy.sh` — a subscriber would have seen the literal JSON string, braces included,
  instead of a readable message. Found and fixed before the drill's real alert was sent, by
  testing the exact webhook call against the exact configured destination rather than assuming
  the existing format was provider-agnostic.
- The scheduled check itself (`*/15 * * * *` via cron, `scripts/testnet_health_check.sh`) is
  now actually installed and running on the operating host, not just documented as possible.

## Decisions

- **Failure notifications.** The scheduled health check (cron, every 15 minutes) posts to a
  `ntfy.sh` channel on failure, and every failure is also written to `LOG_FILE`. This is a
  real, currently-running destination, not a placeholder — see the second drill above for a
  real alert delivered through it. A JSON-expecting provider (e.g. Slack) needs the `curl` call
  in `testnet_health_check.sh`'s `fail()` changed back to wrap the message as `{"text": ...}`.
- **Expected governance timelock.** The health check fails when the timelock differs from `EXPECTED_TIMELOCK` (default 60, the Testnet demo build). A production deployment sets it to 120960 (7 days at 5 seconds per ledger), so a demo build reaching production is caught.
- **Drill cadence.** The health check runs every 15 minutes. A full drill, with people following the four incident categories, runs quarterly and after any contract redeployment — the second drill above is the first instance of that commitment being kept, run the same day as the 2026-10-08 redeploy.
- **Indexer on mainnet.** The indexer refuses to start on `ZKELLA_NETWORK=mainnet` without `INDEXER_API_KEYS`, so query endpoints cannot run unauthenticated in production.

## SDK artifacts

Applies to `@zkella/sdk` (see `docs/SDK_RELEASE.md`).

**Version pinning.** Pin the exact version in client projects (`"@zkella/sdk": "0.1.0"`, not a range). The SDK's proofs depend on the circuit artifacts and verifying keys deployed on the Testnet stack in `deployments.json`, so an unplanned upgrade can change which proofs verify.

**Upgrade guidance.** Upgrade only when the release notes name the deployed stack the new version targets. Before upgrading, compare `TESTNET_CONTRACTS` in the new version with the addresses your application uses, and run the `testnet-config` test in this repository against them.

**Troubleshooting client-side integrations:**

| Symptom | Likely cause | Action |
| --- | --- | --- |
| `Error(Contract, #5)` on transfer or unshield | Anchor aged out of the root-history window | The wallet rebuilds the call against the current root. If it repeats, check the RPC is not lagging behind the indexer. |
| `Error(Contract, #4)` on any proof call | Proof does not verify under the deployed key | The circuit artifacts in the client do not match the deployed verifying key. Check the client's artifact versions against `deployments.json`. |
| `UnknownSanctionsRoot` on compliance publish | The client's sanctions list differs from the root the admin published | Obtain the current list from the maintainer; do not change the root locally. |
| `paused` panic in a swap call | The swap contract is paused | Check the pause status with the contract admin. Cancel and reclaim stay callable. |
| `fetch failed`, `ECONNRESET`, `503` on a contract call | Transient RPC or network failure | The wallet retries three times with backoff. If it still fails, check `scripts/testnet_health_check.sh` output for the RPC. |
| Balance or notes look wrong after a restart | Viewing-key epoch or sync cursor not persisted | Persist `wallet.currentEpoch` and the last synced ledger, and pass the epoch back as `viewingEpoch`. |
| `npm audit` reports findings in a fresh install | Transitive dependencies (see `docs/SDK_RELEASE.md`) | Not reachable from the SDK's library use. Track the planned replacement of `circomlibjs` and `snarkjs`. |
