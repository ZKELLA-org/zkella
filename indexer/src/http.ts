// HTTP API matching `sdk/src/indexer/client.ts`'s `IndexerClient` exactly —
// that file is the contract this server implements.
//
// Auth and rate limiting (Tranche 2 Deliverable 5): every query endpoint
// (everything except `/health` and `/metrics`, which ops tooling and load
// balancers expect to reach unauthenticated — see docs/RUNBOOK.md) requires
// `Authorization: Bearer <key>` against `INDEXER_API_KEYS` when that env var
// is set, and every request — authenticated or not — is subject to a
// per-identity (API key, or client IP for unauthenticated deployments)
// rate limit. Both are real, enforced checks, not just documentation: see
// `tests/unit/indexer-http-limit.test.ts`.
//
// The rate limiter is an in-memory fixed-window counter — correct for a
// single process, but each replica in a multi-instance deployment would
// enforce its own independent budget rather than sharing one; a real
// horizontally-scaled deployment should move this to a shared store (e.g.
// Redis) instead. Documented here rather than silently assumed away.

import { createServer, IncomingMessage, ServerResponse } from 'node:http'
import { timingSafeEqual } from 'node:crypto'
import { rpc, Contract, Account, Keypair, TransactionBuilder, Networks, nativeToScVal, scValToNative } from '@stellar/stellar-sdk'
import type { IndexerDb } from './db.ts'

// A read-only simulation needs *some* syntactically valid source account —
// it never signs or submits anything, so any keypair works, funded or not.
// Generated once at module load rather than hardcoded: a hand-typed StrKey
// is easy to get subtly wrong (this one was, the first time — wrong
// checksum bytes — caught by the indexer's own live-Testnet smoke test).
const SIMULATION_KEYPAIR = Keypair.random()

const DEFAULT_NOTES_LIMIT = 500
const MAX_NOTES_LIMIT     = 1000
const DEFAULT_RATE_LIMIT_PER_MINUTE = 600

export interface HttpConfig {
  db:          IndexerDb
  tokenAddress: string
  rpcUrl:      string
  network:     'testnet' | 'mainnet'
  port:        number
  startLedger: number
  /** Bearer tokens accepted on query endpoints. Empty/unset disables auth entirely. */
  apiKeys?:    string[]
  /** Requests per rolling 60s window, per API key (or per IP when unauthenticated). */
  rateLimitPerMinute?: number
}

/**
 * Clamps the `/notes` `?limit=` query param to `[1, MAX_NOTES_LIMIT]`,
 * falling back to `DEFAULT_NOTES_LIMIT` for anything unparseable. Exported
 * and pulled out of the request handler specifically so this validation
 * logic — the actual fix for a real unbounded-query vector (SQLite treats a
 * *negative* `LIMIT` as "unlimited", so an unvalidated value here wasn't
 * just wasteful for huge inputs, it could return the entire table) — has a
 * direct, fast unit test instead of only being exercisable through a live
 * HTTP request.
 */
export function parseNotesLimit(raw: string | null): number {
  const rawLimit = Number(raw ?? String(DEFAULT_NOTES_LIMIT))
  if (!Number.isFinite(rawLimit)) return DEFAULT_NOTES_LIMIT
  return Math.min(Math.max(Math.trunc(rawLimit), 1), MAX_NOTES_LIMIT)
}

/**
 * Extracts the bearer token from an `Authorization` header, or `null` if
 * absent/malformed. Exported for direct unit testing.
 */
export function parseBearerToken(header: string | undefined): string | null {
  if (!header) return null
  const match = /^Bearer\s+(.+)$/.exec(header.trim())
  return match ? match[1] : null
}

/**
 * True if `token` matches any of `apiKeys`, comparing each with
 * `timingSafeEqual` rather than `Set.has`/`===` — a plain string comparison
 * returns as soon as the first differing byte is found, so its timing
 * leaks how many leading bytes of a guess were correct. Low severity (an
 * attacker needs a very precise, repeatable timing channel against a
 * random high-entropy key), fixed anyway since it costs nothing here: a
 * handful of short strings compared per request, not a hot loop.
 */
export function matchesApiKey(token: string, apiKeys: Set<string> | string[]): boolean {
  const tokenBuf = Buffer.from(token)
  for (const key of apiKeys) {
    const keyBuf = Buffer.from(key)
    // timingSafeEqual throws on a length mismatch instead of returning
    // false, and comparing lengths first is itself not a meaningful leak
    // (key lengths aren't secret).
    if (keyBuf.length === tokenBuf.length && timingSafeEqual(keyBuf, tokenBuf)) return true
  }
  return false
}

/**
 * A fixed-window request counter, one window per identity. `windowMs` and
 * `limit` are fixed at construction; `hit(id)` returns `true` (allowed) or
 * `false` (over budget for the current window). Exported so it has a direct
 * unit test independent of a live HTTP server.
 */
export class RateLimiter {
  private windowMs: number
  private limit: number
  private counts = new Map<string, { windowStart: number; count: number }>()
  // Last time `sweep` ran, so it only does its O(n) pass at most once per
  // window instead of on every call. Starts at 0 (not `Date.now()`) so the
  // very first `hit()` - whenever it happens - triggers one, rather than
  // depending on how much wall-clock time elapsed since construction.
  private lastSweep = 0

  constructor(limitPerWindow: number, windowMs = 60_000) {
    this.limit = limitPerWindow
    this.windowMs = windowMs
  }

  /** Current number of identities being tracked. Exported for direct testing of `sweep`. */
  get size(): number {
    return this.counts.size
  }

  hit(id: string, now = Date.now()): boolean {
    this.sweep(now)
    const entry = this.counts.get(id)
    if (!entry || now - entry.windowStart >= this.windowMs) {
      this.counts.set(id, { windowStart: now, count: 1 })
      return true
    }
    entry.count += 1
    return entry.count <= this.limit
  }

  /**
   * Evicts windows that have already expired. Without this, `counts` grows
   * by one entry per distinct identity ever seen, for the lifetime of the
   * process, and never shrinks - a slow, unbounded memory leak. `/health`
   * and `/metrics` are rate-limited by client IP even when API keys are
   * configured for everything else (see this file's doc comment), so a
   * long-running deployment fielding traffic from many distinct source IPs
   * - a botnet deliberately rotating IPs to inflate this, or just ordinary
   * monitoring/load-balancer traffic over weeks - would otherwise hold one
   * entry per IP forever. Runs at most once per `windowMs`, not on every
   * call, so the amortized cost stays O(1) per `hit`.
   */
  private sweep(now: number): void {
    if (now - this.lastSweep < this.windowMs) return
    this.lastSweep = now
    for (const [id, entry] of this.counts) {
      if (now - entry.windowStart >= this.windowMs) this.counts.delete(id)
    }
  }
}

function sendJson(res: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body)
  res.writeHead(status, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) })
  res.end(payload)
}

async function readJsonBody(req: IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = []
  for await (const chunk of req) chunks.push(chunk as Buffer)
  const raw = Buffer.concat(chunks).toString('utf8')
  return raw.length > 0 ? JSON.parse(raw) : {}
}

/**
 * Proxies a `token` view call directly rather than maintaining a redundant
 * second copy of the Merkle tree — the contract is already the source of
 * truth for current tree state; the indexer's own database only needs to
 * cover what the contract *can't* serve itself (historical note/nullifier
 * events past Stellar RPC's retention window). Returns the RPC node's
 * `latestLedger` alongside the decoded value so callers that issue more than
 * one of these in a row can check whether the node's view of the chain moved
 * between calls — see `callConsistentViews`.
 */
async function simulateView(
  config: HttpConfig, method: string, args: ReturnType<typeof nativeToScVal>[],
): Promise<{ value: unknown; latestLedger: number }> {
  const server = new rpc.Server(config.rpcUrl)
  // A read-only simulation doesn't need a real funded account — any valid
  // account ID works as the simulation's nominal source.
  const dummyAccount = new Account(SIMULATION_KEYPAIR.publicKey(), '0')
  const tx = new TransactionBuilder(dummyAccount, {
    fee: '100',
    networkPassphrase: config.network === 'mainnet' ? Networks.PUBLIC : Networks.TESTNET,
  })
    .addOperation(new Contract(config.tokenAddress).call(method, ...args))
    .setTimeout(10)
    .build()

  const sim = await server.simulateTransaction(tx)
  if (rpc.Api.isSimulationError(sim)) {
    throw new Error(`${method} simulation error: ${sim.error}`)
  }
  const success = sim as rpc.Api.SimulateTransactionSuccessResponse
  return { value: scValToNative(success.result!.retval), latestLedger: success.latestLedger }
}

/**
 * Runs several view calls and retries (up to `MAX_ATTEMPTS` times) until they
 * all land on the same `latestLedger`. Each call is its own independent
 * `simulateTransaction` against whatever the RPC node's current tip happens
 * to be when it runs, so e.g. `merkle_path` followed by `merkle_root` can
 * straddle a ledger close that inserts a new leaf — returning a path from
 * the old tree alongside a root from the new one. That pair doesn't
 * reconstruct to each other, so a wallet building a proof from it gets a
 * path that fails to verify against the root it was handed, even though
 * both values were individually read correctly. Calls run concurrently
 * (`Promise.all`) to keep the window small, and are retried as a batch
 * rather than call-by-call since partial consistency isn't useful here.
 */
async function callConsistentViews(
  config: HttpConfig,
  calls: Array<{ method: string; args: ReturnType<typeof nativeToScVal>[] }>,
): Promise<unknown[]> {
  const MAX_ATTEMPTS = 5
  for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
    const results = await Promise.all(calls.map(c => simulateView(config, c.method, c.args)))
    if (results.every(r => r.latestLedger === results[0].latestLedger)) return results.map(r => r.value)
    if (attempt === MAX_ATTEMPTS) {
      throw new Error('could not read a ledger-consistent snapshot for merkle view calls')
    }
  }
  // Unreachable — the loop above always either returns or throws on its last
  // attempt — but keeps TypeScript happy about the function's return type.
  throw new Error('unreachable')
}

export function startHttpServer(config: HttpConfig): ReturnType<typeof createServer> {
  const apiKeys = new Set(config.apiKeys ?? [])
  const limiter = new RateLimiter(config.rateLimitPerMinute ?? DEFAULT_RATE_LIMIT_PER_MINUTE)
  let requestCount = 0
  let rejectedAuthCount = 0
  let rejectedRateLimitCount = 0
  const startedAt = Date.now()

  const server = createServer(async (req, res) => {
    try {
      const url = new URL(req.url ?? '/', 'http://localhost')
      requestCount += 1

      // /health and /metrics stay unauthenticated by design (load balancers
      // and monitoring tooling expect to reach them with no credentials —
      // see this file's own doc comment), but they're still rate-limited by
      // IP: each does a real RPC round-trip or DB query, so leaving them
      // completely unbounded is a small, easy-to-close DoS surface.
      if (req.method === 'GET' && (url.pathname === '/health' || url.pathname === '/metrics')) {
        const ip = req.socket.remoteAddress ?? 'unknown'
        if (!limiter.hit(ip)) {
          rejectedRateLimitCount += 1
          sendJson(res, 429, { error: 'rate limit exceeded' })
          return
        }
      }

      if (req.method === 'GET' && url.pathname === '/health') {
        const synced = await config.db.getLastSyncedLedger(config.startLedger)
        const server = new rpc.Server(config.rpcUrl)
        const tip = (await server.getLatestLedger()).sequence
        sendJson(res, 200, { syncedLedger: synced, tipLedger: tip, lag: Math.max(0, tip - synced) })
        return
      }

      if (req.method === 'GET' && url.pathname === '/metrics') {
        const rows = await config.db.countRows()
        const synced = await config.db.getLastSyncedLedger(config.startLedger)
        sendJson(res, 200, {
          uptimeSeconds:     Math.floor((Date.now() - startedAt) / 1000),
          syncedLedger:      synced,
          indexedNotes:      rows.notes,
          indexedNullifiers: rows.nullifiers,
          requestsTotal:     requestCount,
          rejectedAuthTotal: rejectedAuthCount,
          rejectedRateLimitTotal: rejectedRateLimitCount,
        })
        return
      }

      // Every endpoint below this point is a query endpoint: authenticated
      // (when `apiKeys` is non-empty) and rate-limited (always).
      const token = parseBearerToken(req.headers.authorization)
      if (apiKeys.size > 0 && (token === null || !matchesApiKey(token, apiKeys))) {
        rejectedAuthCount += 1
        sendJson(res, 401, { error: 'unauthorized: missing or invalid bearer token' })
        return
      }
      const identity = token ?? (req.socket.remoteAddress ?? 'unknown')
      if (!limiter.hit(identity)) {
        rejectedRateLimitCount += 1
        sendJson(res, 429, { error: 'rate limit exceeded' })
        return
      }

      if (req.method === 'GET' && url.pathname === '/notes') {
        const fromLedger = Number(url.searchParams.get('from_ledger') ?? '0')
        const limit = parseNotesLimit(url.searchParams.get('limit'))
        sendJson(res, 200, await config.db.getNotesFrom(fromLedger, limit))
        return
      }

      if (req.method === 'GET' && url.pathname === '/merkle/root') {
        const [root, leafCount] = await callConsistentViews(config, [
          { method: 'merkle_root', args: [] },
          { method: 'leaf_count', args: [] },
        ])
        sendJson(res, 200, {
          root: Buffer.from(root as Uint8Array).toString('hex'),
          leafCount: Number(leafCount),
        })
        return
      }

      const merklePathMatch = url.pathname.match(/^\/merkle\/path\/(\d+)$/)
      if (req.method === 'GET' && merklePathMatch) {
        const leafIndex = Number(merklePathMatch[1])
        const [path, root] = await callConsistentViews(config, [
          { method: 'merkle_path', args: [nativeToScVal(leafIndex, { type: 'u32' })] },
          { method: 'merkle_root', args: [] },
        ]) as [Uint8Array[], Uint8Array]
        sendJson(res, 200, {
          path: path.map(p => Buffer.from(p).toString('hex')),
          index: pathIndicesFor(leafIndex, path.length),
          root: Buffer.from(root as Uint8Array).toString('hex'),
        })
        return
      }

      if (req.method === 'POST' && url.pathname === '/nullifiers/batch') {
        const body = await readJsonBody(req) as { nullifiers: string[] }
        const spent: Record<string, boolean> = {}
        for (const nf of body.nullifiers ?? []) spent[nf] = await config.db.isNullifierSpent(nf)
        sendJson(res, 200, { spent })
        return
      }

      const commitmentMatch = url.pathname.match(/^\/commitment\/([0-9a-f]+)$/)
      if (req.method === 'GET' && commitmentMatch) {
        const leafIndex = await config.db.getLeafByCommitment(commitmentMatch[1])
        if (leafIndex === null) { sendJson(res, 404, { error: 'commitment not found' }); return }
        sendJson(res, 200, { leafIndex })
        return
      }

      sendJson(res, 404, { error: 'not found' })
    } catch (err) {
      sendJson(res, 500, { error: err instanceof Error ? err.message : String(err) })
    }
  })

  server.listen(config.port, () => {
    console.log(`[indexer] HTTP API listening on :${config.port}`)
  })

  return server
}

/** Direction bits for `leafIndex`, matching `contracts/token::merkle::get_path_indices`. */
function pathIndicesFor(leafIndex: number, depth: number): number[] {
  const bits: number[] = []
  let idx = leafIndex
  for (let i = 0; i < depth; i++) {
    bits.push(idx & 1)
    idx = Math.floor(idx / 2)
  }
  return bits
}
