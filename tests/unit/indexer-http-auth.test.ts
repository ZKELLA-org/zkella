// Tranche 2 Deliverable 5's success criterion: query endpoints require real
// request authentication and enforce rate limiting, confirmed by a test
// showing an unauthenticated or excessive request is correctly rejected.
// Runs a real HTTP server (against an in-memory fake `IndexerDb`, so no
// database or live Soroban RPC is needed) and makes real HTTP requests
// against it — not just unit-testing the helper functions in isolation.

import { AddressInfo } from 'node:net'
import { parseBearerToken, matchesApiKey, RateLimiter, startHttpServer } from '../../indexer/src/http.ts'
import type { IndexerDb, StoredNote } from '../../indexer/src/db.ts'

describe('parseBearerToken', () => {
  test('extracts the token from a well-formed header', () => {
    expect(parseBearerToken('Bearer abc123')).toBe('abc123')
  })
  test('returns null for a missing header', () => {
    expect(parseBearerToken(undefined)).toBeNull()
  })
  test('returns null for a header with the wrong scheme', () => {
    expect(parseBearerToken('Basic abc123')).toBeNull()
  })
  test('returns null for an empty bearer token', () => {
    expect(parseBearerToken('Bearer')).toBeNull()
  })
})

describe('matchesApiKey', () => {
  test('matches a key that is present in the set', () => {
    expect(matchesApiKey('secret-key', new Set(['other-key', 'secret-key']))).toBe(true)
  })
  test('rejects a key not in the set', () => {
    expect(matchesApiKey('wrong-key', new Set(['secret-key']))).toBe(false)
  })
  test('rejects a key that only differs in length from a real one', () => {
    expect(matchesApiKey('secret-key-extra', new Set(['secret-key']))).toBe(false)
  })
  test('rejects a prefix of a real key (not just any mismatch)', () => {
    expect(matchesApiKey('secret', new Set(['secret-key']))).toBe(false)
  })
  test('an empty set matches nothing', () => {
    expect(matchesApiKey('anything', new Set())).toBe(false)
  })
})

describe('RateLimiter', () => {
  test('allows up to the limit within one window, then rejects', () => {
    const rl = new RateLimiter(3)
    const now = 1_000_000
    expect(rl.hit('a', now)).toBe(true)
    expect(rl.hit('a', now)).toBe(true)
    expect(rl.hit('a', now)).toBe(true)
    expect(rl.hit('a', now)).toBe(false)
  })

  test('resets once the window elapses', () => {
    const rl = new RateLimiter(1, 1000)
    const now = 1_000_000
    expect(rl.hit('a', now)).toBe(true)
    expect(rl.hit('a', now + 500)).toBe(false)
    expect(rl.hit('a', now + 1001)).toBe(true)
  })

  test('tracks separate identities independently', () => {
    const rl = new RateLimiter(1)
    const now = 1_000_000
    expect(rl.hit('a', now)).toBe(true)
    expect(rl.hit('b', now)).toBe(true)
    expect(rl.hit('a', now)).toBe(false)
    expect(rl.hit('b', now)).toBe(false)
  })
})

function fakeDb(): IndexerDb {
  const notes: StoredNote[] = []
  return {
    async getLastSyncedLedger() { return 0 },
    async setLastSyncedLedger() {},
    async upsertNote(n) { notes.push(n) },
    async markNullifierSpent() {},
    async getNotesFrom(fromLedger, limit) {
      const filtered = notes.filter(n => n.ledger >= fromLedger).slice(0, limit)
      return { notes: filtered, nextLedger: fromLedger }
    },
    async getLeafByCommitment() { return null },
    async isNullifierSpent() { return false },
    async countRows() { return { notes: notes.length, nullifiers: 0 } },
    async close() {},
  }
}

function startTestServer(opts: { apiKeys?: string[]; rateLimitPerMinute?: number }) {
  const server = startHttpServer({
    db: fakeDb(),
    tokenAddress: 'CTEST',
    rpcUrl: 'http://unused.invalid',
    network: 'testnet',
    port: 0,
    startLedger: 0,
    apiKeys: opts.apiKeys,
    rateLimitPerMinute: opts.rateLimitPerMinute,
  })
  const { port } = server.address() as AddressInfo
  return { server, base: `http://127.0.0.1:${port}` }
}

describe('startHttpServer auth + rate limiting (real HTTP requests)', () => {
  test('rejects a query request with no Authorization header when API keys are configured', async () => {
    const { server, base } = startTestServer({ apiKeys: ['secret-key'] })
    try {
      const res = await fetch(`${base}/notes`)
      expect(res.status).toBe(401)
    } finally {
      server.close()
    }
  })

  test('rejects a query request with a wrong bearer token', async () => {
    const { server, base } = startTestServer({ apiKeys: ['secret-key'] })
    try {
      const res = await fetch(`${base}/notes`, { headers: { Authorization: 'Bearer wrong-key' } })
      expect(res.status).toBe(401)
    } finally {
      server.close()
    }
  })

  test('accepts a query request with the correct bearer token', async () => {
    const { server, base } = startTestServer({ apiKeys: ['secret-key'] })
    try {
      const res = await fetch(`${base}/notes`, { headers: { Authorization: 'Bearer secret-key' } })
      expect(res.status).toBe(200)
    } finally {
      server.close()
    }
  })

  test('/health and /metrics stay reachable with no Authorization header even when API keys are configured', async () => {
    const { server, base } = startTestServer({ apiKeys: ['secret-key'] })
    try {
      // /health also calls out to Soroban RPC, which isn't mocked here — the
      // point of this test is that it isn't rejected for *missing auth*
      // (401), not that the RPC call itself succeeds against a fake URL.
      const health = await fetch(`${base}/health`)
      expect(health.status).not.toBe(401)
      // /metrics only reads the local db, so it can be checked fully.
      const metrics = await fetch(`${base}/metrics`)
      expect(metrics.status).toBe(200)
      const body = await metrics.json()
      expect(body).toHaveProperty('indexedNotes')
      expect(body).toHaveProperty('requestsTotal')
    } finally {
      server.close()
    }
  })

  test('an excessive number of requests from the same (unauthenticated) client is rejected with 429', async () => {
    const { server, base } = startTestServer({ rateLimitPerMinute: 3 })
    try {
      const statuses: number[] = []
      for (let i = 0; i < 5; i++) {
        const res = await fetch(`${base}/notes`)
        statuses.push(res.status)
      }
      expect(statuses.slice(0, 3)).toEqual([200, 200, 200])
      expect(statuses.slice(3)).toEqual([429, 429])
    } finally {
      server.close()
    }
  })

  test('/health and /metrics are rate-limited too, even though they need no auth', async () => {
    // Each does a real RPC round-trip or DB query — leaving them completely
    // unbounded would be an easy DoS surface, so they're rate-limited by IP
    // like everything else, just not auth-gated.
    const { server, base } = startTestServer({ rateLimitPerMinute: 2 })
    try {
      const statuses: number[] = []
      for (let i = 0; i < 3; i++) {
        const res = await fetch(`${base}/metrics`)
        statuses.push(res.status)
      }
      expect(statuses).toEqual([200, 200, 429])
    } finally {
      server.close()
    }
  })

  test('with no API keys configured, query endpoints remain open (documented, not a silent gap)', async () => {
    const { server, base } = startTestServer({})
    try {
      const res = await fetch(`${base}/notes`)
      expect(res.status).toBe(200)
    } finally {
      server.close()
    }
  })
})
