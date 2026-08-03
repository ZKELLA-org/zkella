// HTTP API matching `sdk/src/indexer/client.ts`'s `IndexerClient` exactly —
// that file is the contract this server implements.

import { createServer, IncomingMessage, ServerResponse } from 'node:http'
import { SorobanRpc, Contract, Account, Keypair, TransactionBuilder, Networks, nativeToScVal, scValToNative } from '@stellar/stellar-sdk'
import { IndexerDb } from './db.ts'

// A read-only simulation needs *some* syntactically valid source account —
// it never signs or submits anything, so any keypair works, funded or not.
// Generated once at module load rather than hardcoded: a hand-typed StrKey
// is easy to get subtly wrong (this one was, the first time — wrong
// checksum bytes — caught by the indexer's own live-Testnet smoke test).
const SIMULATION_KEYPAIR = Keypair.random()

export interface HttpConfig {
  db:          IndexerDb
  ct20Address: string
  rpcUrl:      string
  network:     'testnet' | 'mainnet'
  port:        number
  startLedger: number
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
 * Proxies `ct20.merkle_path`/`merkle_root` view calls directly rather than
 * maintaining a redundant second copy of the Merkle tree — the contract is
 * already the source of truth for current tree state; the indexer's own
 * database only needs to cover what the contract *can't* serve itself
 * (historical note/nullifier events past Stellar RPC's retention window).
 */
async function callView(config: HttpConfig, method: string, args: ReturnType<typeof nativeToScVal>[]): Promise<unknown> {
  const server = new SorobanRpc.Server(config.rpcUrl)
  // A read-only simulation doesn't need a real funded account — any valid
  // account ID works as the simulation's nominal source.
  const dummyAccount = new Account(SIMULATION_KEYPAIR.publicKey(), '0')
  const tx = new TransactionBuilder(dummyAccount, {
    fee: '100',
    networkPassphrase: config.network === 'mainnet' ? Networks.PUBLIC : Networks.TESTNET,
  })
    .addOperation(new Contract(config.ct20Address).call(method, ...args))
    .setTimeout(10)
    .build()

  const sim = await server.simulateTransaction(tx)
  if (SorobanRpc.Api.isSimulationError(sim)) {
    throw new Error(`${method} simulation error: ${sim.error}`)
  }
  return scValToNative((sim as SorobanRpc.Api.SimulateTransactionSuccessResponse).result!.retval)
}

export function startHttpServer(config: HttpConfig): ReturnType<typeof createServer> {
  const server = createServer(async (req, res) => {
    try {
      const url = new URL(req.url ?? '/', 'http://localhost')

      if (req.method === 'GET' && url.pathname === '/health') {
        const synced = config.db.getLastSyncedLedger(config.startLedger)
        const rpc = new SorobanRpc.Server(config.rpcUrl)
        const tip = (await rpc.getLatestLedger()).sequence
        sendJson(res, 200, { syncedLedger: synced, tipLedger: tip, lag: Math.max(0, tip - synced) })
        return
      }

      if (req.method === 'GET' && url.pathname === '/notes') {
        const fromLedger = Number(url.searchParams.get('from_ledger') ?? '0')
        const limit = Number(url.searchParams.get('limit') ?? '500')
        sendJson(res, 200, config.db.getNotesFrom(fromLedger, limit))
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
        for (const nf of body.nullifiers ?? []) spent[nf] = config.db.isNullifierSpent(nf)
        sendJson(res, 200, { spent })
        return
      }

      const commitmentMatch = url.pathname.match(/^\/commitment\/([0-9a-f]+)$/)
      if (req.method === 'GET' && commitmentMatch) {
        const leafIndex = config.db.getLeafByCommitment(commitmentMatch[1])
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

/** Direction bits for `leafIndex`, matching `contracts/ct20::merkle::get_path_indices`. */
function pathIndicesFor(leafIndex: number, depth: number): number[] {
  const bits: number[] = []
  let idx = leafIndex
  for (let i = 0; i < depth; i++) {
    bits.push(idx & 1)
    idx = Math.floor(idx / 2)
  }
  return bits
}
