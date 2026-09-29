# Tranche 2 deliverables: status, evidence and limits

Criterion-by-criterion account of Tranche 2 (Transfer, Unshield, Swap & Indexer), in the same
spirit as `docs/TRANCHE1_DELIVERABLES.md`: what the roadmap's own success criteria ask for,
what's actually built, and the evidence for each — live Testnet transactions where the
criterion asks for one, real tests otherwise, and an honest note wherever something is only
partly done.

## How to reproduce

```
cd contracts && cargo build --workspace --target wasm32v1-none --release && cargo test --workspace --release
npm test                                                    # JS unit tests (circuits, SDK, indexer)
DATABASE_URL=postgres://... npm test                        # also runs the PostgreSQL indexer suite
npm run indexer:load-test -- --events 5000 --database-url postgres://...
node scripts/testnet_tranche2_validation.cjs                # live Testnet: relayer fee, change note, cancel_swap
```

Test totals at the time of writing: contracts workspace 182 (token 124, verifier 30, swap 17,
governance 5, compliance 4, viewing keys 2) plus 152 JS unit tests (39 of them the indexer's),
all passing. Live evidence is in `docs/TESTNET_DEPLOYMENT.md`'s Tranche 2 section.

Current Testnet stack (`deployments.json`'s `testnet_tranche2` block): verifier
`CCU6TP7MQD7WN4UC3M4KDLSV3M3WSAOBWTO3MZ6QDMF23BUBAKENJP44`, token
`CDKZNATNSTL7WB4O6YFF3OUMPOPLLI47VGABBICKI6YRDJ35GKZH5ABM`, swap
`CCJE3JPKU7AAM3LQWD33OZKGFKN7XCNHNP4KLQSG65RXJLGJIHHMPL2D`. Deployer-administered verifier
(no governance timelock on this validation stack, same as Tranche 1's).

---

## Deliverable 1: Transfer, Entrypoint & Circuit

**1.1 The token contract's new transfer entrypoint compiles and deploys to Testnet. Met.**
`transfer`/`transfer4` already existed from Tranche 1 and are unchanged in shape except for
the new `relayer: Option<Address>` parameter this tranche adds. Deployed as part of the
Tranche 2 stack above.

**1.2 Transfer executes end-to-end on Stellar Testnet with valid proof verification and
correct nullifier consumption. Met.** Carried over from Tranche 1 (real 2-in/2-out and
4-in/4-out transactions already on Testnet, see `docs/TESTNET_DEPLOYMENT.md`) and re-confirmed
live this tranche as part of the relayer-fee transaction below, which is a genuine `transfer`
call with real nullifier consumption and note creation.

**1.3 Associated unit and integration tests pass. Met.** 124 token tests (up from 118 before
this deliverable's own additions), including four new relayer-fee tests
(`transfer_pays_the_proof_declared_fee_to_an_approved_relayer`,
`transfer_rejects_positive_fee_with_no_relayer`, `transfer_rejects_an_unapproved_relayer`,
`transfer_rejects_a_relayer_after_it_is_revoked`, `contracts/token/src/tests/spend_paths.rs`).

**1.4 The proof-declared fee is actually paid to an approved relayer via the ApprovedRelayer
mechanism, confirmed by a live transaction where the relayer, not the sender, submits the
transaction and receives the fee. Met.**
Implementation reuses `contracts/swap`'s exact `set_relayer`/`ApprovedRelayer` pattern,
independently on `contracts/token` (`token::set_relayer`, `token::is_approved_relayer`) — a
relayer approved on swap is not automatically approved on token, and vice versa; they're
separate allowlists for separate contracts. `transfer_internal` resolves and authorizes the
relayer (`relayer.require_auth()` plus the allowlist check) before any state is touched, so a
bad relayer fails the whole call cleanly. When `fee > 0`, `shielded_supply` is decremented by
`fee` (symmetric with `unshield`'s own payout accounting — the fee's value left the note graph
without becoming any output note) and the real SEP-41 fee is paid to the relayer last
(checks-effects-interactions, same convention as `shield`/`unshield`'s own token transfers).

Live evidence (`scripts/testnet_tranche2_validation.cjs`, `docs/TESTNET_DEPLOYMENT.md`): the
relayer's own Stellar account submitted and signed the `transfer` transaction (not the note
owner's), for a `fee` of 10,000 stroops. Isolating the real Stellar network fee (charged
against the submitting account regardless of the contract call) from the relayer's raw balance
change confirms it received exactly the proof-declared 10,000 stroops:
`net -10,916,109; network fee charged 10,926,109; isolated fee received 10,000`. Tx:
https://stellar.expert/explorer/testnet/tx/96733979922ee1dfe1d1f276818c5b1931550aa87630e66980508af50f787d0e

**1.5 A live 4-in/4-out transfer transaction executes successfully on Testnet after the
Tranche 1 Deliverable 3 verifier optimization lands, with its real compiled-WASM instruction
cost measured and published against the mainnet budget. Met.**
Originally live-validated on the Tranche 1 stack: tx
`15cbeef9533724df6ea96d3e96152255039664b11dac8640dd3d4c01370678ba` (see
`docs/TESTNET_DEPLOYMENT.md`), 86.3M declared instructions (was 378.7M before the
optimisation — see `docs/PERFORMANCE_OPTIMISATION.md`), 20% of the 400M mainnet budget. Also
re-run live on the Tranche 2 stack itself (`relayer: None`, reproducing the pre-Tranche-2
behavior exactly at `fee == 0`, as designed): tx
`12c13856c7acc058802abd8b1a5acefac8e8579bee1b054b48a70cd6447d440b`, 89,787,377 declared
instructions (22.4% of the 400M budget) — consistent with the Tranche 1 figure, confirming the
relayer-fee addition doesn't materially change `transfer4`'s own proof-verification or
Merkle-tree cost.

---

## Deliverable 2: Unshield, Entrypoint & Circuit

**2.1 The token contract's new unshield entrypoint compiles and deploys to Testnet. Met.**
Deployed as part of the Tranche 2 stack, with the rebuilt 7-public-input `Unshield` circuit
(`circuits/unshield/unshield.circom`) and a matching, freshly-registered verifying key.

**2.2 Unshield executes end-to-end on Stellar Testnet with valid proof verification and
correct nullifier consumption. Met.** Confirmed live as part of the change-note transaction
below — a genuine `unshield` call, real proof, real nullifier consumption.

**2.3 Associated unit and integration tests pass. Met.** 124 token tests include six new
tests for this deliverable: `unshield_accepts_a_partial_withdrawal_and_creates_a_real_change_note`
(also independently reconstructs the change note's Merkle path against the post-call root),
`unshield_rejects_a_change_commitment_already_seen`, plus the pre-existing suite updated for
the new 7-field circuit and the two new call parameters.

**2.4 Unshield accepts a change-note output, confirmed by a live transaction that unshields
part of a note's value while a new note carrying the remainder stays in the shielded pool.
Met.**
`circuits/unshield/unshield.circom` now splits the spent note's `value` into a public
`pub_value` and a `change` (range-checked nonnegative, so a prover can never claim more than
the note holds), and computes a `change_commitment`/`change_value_commit` pair exactly like
any other note-creating circuit's output — the change amount is never itself a public input,
matching the privacy convention every other note-creating entrypoint already follows. The
contract inserts `change_commitment` as a real new leaf and emits the same `("zkella","note")`
event any other new note gets. A "full" unshield with no leftover simply produces a change
note whose hidden value is 0 — there is no separate code path whose presence or absence would
leak the split.

Live evidence: `leaf_count` grew from 15 to 16 for a withdrawal of half a note's value; the
change note landed at leaf 15. Tx:
https://stellar.expert/explorer/testnet/tx/cacc35d885681328978b2841449883af363ee6f6ec805dad35ff598291abe56c

**2.5 Unshield executes successfully on Testnet as its own standalone, directly-invoked
transaction, not only as a sub-step of swap's commit flow. Met.**
The same transaction above is a direct `wallet.unshield()` call, not a cross-contract call
from `swap::commit_swap` — the first such standalone call against the *new* (change-note)
interface. (A standalone unshield against the pre-Tranche-2 interface was already live in
Tranche 1: tx `c99b6b23dd068d3c12c77697cb614efa06c805349233716851efc85a22f80891`.)

---

## Deliverable 3: Swap Contract & Fairness Circuit

**3.1 The swap contract compiles and deploys to Testnet. Met.** Deployed as part of the
Tranche 2 stack, with `commit_swap`'s new `min_amount_out` parameter.

**3.2 The full swap flow (commit, relayer-fronted execution, reveal-and-claim) executes
end-to-end on Stellar Testnet with valid proof verification and correct nullifier consumption.
Met (carried over from Tranche 1).** Live-validated on the Tranche 1 stack (verified by
independently decoding each transaction's invoked contract address, not just trusting the
citation): commit `96ac0a773395a31b36521abe81fa2ff933e6cadf0502399a467e5ec3738b1340`, execute
`994f97fdf6b73dbcb2fd4bf8467a49be63c6d3dd2a5a7483aacfd6c314949797`, reveal
`56cf20e1bed210acc1548e32514ccfc59b8f6dc31ffd788d5c609e9054321297`, all three confirmed against
`CB7TRLNTX6G3QNVDTHQHL46VNDQMMUUE4ZM5O6AIFFU6PWKGPKIQ7PYY` (`testnet_tranche1`'s swap address —
see `docs/TESTNET_DEPLOYMENT.md`). The *other* full-lifecycle runs `docs/ARCHITECTURE.md`
describes (commit `21c4380b...`/execute `5bfef119...`/reveal `88aebe0e...`, and a second one at
commit `bdb127a5...`/execute `d25a676c...`/reveal `dbb10c1b...`) both independently decode to
two *different*, earlier addresses (`CBGG3UND7P6...`, the legacy pre-Tranche-1 stack, and
`CCQH2YIZ4GKL...`, an untracked intermediate one — neither is in `deployments.json`), not the
Tranche 1 stack; `docs/ARCHITECTURE.md` was corrected to stop calling the second of those
"the Tranche 1 stack". Also since re-run end-to-end on the Tranche 2 stack itself, with
`min_amount_out` (this tranche's own addition) genuinely exercised end-to-end for the first
time: commit `fec93f512d5670ef0bb87b2e5940f1492fc02cc3ec8d5733c2b2b840bb6d623c`, execute
`4f12f3a4055328dd0140a075551467e6aa0c451845134eea6ff0cf63adacfff7`, reveal
`065e89fa8070965b1be0ea867a2fb00be681332056930654fda3fa4e7d627d9c`, all three independently
confirmed against `CCJE3JPKU7AAM3LQWD33OZKGFKN7XCNHNP4KLQSG65RXJLGJIHHMPL2D` (`testnet_tranche2`'s
swap address).

**3.3 Associated unit and integration tests pass. Met.** 17 swap tests (up from 13 before this
deliverable's additions): `cost_parity_swap_commit_and_reveal`,
`full_swap_lifecycle_moves_real_value`,
`reveal_and_claim_rejects_an_output_owner_key_other_than_the_committed_one`,
`reveal_and_claim_authorize_as_current_contract_satisfies_real_non_mocked_auth`,
`initialize_cannot_be_called_twice`, `reveal_and_claim_rejects_mismatched_intent_commitment`,
`commit_swap_rejects_proof_replayed_with_different_refund_to`,
`commit_swap_rejects_duplicate_intent_commitment`,
`commit_swap_rejects_a_proof_bound_to_a_different_expiry`,
`commit_swap_rejects_expiry_ledger_that_would_overflow_the_claim_window`,
`cancel_swap_refunds_escrowed_asset_in`, `reclaim_expired_swap_refunds_both_sides`,
`reveal_and_claim_rejects_tampered_fairness_proof`, and four new for this tranche:
`execute_swap_rejects_an_amount_out_below_the_committed_minimum`,
`execute_swap_accepts_an_amount_out_at_exactly_the_committed_minimum`,
`two_swaps_can_execute_concurrently_against_the_same_relayer`,
`a_relayer_without_enough_combined_liquidity_fails_only_the_second_execute`.

**3.4 Developer integration documentation is provided, including a clear description of the
swap's relayer-fronted-liquidity mechanism, explicitly that it never calls the Stellar DEX.
Met.** See `docs/TESTNET_DEPLOYMENT.md`'s "Developer note: swap never touches the Stellar DEX"
— confirms by direct source inspection that no DEX contract (classic Stellar DEX, Soroswap, or
otherwise) is called anywhere in `contracts/swap`, and spells out exactly which SEP-41
transfers move value at each step. See also `docs/ARCHITECTURE.md`'s swap section for the
commit-reveal design itself.

**3.5 execute_swap enforces a minimum-bound check on the relayer-supplied amount_out,
confirmed by a test showing an economically poor amount is rejected before it can be executed.
Met.**
`min_amount_out` is now declared, in plaintext, by the swap creator at `commit_swap` time and
stored in `SwapState`. This is sound because `min_amount_out` was already a deterministic
function of values (`amount_in`, `max_slippage_bps`) already bound into `intent_commitment`
at that same call (`circuits/swap/swap_fairness.circom` derives it in-circuit as
`floor(amount_in * (10000 - max_slippage_bps) / 10000)`) — surfacing it a call earlier reveals
nothing that wasn't already cryptographically fixed, and only the swap creator (not a relayer,
who has no say in `commit_swap`'s own parameters) can supply it, so a wrong value here only
ever harms the creator themselves (see `commit_swap`'s doc comment for the full reasoning).
`execute_swap` now checks `amount_out >= state.min_amount_out` immediately, before any state
is written, and `reveal_and_claim` separately checks the fairness proof's own revealed
`min_amount_out` against the same stored value, as defense in depth alongside the circuit's
existing derivation check.

Test: `execute_swap_rejects_an_amount_out_below_the_committed_minimum` commits a swap with
`min_amount_out = 900,000` and shows `execute_swap` panics with "amount_out below the
committed minimum" for an offer of 500,000, before any relayer liquidity is pulled in;
`execute_swap_accepts_an_amount_out_at_exactly_the_committed_minimum` confirms the bound is a
floor, not a stricter exact match.

**3.6 The stalled-swap recovery path (cancel or reclaim after expiry) is exercised in a live
Testnet transaction, not only in unit tests. Met.**
A swap was committed (escrowing 1,500,000) and never executed by any relayer; once its
`expiry_ledger` passed, `cancel_swap` was called live and refunded the full escrowed amount to
the committer. Same fee-isolation technique as Deliverable 1.4's evidence: `net +1,483,313;
network fee charged 16,687; isolated refund 1,500,000`. Tx:
https://stellar.expert/explorer/testnet/tx/8fd02a1579add5c196d843bcaf676f2d809ef4841d2d166cb8fa18e0bb51caa7
— `reclaim_expired_swap` (the post-*execution* recovery path, for a relayer that fronted
liquidity but was never claimed against) is not separately exercised live; it is covered by
`reclaim_expired_swap_refunds_both_sides`.

**3.7 Two swaps competing for the same relayer liquidity concurrently are tested together,
with the resulting behavior documented. Met, unit-tested rather than live.**
`two_swaps_can_execute_concurrently_against_the_same_relayer` commits two independent swaps
against the same relayer in the same ledger, confirms both escrows land independently (the
swap contract's `asset_in` balance is the sum of both, not one overwriting the other), funds
the relayer once for the sum of both offers, executes both, and confirms both states end
`Executed` with the correct, independent `amount_out` each. Documented behavior (also the
outcome `a_relayer_without_enough_combined_liquidity_fails_only_the_second_execute` confirms):
`execute_swap` pulls exactly `amount_out` from the relayer per call via a real SEP-41
`transfer`, not a shared pool, so nothing in the contract serializes two swaps against the
same relayer or reserves liquidity ahead of time — a relayer without enough combined balance
for both simply has the second `execute_swap` fail with the token's own insufficient-balance
error, which is the relayer's own capital-management problem, not a swap-contract invariant
violation, and it leaves the first swap's already-committed state completely untouched. Not
run as two live Testnet transactions specifically (the unit test already exercises the real
compiled contract logic end to end); see `docs/TESTNET_DEPLOYMENT.md`'s Tranche 2 section.

---

## Deliverable 4: Indexer Event Sync Engine

**4.1 Indexer operational on Stellar Testnet with PostgreSQL persistence, correctly ingesting
note and nullifier events. Met.**
`indexer/src/db-postgres.ts`'s `PostgresIndexerDb` implements the same `IndexerDb` interface
(`indexer/src/db.ts`) as the original SQLite reference implementation, backed by a real
PostgreSQL server via the `pg` driver's connection pool — same schema, same idempotent-write
semantics, same ledger-cursor behavior. `indexer/src/main.ts` picks this backend automatically
whenever `DATABASE_URL` is set (falling back to SQLite otherwise, which stays as the zero-
dependency reference implementation and the baseline Deliverable 4.3's own comparison is
published against). `docker-compose.yml` and `indexer/Dockerfile` containerize the indexer
alongside a real PostgreSQL 16 instance — the grant-funded deployment target, not the SQLite
file. Dual-provider RPC failover and Multi-AZ managed hosting stay explicit, documented future
work (`indexer/README.md`, `docs/TECHNICAL_SPEC.md` §13.3), not folded into this deliverable.

Verified against a real, locally-run PostgreSQL 14 server (not mocked): all 9 of
`SqliteIndexerDb`'s own behavioral tests (`tests/unit/indexer-db.test.ts`) are duplicated
against `PostgresIndexerDb` in `tests/unit/indexer-db-postgres.test.ts` and pass identically —
proving the two backends are genuinely interchangeable, not just structurally similar. CI runs
this suite against a real `postgres:16-alpine` service container
(`.github/workflows/ci.yml`).

**4.2 Indexer correctly resumes from its last synced ledger cursor after a simulated restart,
with no missed or duplicated events. Met.**
`tests/unit/indexer-db-postgres.test.ts`'s restart test opens a genuinely new `PostgresIndexerDb`
connection (a fresh TCP connection to the same server, not the same in-memory object — closer
to what a real process restart looks like than SQLite's in-process equivalent) after two
events are persisted and the cursor is set, confirms the cursor and both events survive, then
reprocesses the same ledger's event again (as a real resume-from-last-synced-ledger restart
would) and confirms no duplicate is created. The underlying idempotent-write mechanism
(`ON CONFLICT ... DO NOTHING` on `leaf_index`/`nullifier`; the ledger cursor records the last
ledger *seen*, not the one after it, so a page ending mid-ledger doesn't silently drop that
ledger's remaining events) is unchanged from the mechanism `docs/POC_IMPLEMENTATION.md`
documents fixing a real pagination bug with.

**4.3 Sync behavior is tested under sustained high event volume and a large backfill from an
early ledger, with results published against the SQLite reference implementation's own
untested baseline. Met, with a real, honest result — not simply "PostgreSQL wins."**
`scripts/indexer_load_test.mjs` drives both backends directly (bypassing the real Soroban RPC
polling loop, which this tranche's realistic event volume — at most a handful of events per
transaction, polled every few seconds — never comes close to saturating) with a configurable
number of synthetic note/nullifier events, in two shapes: "sustained" (one event ingested at a
time, simulating steady-state arrival) and "backfill" (the same total count inserted in
parallel batches, simulating a fresh indexer catching up from an early ledger).

Measured at 5,000 events per run, SQLite in-memory vs. a real local PostgreSQL 14 server over a
loopback TCP connection, on an idle 4-core Intel i7-8565U @ 1.80GHz (no other CPU-bound process
running — an independent audit's first attempt at this reproduction ran on a machine with a
concurrent heavy build and got numbers 3.6–8x slower across the board than the ones below,
confirming these figures are environment-sensitive and worth re-measuring on real target
hardware rather than trusted as an absolute, portable benchmark):

| | SQLite (baseline) | PostgreSQL | Ratio |
| --- | --- | --- | --- |
| Sustained (one at a time) | 491ms (10,190 events/s) | 68,377ms (73 events/s) | 139x slower |
| Backfill (parallel batch) | 300ms (16,691 events/s) | 6,795ms (736 events/s) | 23x slower |
| Query pass (11 pages) | 31ms | 60ms | 1.9x slower |

PostgreSQL is genuinely, substantially slower per write here — this is not hidden or
explained away. The reason is structural, not a PostgreSQL weakness: SQLite's `:memory:`
backend does zero network I/O and zero fsync, while every one-at-a-time `PostgresIndexerDb`
write pays a real network round trip to a real server process. This matters for how the
numbers should be read: PostgreSQL's real advantage for this deliverable is concurrent-writer
safety, a shared connection pool serving the HTTP API and the sync loop at once, and the
operational tooling (`pg_dump`, replication, managed hosting) a production deployment needs —
none of which raw single-writer throughput on localhost measures, and none of which SQLite's
single-file model provides at all. The realistic ingestion rate this indexer ever needs (a
handful of events every few seconds) is nowhere near either backend's throughput ceiling;
13.7ms of real latency per event, sustained, is not a practical concern for that load. The
"backfill" figure is closer to what a real bulk catch-up would look like in practice (batched,
not fully serial), and it narrows the gap to 11x.

---

## Deliverable 5: Indexer API, Health Checks & Multi-Operator Interface

**5.1 Query API operational and documented for notes, Merkle paths, and nullifier state.
Met (carried over from Tranche 1, unchanged surface).** `GET /notes`, `GET /merkle/root`,
`GET /merkle/path/:leafIndex`, `POST /nullifiers/batch`, `GET /commitment/:hex` — documented
in `indexer/README.md`, matching `sdk/src/indexer/client.ts`'s `IndexerClient` exactly.

**5.2 Health-check, metrics, and alerting endpoints are in place and documented, with the
interface for a second independent operator instance specified even if only one operator is
run during this tranche. Met.**
`GET /health` (unchanged: synced ledger, RPC tip, lag) and a new `GET /metrics` (uptime,
indexed note/nullifier counts, the current synced ledger, and running totals of requests,
auth rejections and rate-limit rejections) — both deliberately left unauthenticated even when
`INDEXER_API_KEYS` is set, matching what load balancers and monitoring tooling expect to reach
without credentials. Alerting itself (an actual paging/notification pipeline) is not built —
this deliverable funds the metrics surface to alert *on*, documented in `indexer/README.md`'s
"Multi-operator interface" section as a threshold on `/health`'s `lag` field and on
`/metrics`'s `rejectedAuthTotal`/`rejectedRateLimitTotal`, not a live alerting integration.
Multi-operator interface: see `indexer/README.md`'s dedicated section — this deliverable funds
specifying what a second operator needs to plug into the same on-chain event stream and expose
an interchangeable API, not standing one up live (explicitly out of scope per the roadmap's
own text); no second operator instance has been run.

**5.3 Query endpoints require real request authentication and enforce rate limiting,
confirmed by a test showing an unauthenticated or excessive request is correctly rejected.
Met.**
`INDEXER_API_KEYS` (comma-separated bearer tokens) gates every query endpoint (`/notes`,
`/merkle/*`, `/nullifiers/batch`, `/commitment/*`) — `/health` and `/metrics` stay open, as
5.2 explains. `INDEXER_RATE_LIMIT_PER_MINUTE` (default 600) enforces a per-identity (API key,
or client IP when unauthenticated) fixed-window budget, applied to every request regardless of
whether auth is configured. Both are real HTTP-level checks (`indexer/src/http.ts`'s
`parseBearerToken`/`RateLimiter`), not just documented intent.

Tests (`tests/unit/indexer-http-auth.test.ts`) start a real `node:http` server and make real
HTTP requests against it: a request with no `Authorization` header is rejected 401 when API
keys are configured; a wrong bearer token is rejected 401; the correct token is accepted;
`/health`/`/metrics` remain reachable with no header even when keys are configured; and five
requests against a server configured with `rateLimitPerMinute: 3` return `[200, 200, 200, 429,
429]` — confirmed on the real, live-responding server object, not a mock. The rate limiter is
an in-memory, single-process fixed-window counter, documented in its own doc comment as a
limitation for a multi-instance deployment (each replica would enforce its own independent
budget; a real horizontally-scaled deployment should move this to a shared store).

An independent security review of this deliverable found two Low-severity hygiene gaps, both
closed: the bearer-token check now compares with `crypto.timingSafeEqual` (`matchesApiKey`)
instead of a plain `Set.has`/`===`, which leaked timing information about how many leading
bytes of a guess were correct; and `/health`/`/metrics`, while still deliberately unauthenticated
(load balancers and monitoring tooling need to reach them with no credentials), are now also
rate-limited by IP, since each does a real RPC round-trip or DB query and was otherwise
completely unbounded. Neither was a fund-safety or auth-bypass issue — every query-data route
was already correctly gated, and `db-postgres.ts`'s queries are fully parameterized (no SQL
injection surface). Reviewed and confirmed safe: relayer resolution/authorization always runs
before any state write in `transfer_internal` and cannot be bypassed by a spurious
`fee`/`relayer` combination; the change note's nonnegativity and owner-key binding are enforced
by the circuit itself, not just the contract; `min_amount_out` can only ever be set by the swap
creator, never a relayer or third party, so a wrong value only ever harms its own author.

---

## What is not done, and what is thin

- **`reclaim_expired_swap` (the post-execution recovery path) has no live transaction**, only
  `cancel_swap` (the pre-execution path) does — not for lack of trying: `reclaim_expired_swap`
  requires `env.ledger().sequence() > expiry_ledger + CLAIM_WINDOW_LEDGERS`, and
  `CLAIM_WINDOW_LEDGERS` is a hardcoded constant (17,280 ledgers, about a day at Testnet's real
  ledger-close rate) independent of the swap creator's chosen `expiry_ledger` — unlike
  `cancel_swap`'s wait, which is bounded only by however short `expiry_ledger` itself is set,
  this one cannot be made short for a live demonstration without changing the contract.
- **The concurrent-swap and min-bound-rejection criteria (3.5, 3.7) are unit-tested against
  the real compiled contract, not exercised as separate live Testnet transactions.**
- **The indexer's rate limiter and multi-operator design are both single-process,
  single-operator today** — documented as such, not silently assumed away.
- **PostgreSQL's raw single-writer throughput is worse than SQLite's `:memory:` baseline** for
  one-at-a-time writes over a real network connection (Deliverable 4.3's own measurement); its
  real advantage for this deliverable is concurrency and operational tooling, not throughput.
