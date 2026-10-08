// Entry point: `node --experimental-strip-types indexer/src/main.ts`
//
// Required env vars:
//   TOKEN_CONTRACT_ID    - deployed token contract address
//   SOROBAN_RPC_URL     - e.g. https://soroban-testnet.stellar.org
//   ZKELLA_NETWORK      - "testnet" | "mainnet"
//   INDEXER_START_LEDGER - ledger to begin syncing from (typically token's deploy ledger)
// Optional:
//   DATABASE_URL        - postgres://... connection string. When set, the indexer runs
//                          against PostgreSQL (the grant-funded, containerized stack - see
//                          docker-compose.yml); when unset, it falls back to the SQLite
//                          reference implementation.
//   INDEXER_DB_PATH     - SQLite file path when DATABASE_URL is unset (default: ./indexer.db)
//   INDEXER_HTTP_PORT   - (default: 8787)
//   INDEXER_POLL_MS     - (default: 5000)
//   INDEXER_API_KEYS    - comma-separated bearer tokens required on every /notes,
//                          /merkle/*, /nullifiers/* and /commitment/* request (see
//                          http.ts's doc comment). Unset disables auth - fine for local
//                          development, not for a public-facing deployment.
//   INDEXER_RATE_LIMIT_PER_MINUTE - per-key/per-IP request budget (default: 600)

import { openIndexerDb } from './db.ts'
import { Syncer } from './sync.ts'
import { startHttpServer } from './http.ts'

function requireEnv(name: string): string {
  const value = process.env[name]
  if (!value) throw new Error(`missing required env var: ${name}`)
  return value
}

async function main() {
  const tokenAddress = requireEnv('TOKEN_CONTRACT_ID')
  const rpcUrl       = requireEnv('SOROBAN_RPC_URL')
  const network      = requireEnv('ZKELLA_NETWORK') as 'testnet' | 'mainnet'
  const startLedger  = Number(requireEnv('INDEXER_START_LEDGER'))

  const databaseUrl = process.env.DATABASE_URL
  const sqlitePath  = process.env.INDEXER_DB_PATH ?? './indexer.db'
  const port        = Number(process.env.INDEXER_HTTP_PORT ?? '8787')
  const pollMs      = Number(process.env.INDEXER_POLL_MS ?? '5000')
  const apiKeys     = (process.env.INDEXER_API_KEYS ?? '').split(',').map(s => s.trim()).filter(Boolean)
  if (network === 'mainnet' && apiKeys.length === 0) {
    throw new Error('refusing to start on mainnet with INDEXER_API_KEYS unset: query endpoints would be unauthenticated')
  }
  const rateLimitPerMinute = Number(process.env.INDEXER_RATE_LIMIT_PER_MINUTE ?? '600')

  const db = await openIndexerDb({ databaseUrl, sqlitePath })

  const syncer = new Syncer({ rpcUrl, tokenAddress, db, startLedger, pollIntervalMs: pollMs })
  const httpServer = startHttpServer({ db, tokenAddress, rpcUrl, network, port, startLedger, apiKeys, rateLimitPerMinute })

  const shutdown = () => {
    console.log('[indexer] shutting down')
    syncer.stop()
    httpServer.close()
    db.close().catch(err => console.error('[indexer] error closing db:', err))
    process.exit(0)
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)

  console.log(`[indexer] backend: ${databaseUrl ? 'postgres' : 'sqlite (' + sqlitePath + ')'}`)
  // db.getLastSyncedLedger falls back to startLedger only when no cursor was ever
  // persisted; logging startLedger unconditionally here used to make a restart
  // against an existing database look like it was about to re-sync from scratch,
  // which it isn't — see RUNBOOK.md Category 2.
  const resumeLedger = await db.getLastSyncedLedger(startLedger)
  console.log(resumeLedger === startLedger
    ? `[indexer] syncing ${tokenAddress} on ${network} from ledger ${startLedger}`
    : `[indexer] syncing ${tokenAddress} on ${network}, resuming from persisted ledger ${resumeLedger} (configured start was ${startLedger})`)
  if (apiKeys.length === 0) {
    console.log('[indexer] WARNING: INDEXER_API_KEYS is unset - query endpoints are unauthenticated')
  }
  await syncer.run()
}

main().catch(err => {
  console.error('[indexer] fatal:', err)
  process.exit(1)
})
