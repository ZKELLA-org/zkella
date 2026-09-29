// PostgreSQL backend - the grant-funded, containerized reference indexer
// stack (see `db.ts`'s doc comment for how this relates to
// `SqliteIndexerDb`, and `docker-compose.yml` for how it's actually run).
//
// Schema mirrors `SqliteIndexerDb`'s exactly (same tables, same columns,
// same semantics for idempotent writes and the ledger cursor) so the two
// backends are interchangeable behind `IndexerDb` - a query written against
// one reads the same way against the other. `pg`'s connection pool handles
// concurrent requests (the HTTP API and the sync loop share one `Pool`),
// which is the main capability this backend adds over SQLite's single-writer
// file: multiple indexer processes, or a wallet's many concurrent HTTP
// requests, don't serialize behind a single file lock.
import pg from 'pg'
import type { IndexerDb, StoredNote } from './db.ts'

const { Pool } = pg

export class PostgresIndexerDb implements IndexerDb {
  private pool: pg.Pool

  private constructor(pool: pg.Pool) {
    this.pool = pool
  }

  static async connect(connectionString: string): Promise<PostgresIndexerDb> {
    const pool = new Pool({ connectionString })
    const db = new PostgresIndexerDb(pool)
    await db.migrate()
    return db
  }

  private async migrate(): Promise<void> {
    // Idempotent, hand-rolled "migration" - adequate for this reference
    // implementation's single, stable schema. A real production rollout
    // with schema evolution across releases would want a real migration
    // tool (e.g. node-pg-migrate); tracked as remaining work, not silently
    // assumed away - see docs/ARCHITECTURE.md's indexer section.
    await this.pool.query(`
      CREATE TABLE IF NOT EXISTS notes (
        leaf_index     BIGINT PRIMARY KEY,
        commitment     TEXT NOT NULL UNIQUE,
        encrypted_note TEXT NOT NULL,
        ledger         BIGINT NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_notes_ledger ON notes(ledger);

      CREATE TABLE IF NOT EXISTS nullifiers (
        nullifier    TEXT PRIMARY KEY,
        spent_ledger BIGINT NOT NULL
      );

      CREATE TABLE IF NOT EXISTS sync_state (
        id                 INTEGER PRIMARY KEY CHECK (id = 1),
        last_synced_ledger BIGINT NOT NULL
      );
    `)
  }

  async getLastSyncedLedger(startLedger: number): Promise<number> {
    const res = await this.pool.query<{ last_synced_ledger: string }>(
      'SELECT last_synced_ledger FROM sync_state WHERE id = 1'
    )
    return res.rows.length > 0 ? Number(res.rows[0].last_synced_ledger) : startLedger
  }

  async setLastSyncedLedger(ledger: number): Promise<void> {
    await this.pool.query(
      'INSERT INTO sync_state (id, last_synced_ledger) VALUES (1, $1) ' +
      'ON CONFLICT (id) DO UPDATE SET last_synced_ledger = excluded.last_synced_ledger',
      [ledger]
    )
  }

  async upsertNote(note: StoredNote): Promise<void> {
    await this.pool.query(
      'INSERT INTO notes (leaf_index, commitment, encrypted_note, ledger) VALUES ($1, $2, $3, $4) ' +
      'ON CONFLICT (leaf_index) DO NOTHING',
      [note.leafIndex, note.commitment, note.encryptedNote, note.ledger]
    )
  }

  async markNullifierSpent(nullifierHex: string, ledger: number): Promise<void> {
    await this.pool.query(
      'INSERT INTO nullifiers (nullifier, spent_ledger) VALUES ($1, $2) ON CONFLICT (nullifier) DO NOTHING',
      [nullifierHex, ledger]
    )
  }

  async getNotesFrom(fromLedger: number, limit: number): Promise<{ notes: StoredNote[]; nextLedger: number }> {
    type Row = { leaf_index: string; commitment: string; encrypted_note: string; ledger: string }
    const res = await this.pool.query<Row>(
      'SELECT leaf_index, commitment, encrypted_note, ledger FROM notes ' +
      'WHERE ledger >= $1 ORDER BY leaf_index ASC LIMIT $2',
      [fromLedger, limit]
    )
    let rows = res.rows

    // Same boundary-ledger fix-up as `SqliteIndexerDb.getNotesFrom` - see
    // its doc comment for why a full page needs this.
    if (rows.length === limit && rows.length > 0) {
      const boundary = rows[rows.length - 1].ledger
      const seen = new Set(rows.map(r => r.leaf_index))
      const rest = await this.pool.query<Row>(
        'SELECT leaf_index, commitment, encrypted_note, ledger FROM notes WHERE ledger = $1 ORDER BY leaf_index ASC',
        [boundary]
      )
      rows = rows.concat(rest.rows.filter(r => !seen.has(r.leaf_index)))
    }

    const notes = rows.map(r => ({
      leafIndex: Number(r.leaf_index), commitment: r.commitment, encryptedNote: r.encrypted_note, ledger: Number(r.ledger),
    }))
    const nextLedger = notes.length > 0 ? notes[notes.length - 1].ledger + 1 : fromLedger
    return { notes, nextLedger }
  }

  async getLeafByCommitment(commitmentHex: string): Promise<number | null> {
    const res = await this.pool.query<{ leaf_index: string }>(
      'SELECT leaf_index FROM notes WHERE commitment = $1', [commitmentHex]
    )
    return res.rows.length > 0 ? Number(res.rows[0].leaf_index) : null
  }

  async isNullifierSpent(nullifierHex: string): Promise<boolean> {
    const res = await this.pool.query('SELECT 1 FROM nullifiers WHERE nullifier = $1', [nullifierHex])
    return (res.rowCount ?? 0) > 0
  }

  async countRows(): Promise<{ notes: number; nullifiers: number }> {
    const [n, f] = await Promise.all([
      this.pool.query<{ n: string }>('SELECT COUNT(*) AS n FROM notes'),
      this.pool.query<{ n: string }>('SELECT COUNT(*) AS n FROM nullifiers'),
    ])
    return { notes: Number(n.rows[0].n), nullifiers: Number(f.rows[0].n) }
  }

  async close(): Promise<void> {
    await this.pool.end()
  }
}
