// Regression test for a race between the two sequential RPC view-call
// requests behind `/merkle/root` and `/merkle/path/:leafIndex`. Each
// endpoint issues two independent `simulateTransaction` calls (root +
// leafCount, or path + root) against whatever the RPC node's current tip
// happens to be when each one runs. A ledger close that inserts a new leaf
// in between those two calls makes the returned pair describe two
// different tree states that don't reconstruct to each other — a wallet
// building a proof from a mismatched path+root pair gets a proof that
// fails verification even though both values were individually read
// correctly. Runs a real HTTP server against a fake `IndexerDb` and a
// mocked `rpc.Server`, so no live Soroban RPC is needed.

import { AddressInfo } from 'node:net'
import { xdr } from '@stellar/stellar-sdk'

jest.mock('@stellar/stellar-sdk', () => {
  const actual = jest.requireActual('@stellar/stellar-sdk')
  return { ...actual, rpc: { ...actual.rpc, Server: jest.fn() } }
})

import { rpc } from '@stellar/stellar-sdk'
import { startHttpServer } from '../../indexer/src/http.ts'
import type { IndexerDb } from '../../indexer/src/db.ts'

function fakeDb(): IndexerDb {
  return {
    async getLastSyncedLedger() { return 0 },
    async setLastSyncedLedger() {},
    async upsertNote() {},
    async markNullifierSpent() {},
    async getNotesFrom() { return { notes: [], nextLedger: 0 } },
    async getLeafByCommitment() { return null },
    async isNullifierSpent() { return false },
    async countRows() { return { notes: 0, nullifiers: 0 } },
    async close() {},
  }
}

// A minimal stand-in for `rpc.Api.SimulateTransactionSuccessResponse` —
// just the fields `simulateView` actually reads (`result.retval`,
// `latestLedger`), plus the fields that make `rpc.Api.isSimulationError`
// (the real implementation, not mocked) say "not an error".
function simResult(retval: xdr.ScVal, latestLedger: number) {
  return {
    latestLedger,
    events: [],
    transactionData: {},
    minResourceFee: '100',
    result: { retval },
  }
}

// A syntactically valid contract strkey — `Contract(...)` decodes this
// eagerly when building the simulation transaction, so (unlike the `/notes`
// tests' `'CTEST'`) it has to actually be a well-formed address for these
// merkle-endpoint tests to get anywhere near the mocked RPC call.
const TOKEN_ADDRESS = 'CAAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQCAIBAEAQC526'

function startTestServer(simulateTransaction: jest.Mock) {
  ;(rpc.Server as unknown as jest.Mock).mockImplementation(() => ({ simulateTransaction }))
  const server = startHttpServer({
    db: fakeDb(),
    tokenAddress: TOKEN_ADDRESS,
    rpcUrl: 'http://unused.invalid',
    network: 'testnet',
    port: 0,
    startLedger: 0,
  })
  const { port } = server.address() as AddressInfo
  return { server, base: `http://127.0.0.1:${port}` }
}

describe('/merkle/root and /merkle/path read a ledger-consistent snapshot', () => {
  test('/merkle/root retries when merkle_root and leaf_count land on different ledgers', async () => {
    const rootStale = xdr.ScVal.scvBytes(Buffer.alloc(32, 0xaa))
    const rootFresh = xdr.ScVal.scvBytes(Buffer.alloc(32, 0xbb))
    const simulateTransaction = jest.fn()
      .mockResolvedValueOnce(simResult(rootStale, 100))        // attempt 1: merkle_root @ 100
      .mockResolvedValueOnce(simResult(xdr.ScVal.scvU32(7), 101)) // attempt 1: leaf_count @ 101 - mismatch
      .mockResolvedValueOnce(simResult(rootFresh, 102))        // attempt 2: merkle_root @ 102
      .mockResolvedValueOnce(simResult(xdr.ScVal.scvU32(9), 102)) // attempt 2: leaf_count @ 102 - matches

    const { server, base } = startTestServer(simulateTransaction)
    try {
      const res = await fetch(`${base}/merkle/root`)
      expect(res.status).toBe(200)
      const body = await res.json()
      // Must come from the second (consistent) attempt, never a stale/fresh
      // mix from the first, mismatched attempt.
      expect(body).toEqual({ root: Buffer.alloc(32, 0xbb).toString('hex'), leafCount: 9 })
      expect(simulateTransaction).toHaveBeenCalledTimes(4)
    } finally {
      server.close()
    }
  })

  test('/merkle/path/:leafIndex retries when merkle_path and merkle_root land on different ledgers', async () => {
    const leafBytes = Buffer.alloc(32, 0x01)
    const path = xdr.ScVal.scvVec([xdr.ScVal.scvBytes(leafBytes)])
    const rootStale = xdr.ScVal.scvBytes(Buffer.alloc(32, 0xaa))
    const rootFresh = xdr.ScVal.scvBytes(Buffer.alloc(32, 0xbb))
    const simulateTransaction = jest.fn()
      .mockResolvedValueOnce(simResult(path, 200))       // attempt 1: merkle_path @ 200
      .mockResolvedValueOnce(simResult(rootStale, 201))  // attempt 1: merkle_root @ 201 - mismatch
      .mockResolvedValueOnce(simResult(path, 202))       // attempt 2: merkle_path @ 202
      .mockResolvedValueOnce(simResult(rootFresh, 202))  // attempt 2: merkle_root @ 202 - matches

    const { server, base } = startTestServer(simulateTransaction)
    try {
      const res = await fetch(`${base}/merkle/path/0`)
      expect(res.status).toBe(200)
      const body = await res.json() as { root: string; path: string[] }
      expect(body.root).toBe(Buffer.alloc(32, 0xbb).toString('hex'))
      expect(body.path).toEqual([leafBytes.toString('hex')])
      expect(simulateTransaction).toHaveBeenCalledTimes(4)
    } finally {
      server.close()
    }
  })

  test('gives up and errors rather than ever returning a mismatched pair', async () => {
    let ledger = 0
    // Every call lands on a distinct ledger, so the pair can never agree -
    // this must exhaust retries and fail loudly, not fall back to stale data.
    const simulateTransaction = jest.fn().mockImplementation(() =>
      Promise.resolve(simResult(xdr.ScVal.scvU32(1), ++ledger))
    )
    const { server, base } = startTestServer(simulateTransaction)
    try {
      const res = await fetch(`${base}/merkle/root`)
      expect(res.status).toBe(500)
    } finally {
      server.close()
    }
  })
})
