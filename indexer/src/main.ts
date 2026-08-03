// Entry point: `node --experimental-strip-types indexer/src/main.ts`
//
// Required env vars:
//   CT20_CONTRACT_ID    — deployed ct20 contract address
//   SOROBAN_RPC_URL     — e.g. https://soroban-testnet.stellar.org
//   ZKELLA_NETWORK      — "testnet" | "mainnet"
//   INDEXER_START_LEDGER — ledger to begin syncing from (typically ct20's deploy ledger)
// Optional:
//   INDEXER_DB_PATH     — SQLite file path (default: ./indexer.db)
//   INDEXER_HTTP_PORT   — (default: 8787)
//   INDEXER_POLL_MS     — (default: 5000)

import { IndexerDb } from './db.ts'
import { Syncer } from './sync.ts'
import { startHttpServer } from './http.ts'

function requireEnv(name: string): string {
  const value = process.env[name]
  if (!value) throw new Error(`missing required env var: ${name}`)
  return value
}

async function main() {
  const ct20Address = requireEnv('CT20_CONTRACT_ID')
  const rpcUrl       = requireEnv('SOROBAN_RPC_URL')
  const network      = requireEnv('ZKELLA_NETWORK') as 'testnet' | 'mainnet'
  const startLedger  = Number(requireEnv('INDEXER_START_LEDGER'))

  const dbPath  = process.env.INDEXER_DB_PATH ?? './indexer.db'
  const port    = Number(process.env.INDEXER_HTTP_PORT ?? '8787')
  const pollMs  = Number(process.env.INDEXER_POLL_MS ?? '5000')

  const db = new IndexerDb(dbPath)

  const syncer = new Syncer({ rpcUrl, ct20Address, db, startLedger, pollIntervalMs: pollMs })
  const httpServer = startHttpServer({ db, ct20Address, rpcUrl, network, port, startLedger })

  const shutdown = () => {
    console.log('[indexer] shutting down')
    syncer.stop()
    httpServer.close()
    db.close()
    process.exit(0)
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)

  console.log(`[indexer] syncing ${ct20Address} on ${network} from ledger ${startLedger}`)
  await syncer.run()
}

main().catch(err => {
  console.error('[indexer] fatal:', err)
  process.exit(1)
})
