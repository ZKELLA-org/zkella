// Tranche 2 Deliverable 4's success criterion: sync behavior tested under
// sustained high event volume and a large backfill from an early ledger,
// with results published against the SQLite reference implementation's own
// untested baseline.
//
// This drives both `IndexerDb` backends directly (not through the real
// Soroban RPC sync loop) with a large number of synthetic note/nullifier
// events, in two shapes:
//   - "sustained": events inserted one at a time, simulating steady-state
//     ingestion of new events as they arrive.
//   - "backfill": the same total event count inserted in parallel batches,
//     simulating a fresh indexer catching up from an early ledger.
// Both are run against SqliteIndexerDb and (if DATABASE_URL is set) against
// PostgresIndexerDb, and the results are printed side by side.
//
// Usage (run with the same flag the indexer itself needs for its .ts sources):
//   node --experimental-strip-types scripts/indexer_load_test.mjs [--events N] [--database-url postgres://...]
// N defaults to 20000. DATABASE_URL env var also works in place of --database-url.

import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))

function parseArgs() {
  const args = process.argv.slice(2)
  let events = 20000
  let databaseUrl = process.env.DATABASE_URL
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--events') events = Number(args[++i])
    if (args[i] === '--database-url') databaseUrl = args[++i]
  }
  return { events, databaseUrl }
}

function randomHex(bytes) {
  const buf = Buffer.alloc(bytes)
  for (let i = 0; i < bytes; i++) buf[i] = Math.floor(Math.random() * 256)
  return buf.toString('hex')
}

async function loadDb(kind, databaseUrl) {
  if (kind === 'sqlite') {
    const { SqliteIndexerDb } = await import(path.join(__dirname, '../indexer/src/db-sqlite.ts'))
    return new SqliteIndexerDb(':memory:')
  }
  const { PostgresIndexerDb } = await import(path.join(__dirname, '../indexer/src/db-postgres.ts'))
  const db = await PostgresIndexerDb.connect(databaseUrl)
  await db.pool.query('TRUNCATE notes, nullifiers, sync_state')
  return db
}

async function runSustained(db, n) {
  const start = process.hrtime.bigint()
  for (let i = 0; i < n; i++) {
    await db.upsertNote({ leafIndex: i, commitment: randomHex(32), encryptedNote: randomHex(88), ledger: 1000 + i })
    await db.markNullifierSpent(randomHex(32), 1000 + i)
    await db.setLastSyncedLedger(1000 + i)
  }
  return Number(process.hrtime.bigint() - start) / 1e6
}

async function runBackfillBatch(db, n) {
  const start = process.hrtime.bigint()
  const batch = []
  for (let i = 0; i < n; i++) {
    batch.push(db.upsertNote({ leafIndex: i, commitment: randomHex(32), encryptedNote: randomHex(88), ledger: 1000 + i }))
    batch.push(db.markNullifierSpent(randomHex(32), 1000 + i))
  }
  await Promise.all(batch)
  await db.setLastSyncedLedger(1000 + n)
  return Number(process.hrtime.bigint() - start) / 1e6
}

async function runQueryPass(db, n, pageSize) {
  const start = process.hrtime.bigint()
  let cursor = 0
  let pages = 0
  for (;;) {
    const { notes, nextLedger } = await db.getNotesFrom(cursor, pageSize)
    pages++
    if (notes.length === 0 || nextLedger === cursor) break
    cursor = nextLedger
    if (cursor > 1000 + n) break
  }
  return { elapsedMs: Number(process.hrtime.bigint() - start) / 1e6, pages }
}

async function bench(kind, databaseUrl, n) {
  const sustainedDb = await loadDb(kind, databaseUrl)
  const sustainedMs = await runSustained(sustainedDb, n)
  const { notes } = await sustainedDb.countRows()
  const queryResult = await runQueryPass(sustainedDb, n, 500)
  await sustainedDb.close()

  const backfillDb = await loadDb(kind, databaseUrl)
  const backfillMs = await runBackfillBatch(backfillDb, n)
  await backfillDb.close()

  return {
    kind, n, notesIndexed: notes,
    sustainedMs, sustainedPerSec: (n / (sustainedMs / 1000)).toFixed(1),
    backfillMs, backfillPerSec: (n / (backfillMs / 1000)).toFixed(1),
    queryMs: queryResult.elapsedMs, queryPages: queryResult.pages,
  }
}

const { events, databaseUrl } = parseArgs()
console.log(`Indexer load test: ${events} events per run\n`)

const sqliteResult = await bench('sqlite', undefined, events)
console.log('SQLite (reference baseline):')
console.log(`  sustained ingestion : ${sqliteResult.sustainedMs.toFixed(0)}ms (${sqliteResult.sustainedPerSec} events/s)`)
console.log(`  backfill (parallel) : ${sqliteResult.backfillMs.toFixed(0)}ms (${sqliteResult.backfillPerSec} events/s)`)
console.log(`  query pass (${sqliteResult.queryPages} pages) : ${sqliteResult.queryMs.toFixed(0)}ms`)
console.log(`  rows indexed        : ${sqliteResult.notesIndexed}`)

if (!databaseUrl) {
  console.log('\nDATABASE_URL not set — skipping the PostgreSQL side of the comparison.')
  console.log('Set DATABASE_URL (or pass --database-url) to a real Postgres server to compare both backends.')
  process.exit(0)
}

const pgResult = await bench('postgres', databaseUrl, events)
console.log('\nPostgreSQL (grant-funded stack):')
console.log(`  sustained ingestion : ${pgResult.sustainedMs.toFixed(0)}ms (${pgResult.sustainedPerSec} events/s)`)
console.log(`  backfill (parallel) : ${pgResult.backfillMs.toFixed(0)}ms (${pgResult.backfillPerSec} events/s)`)
console.log(`  query pass (${pgResult.queryPages} pages) : ${pgResult.queryMs.toFixed(0)}ms`)
console.log(`  rows indexed        : ${pgResult.notesIndexed}`)

console.log('\nComparison (Postgres relative to SQLite):')
console.log(`  sustained ingestion : ${(pgResult.sustainedMs / sqliteResult.sustainedMs).toFixed(2)}x the time`)
console.log(`  backfill (parallel) : ${(pgResult.backfillMs / sqliteResult.backfillMs).toFixed(2)}x the time`)
console.log(`  query pass          : ${(pgResult.queryMs / sqliteResult.queryMs).toFixed(2)}x the time`)
