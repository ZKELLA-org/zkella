// The indexer's storage interface, and the factory that picks a backend.
//
// Two implementations exist side by side, both satisfying `IndexerDb`:
//   - `SqliteIndexerDb` (`./db-sqlite.ts`) - the original zero-dependency
//     reference implementation (Node's built-in `node:sqlite`), kept as the
//     baseline this deliverable's own load-test results are published
//     against (see `scripts/indexer_load_test.ts`), and as a no-external-
//     dependency option for local development.
//   - `PostgresIndexerDb` (`./db-postgres.ts`) - the grant-funded,
//     containerized stack (see `docker-compose.yml`), used whenever
//     `DATABASE_URL` is set. This is the one meant for a real deployment:
//     concurrent writers, replication, and the operational tooling
//     (`pg_dump`, monitoring, managed hosting) a production indexer needs,
//     none of which SQLite's single-file, single-writer model gives you.
//
// Both are async (even SQLite's, whose underlying calls are synchronous)
// so `sync.ts`/`http.ts`/tests can depend on the interface without caring
// which backend is actually running.

export interface StoredNote {
  leafIndex:     number
  commitment:    string // hex
  encryptedNote: string // hex
  ledger:        number
}

export interface IndexerDb {
  getLastSyncedLedger(startLedger: number): Promise<number>
  setLastSyncedLedger(ledger: number): Promise<void>
  upsertNote(note: StoredNote): Promise<void>
  markNullifierSpent(nullifierHex: string, ledger: number): Promise<void>
  getNotesFrom(fromLedger: number, limit: number): Promise<{ notes: StoredNote[]; nextLedger: number }>
  getLeafByCommitment(commitmentHex: string): Promise<number | null>
  isNullifierSpent(nullifierHex: string): Promise<boolean>
  /** Total row counts, for `/metrics` and the load-test script. */
  countRows(): Promise<{ notes: number; nullifiers: number }>
  close(): Promise<void>
}

/**
 * Opens the backend named by `DATABASE_URL` (a `postgres://...` connection
 * string) if set, otherwise falls back to the SQLite reference
 * implementation at `sqlitePath`. This is the only place that chooses
 * between the two - everything else depends on `IndexerDb` alone.
 */
export async function openIndexerDb(opts: { databaseUrl?: string; sqlitePath: string }): Promise<IndexerDb> {
  if (opts.databaseUrl) {
    const { PostgresIndexerDb } = await import('./db-postgres.ts')
    return PostgresIndexerDb.connect(opts.databaseUrl)
  }
  const { SqliteIndexerDb } = await import('./db-sqlite.ts')
  return new SqliteIndexerDb(opts.sqlitePath)
}
