# ZKELLA — Operational Runbook and Incident Response

This is the operational runbook referenced as an open item throughout `docs/POC_IMPLEMENTATION.md`, `docs/ARCHITECTURE.md`, `README.md`, and `docs/SCF_REVIEWER_RESPONSE.md` §4. It exists to make deployment, monitoring, key handling, and incident response concrete rather than aspirational.

**Status of this document itself:** first version, written against the current soft-PoC deployment (single team, single indexer operator, Stellar Testnet only). It has not yet been exercised in a real incident or run through a drill. Treat it as a starting operational baseline, not a mature, battle-tested process — see "Known limitations" at the end.

---

## 1. System components at a glance

Every admin-gated action below refers to the real contract entrypoints in this repository, not a hypothetical interface. Current live addresses (Testnet, see `deployments.json` for the machine-readable, always-current version — addresses here are a point-in-time snapshot):

| Component | Address | Admin model | Has `pause()` |
|---|---|---|---|
| `token` (`ShieldedToken`) | `CACD4IA6OJQPG3AVGPQPJT3SJKP7YQQM4BIHUD7F7NG74KDJQLGIZQOQ` | single admin key, two-step transfer | **yes** |
| `verifier` | `CAD7I5VEXC6QXO6A4K3PP5GLCLY6EJZ6LXLAPDR4WILBRJFINXDGQOER` | admin = `governance` contract address (cross-call auth) | no |
| `governance` | `CCO72PR2RHEUWXWKB5D5UTHMSJOWNLNA3FELUSAFVXXGTCDGVEUQL4MS` | single admin key, two-step transfer; VK updates additionally timelocked — production value is 7 days (`VK_TIMELOCK_LEDGERS = 120_960`), **but this specific Testnet instance was built with the `testnet-fast-timelock` feature (~5 minutes) so the full timelock path could be demonstrated live — never assume a 5-minute timelock for a production deployment** | no |
| `compliance` | `CAA6GVANAT7GBWBA3CRXIL7WX4O62NEGBC6XHMPTFMEZPBHMM5PKRNOS` | single admin key | no |
| `swap` | `CBGG3UND7P6GMHCUSSYVGIOB6FUO5KK7OZVBA7LI7K4K7CJEV5T3ZRXN` | single admin key; per-relayer allowlist via `set_relayer` | no |
| `indexer/` | self-hosted, `http://localhost:8787` by default | single process, single SQLite file | n/a (`/health` endpoint) |

**Only `token` has a pause switch today.** Calling `token.pause()` stops `shield()`/`transfer()`/`transfer4()`/`unshield()` directly, and *indirectly* blocks `swap.commit_swap()` and `swap.reveal_and_claim()` (both cross-call into `token`'s `unshield`/`shield`, which check `assert_not_paused()` on entry). It does **not** stop `swap.execute_swap()` (a plain SEP-41 transfer, no call into `token`) or `swap.cancel_swap()`/`reclaim_expired_swap()` (refund paths, also plain transfers). `verifier`, `governance`, and `compliance` have no pause mechanism at all — see "Known limitations."

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

- `HostError: Error(Budget, ExceededLimit)` — instruction budget exhaustion. Compare against the measured baseline (~104M/400M for `shield()`, `contracts/token/src/lib.rs`'s `shield_fits_within_mainnet_instruction_budget` test) — a large deviation suggests a regression, not normal variance.
- `Error(Contract, #5)` on `token` (`Error::InvalidAnchor`) — the caller's proof anchor fell outside the 32-root history window (`contracts/token/src/merkle.rs`'s `ROOT_HISTORY_SIZE`). Expected occasionally under concurrent load; a sudden spike means proofs are being generated much slower than the tree is advancing, or the window needs re-tuning.
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
2. Wait for `eta` (the queued ledger sequence) to pass. This window exists specifically so users can exit before an untrusted or malicious VK takes effect — do not shorten it operationally even under incident pressure; if a VK is actively being exploited, `token.pause()` is the correct immediate lever, not rushing a VK swap.
3. `governance.execute_vk_update(circuit)` — cross-calls `verifier.update_verifying_key()`.
4. If the update should be aborted before `eta`, `governance.cancel_vk_update(circuit)`.

### Relayer key management (swap)

`swap.set_relayer(relayer_address, approved: bool)`, admin-gated. To revoke a compromised or misbehaving relayer: `set_relayer(relayer, false)` immediately — this only blocks *future* `execute_swap` calls from that address; it does not affect swaps that address already executed (those still need `reveal_and_claim` or the unwind paths in §4 to resolve).

### Indexer operational keypair

The indexer uses one Stellar keypair internally for read-only simulation calls against `token` (see `docs/POC_IMPLEMENTATION.md`'s account on the indexer's own address-generation bug fix). This key holds no funds and no admin privilege — rotating it is just restarting the process with a freshly generated keypair, no on-chain action required.

---

## 4. Incident response

### Category 1 — Contract or proof-verification failure

**Symptoms:** budget-exceeded errors on previously-working calls, `shielded_supply` not reconciling, unexpected proof-verification failures at a rate inconsistent with normal user error.

1. Determine whether the failure is circuit-side (VK mismatch — check `verifier.get_verifying_key(circuit)` against the expected artifact hash) or budget-side (compare instruction cost against the measured baselines in `docs/CIRCUIT_SPEC.md` §8 and `docs/POC_IMPLEMENTATION.md`).
2. If the issue is exploitable (a proof that shouldn't verify is being accepted, or vice versa for legitimate proofs): `token.pause()` immediately. This is the only real circuit breaker available today (see §1's pause-scope caveat — it does not stop `execute_swap`/`cancel_swap`/`reclaim_expired_swap`).
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

1. **Admin key (any contract):** follow §3's admin rotation procedure immediately. Until rotation completes, treat every admin-gated function on that contract as potentially attacker-controlled — this includes `token.pause()`/`unpause()` and `swap.set_relayer()`. A compromised `token` admin *cannot* forge proofs or steal shielded funds directly (that requires breaking Groth16/BN254, not the admin key), but *can* pause the contract, register a malicious VK via a queued `governance` update (blocked by the 7-day timelock — this is exactly the scenario the timelock exists for), or mismanage `ShieldedSupply` bookkeeping only insofar as the contract's own logic allows.
2. **Relayer key:** `set_relayer(compromised_relayer, false)` immediately. Audit any `swap`s that relayer executed but weren't yet claimed — they may need to go through `reclaim_expired_swap` once their claim window passes rather than a legitimate `reveal_and_claim`.
3. **Indexer key:** no funds or privilege at risk (see §3) — rotate by restart, no urgency beyond routine hygiene.
4. In every case: preserve evidence of the exposure (how it was discovered, what if anything was accessed) before rotating, if it's safe to take the time to do so — rotation is more urgent than forensics, but don't discard the latter unnecessarily.

### Category 4 — Misconfigured verifier or admin flow

**Symptoms:** a deployment or configuration step left a contract pointed at the wrong dependency (e.g. `token` initialized with the wrong `verifier` address, `swap` initialized with the wrong `token` address) — most likely right after a redeployment.

1. Halt new operations: `token.pause()` if the misconfiguration is on `token` or anything that depends on it (i.e. most of the stack). There is no equivalent for `swap`/`verifier`/`governance`/`compliance` — communicate the exposure window honestly rather than implying a pause exists where it doesn't.
2. Read back every cross-contract address each contract actually stores (`token`'s configured verifier, `swap`'s configured token/verifier, `compliance`'s configured verifier) and diff against the intended `deployments.json` set.
3. Soroban contracts can't have their constructor-set addresses changed in place — a genuine misconfiguration at `initialize()` time means redeploying the affected contract(s), not patching state. Follow the same redeployment + re-wiring process documented in `docs/POC_IMPLEMENTATION.md`'s "senior audit, contract-stack redeployment" update, and update `deployments.json`/`docs/TESTNET_DEPLOYMENT.md` immediately afterward so they don't go stale.
4. Any swap or note state stranded in a superseded contract instance follows the same recovery path already documented for the superseded `swap` instance in `docs/TESTNET_DEPLOYMENT.md` ("Swap redeployment") — `reclaim_expired_swap` once the window passes; there is no equivalent unwind for `token`, which is why getting `token`'s configuration right the first time matters more than any other single deployment step.

---

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
- **No automated alerting or paging.** Every check in §2 and §5 is manual today. Wiring these into an actual alerting pipeline (e.g. a monitoring service watching the endpoints and thresholds described above) is planned, not done.
- **No indexer failover.** One process, one SQLite file, no secondary instance, and a single RPC provider for event ingestion. `docs/ARCHITECTURE.md` and `docs/POC_IMPLEMENTATION.md` describe multi-operator indexing as target architecture; `docs/TECHNICAL_SPEC.md` §13.3 sets out the planned production design (dual-provider RPC failover, managed Postgres with Multi-AZ, a second operator in a different region or cloud provider) — none of it is built yet.
- **No pause mechanism on `verifier`, `governance`, `compliance`, or `swap`.** Only `token` can be halted directly; see §1 and Category 4 for the practical consequences.
- **This document is untested.** It has not been exercised in a real incident or a scheduled drill. Treat every procedure above as a first draft to be corrected by the first real use, not a proven playbook.
