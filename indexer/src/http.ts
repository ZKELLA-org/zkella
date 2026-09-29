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

  constructor(limitPerWindow: number, windowMs = 60_000) {
    this.limit = limitPerWindow
    this.windowMs = windowMs
  }

  hit(id: string, now = Date.now()): boolean {
    const entry = this.counts.get(id)
    if (!entry || now - entry.windowStart >= this.windowMs) {
      this.counts.set(id, { windowStart: now, count: 1 })
      return true
    }
    entry.count += 1
    return entry.count <= this.limit
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
 * Proxies `token.merkle_path`/`merkle_root` view calls directly rather than
 * maintaining a redundant second copy of the Merkle tree — the contract is
 * already the source of truth for current tree state; the indexer's own
 * database only needs to cover what the contract *can't* serve itself
 * (historical note/nullifier events past Stellar RPC's retention window).
 */
async function callView(config: HttpConfig, method: string, args: ReturnType<typeof nativeToScVal>[]): Promise<unknown> {
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
  return scValToNative((sim as rpc.Api.SimulateTransactionSuccessResponse).result!.retval)
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
        const root = await callView(config, 'merkle_root', [])
        const leafCount = await callView(config, 'leaf_count', [])
        sendJson(res, 200, {
          root: Buffer.from(root as Uint8Array).toString('hex'),
          leafCount: Number(leafCount),
        })
        return
      }

      const merklePathMatch = url.pathname.match(/^\/merkle\/path\/(\d+)$/)
      if (req.method === 'GET' && merklePathMatch) {
        const leafIndex = Number(merklePathMatch[1])
        const path = await callView(config, 'merkle_path', [nativeToScVal(leafIndex, { type: 'u32' })]) as Uint8Array[]
        const root = await callView(config, 'merkle_root', [])
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
