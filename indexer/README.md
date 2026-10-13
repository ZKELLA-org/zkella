# ZKELLA Indexer

Persists `token`'s `("zkella","note")` and `("zkella","nf")` contract events
past Stellar RPC's own short retention window, and serves them over HTTP in
exactly the shape `sdk/src/indexer/client.ts`'s `IndexerClient` expects.
`merkle_root`/`merkle_path` are proxied live to `token` itself rather than
duplicated — the contract is already the source of truth for current tree
state.

## Running

### Containerized (PostgreSQL — the grant-funded stack)

```sh
TOKEN_CONTRACT_ID=C... INDEXER_START_LEDGER=<token's deploy ledger> \
docker compose up --build
```

Runs the indexer alongside a real PostgreSQL 16 instance (`docker-compose.yml`), with the
indexed history in the `pgdata` named volume (`docker compose down -v` to also drop it).
`INDEXER_API_KEYS`/`INDEXER_RATE_LIMIT_PER_MINUTE` (see below) can be passed as env vars to
the compose invocation.

### Local development (SQLite, no external dependency)

```sh
TOKEN_CONTRACT_ID=C...       \
SOROBAN_RPC_URL=https://soroban-testnet.stellar.org \
ZKELLA_NETWORK=testnet       \
INDEXER_START_LEDGER=<token's deploy ledger> \
npm run indexer
```

Runs directly via `node --experimental-strip-types`, no build step required. Uses Node's
built-in `node:sqlite` (experimental, Node 22.5+) when `DATABASE_URL` is unset — see
`src/db.ts`'s doc comment for how the two backends relate; both implement the same
`IndexerDb` interface, so application code (`src/sync.ts`, `src/http.ts`) doesn't care which
one is running.

Env vars: `DATABASE_URL` (a `postgres://...` connection string — when set, runs against
PostgreSQL instead of SQLite), `INDEXER_DB_PATH` (SQLite file path when `DATABASE_URL` is
unset, default `./indexer.db`), `INDEXER_HTTP_PORT` (default `8787`), `INDEXER_POLL_MS`
(default `5000`), `INDEXER_API_KEYS` (comma-separated bearer tokens required on query
endpoints — unset disables auth, fine for local development, not for a public deployment),
`INDEXER_RATE_LIMIT_PER_MINUTE` (default `600`).

### Load testing

```sh
node --experimental-strip-types scripts/indexer_load_test.mjs --events 20000 --database-url postgres://...
```

Drives both backends directly with synthetic events under sustained and backfill load, and
prints SQLite vs. PostgreSQL results side by side — see
`docs/TRANCHE2_DELIVERABLES.md`'s Deliverable 4.3 for a worked example and an honest reading
of what the numbers do and don't mean.

## HTTP API

| Endpoint | Auth? | Description |
| --- | --- | --- |
| `GET /health` | No | `{ syncedLedger, tipLedger, lag }` |
| `GET /metrics` | No | `{ uptimeSeconds, syncedLedger, indexedNotes, indexedNullifiers, requestsTotal, rejectedAuthTotal, rejectedRateLimitTotal }` |
| `GET /notes?from_ledger=&limit=` | Yes | Paginated note history |
| `GET /merkle/root` | Yes | Proxies `token.merkle_root()`/`leaf_count()` |
| `GET /merkle/path/:leafIndex` | Yes | Proxies `token.merkle_path()` |
| `POST /nullifiers/batch` | Yes | `{ nullifiers: string[] }` → `{ spent: Record<string, boolean> }` |
| `GET /commitment/:hex` | Yes | Leaf index for a known commitment, 404 otherwise |

"Auth?" means gated by `INDEXER_API_KEYS`/`INDEXER_RATE_LIMIT_PER_MINUTE` when set (see
`src/http.ts`'s doc comment) — `/health` and `/metrics` stay reachable unauthenticated even
when keys are configured, matching what load balancers and monitoring tooling expect.

## Multi-operator interface

Only one operator instance has ever actually been run — this section specifies the interface
a second, independent operator would need to implement to serve the same state
interchangeably, per Tranche 2 Deliverable 5; it is not itself a second running instance.

**What makes two operators interchangeable:**
1. **Same event source, same processing rules.** Both poll the identical Soroban RPC
   `getEvents` filter (`token`'s `("zkella","note")`/`("zkella","nf")` topics) and apply the
   identical, deterministic transform (`src/sync.ts`'s `syncOnce`) — given the same on-chain
   history, two independent operators produce byte-identical `notes`/`nullifiers` tables. There
   is no operator-specific state or configuration that affects what gets stored.
2. **Idempotent, replay-safe writes.** `upsertNote`/`markNullifierSpent` are both
   `ON CONFLICT ... DO NOTHING` — an operator can safely re-poll or restart from an earlier
   cursor without an operator-specific reconciliation step.
3. **The same `IndexerDb` interface (`src/db.ts`), any conforming backend.** A second operator
   isn't required to run PostgreSQL specifically — anything implementing `IndexerDb`
   (`getLastSyncedLedger`, `upsertNote`, `getNotesFrom`, ...) is a valid second operator, which
   is also why this interface is documented rather than only implemented.
4. **The same HTTP contract.** `sdk/src/indexer/client.ts`'s `IndexerClient` is written against
   the endpoint shapes in the table above; any operator serving those same shapes is a drop-in
   replacement for a wallet client, including one running its own auth/rate-limit
   configuration independently of any other operator's.

**Trust model: wallets don't have to trust any single operator.** `merkle_root`/`merkle_path`
are proxied live from the contract itself (not from the operator's own database), so a wallet
can independently verify a returned Merkle path against the contract's real current root
regardless of which operator served it. For `notes`/`nullifiers` — history the contract itself
doesn't serve past Stellar RPC's retention window — a wallet that queries two independent
operators and gets divergent results has direct evidence one of them is either lagging or
misbehaving; this document does not build that reconciliation client, but the identical schema
and deterministic processing above is what would make it possible.

**What alerting on `/metrics`/`/health` looks like**, since this deliverable funds the surface
to alert on rather than a live paging pipeline: monitor `/health`'s `lag` (ledgers behind the
RPC tip — a real indexer is expected to stay within a few ledgers under normal load) and
`/metrics`'s `rejectedAuthTotal`/`rejectedRateLimitTotal` (a sudden spike in either suggests a
misconfigured client or an attempted abuse pattern worth paging on), wired into whatever
alerting stack (PagerDuty, Opsgenie, a Prometheus Alertmanager rule) the deployment already
uses — not built here.

## Status

Validated against live Stellar Testnet: a real `shield()` transaction's `("zkella","note")`
event was correctly synced, persisted, and served back through every HTTP endpoint, including
`merkle_root`/`merkle_path` proxying to the real deployed `token` contract
(`docs/POC_IMPLEMENTATION.md`). The PostgreSQL backend (Tranche 2 Deliverable 4) is verified
against a real, locally-run PostgreSQL server — all of `SqliteIndexerDb`'s own tests are
duplicated against `PostgresIndexerDb` and pass identically
(`tests/unit/indexer-db-postgres.test.ts`), and CI runs that suite against a real
`postgres:16-alpine` service container. Not yet covered: an actual second operator instance
running concurrently, and a live alerting pipeline wired to `/metrics` (see
"Multi-operator interface" above for what's specified vs. what's actually run). Dual-provider
RPC failover and Multi-AZ managed hosting are described in `docs/TECHNICAL_SPEC.md` §13.3 —
planned, mainnet-stage work, not part of this tranche. An operational runbook covering this
service alongside the rest of the stack exists at `docs/RUNBOOK.md`, exercised in two drills,
including a real indexer outage.
