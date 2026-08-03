// Persistent store backing the indexer's HTTP API.
//
// Uses Node's built-in `node:sqlite` (experimental as of Node 22, stable
// enough for a reference implementation) rather than an external database
// dependency — this is exactly the kind of small, self-contained service
// that shouldn't need to stand up Postgres just to exist. Swapping in a
// real database for production scale is a config/deployment change, not a
// rewrite: every query here is a single indexed lookup or a small range
// scan, nothing SQLite-specific.
//
// Schema is intentionally small: `notes` (append-only, one row per
// `("zkella","note")` event — this is *why* an indexer exists at all, since
// Stellar RPC's own event retention window is too short to serve this
// history directly, see docs/ARCHITECTURE.md), `nullifiers` (append-only,
// one row per `("zkella","nf")` event, tracked for `batchCheckNullifiers`),
// and `sync_state` (a single row tracking how far the sync loop has read).

import { DatabaseSync } from 'node:sqlite'

export interface StoredNote {
  leafIndex:     number
  commitment:    string // hex
  encryptedNote: string // hex
  ledger:        number
}

export class IndexerDb {
  private db: DatabaseSync

  constructor(path: string) {
    this.db = new DatabaseSync(path)
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS notes (
        leaf_index     INTEGER PRIMARY KEY,
        commitment     TEXT NOT NULL UNIQUE,
        encrypted_note TEXT NOT NULL,
        ledger         INTEGER NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_notes_ledger ON notes(ledger);

      CREATE TABLE IF NOT EXISTS nullifiers (
        nullifier   TEXT PRIMARY KEY,
        spent_ledger INTEGER NOT NULL
      );

      CREATE TABLE IF NOT EXISTS sync_state (
        id                  INTEGER PRIMARY KEY CHECK (id = 1),
        last_synced_ledger  INTEGER NOT NULL
      );
    `)
  }

  getLastSyncedLedger(startLedger: number): number {
    const row = this.db.prepare('SELECT last_synced_ledger FROM sync_state WHERE id = 1').get() as
      { last_synced_ledger: number } | undefined
    return row ? row.last_synced_ledger : startLedger
  }

  setLastSyncedLedger(ledger: number): void {
    this.db.prepare(
      'INSERT INTO sync_state (id, last_synced_ledger) VALUES (1, ?) ' +
      'ON CONFLICT(id) DO UPDATE SET last_synced_ledger = excluded.last_synced_ledger'
    ).run(ledger)
  }

  upsertNote(note: StoredNote): void {
    this.db.prepare(
      'INSERT INTO notes (leaf_index, commitment, encrypted_note, ledger) VALUES (?, ?, ?, ?) ' +
      'ON CONFLICT(leaf_index) DO NOTHING'
    ).run(note.leafIndex, note.commitment, note.encryptedNote, note.ledger)
  }

  markNullifierSpent(nullifierHex: string, ledger: number): void {
    this.db.prepare(
      'INSERT INTO nullifiers (nullifier, spent_ledger) VALUES (?, ?) ' +
      'ON CONFLICT(nullifier) DO NOTHING'
    ).run(nullifierHex, ledger)
  }

  getNotesFrom(fromLedger: number, limit: number): { notes: StoredNote[]; nextLedger: number } {
    const rows = this.db.prepare(
      'SELECT leaf_index, commitment, encrypted_note, ledger FROM notes ' +
      'WHERE ledger >= ? ORDER BY leaf_index ASC LIMIT ?'
    ).all(fromLedger, limit) as Array<{ leaf_index: number; commitment: string; encrypted_note: string; ledger: number }>

    const notes = rows.map(r => ({
      leafIndex: r.leaf_index, commitment: r.commitment, encryptedNote: r.encrypted_note, ledger: r.ledger,
    }))
    const nextLedger = notes.length > 0 ? notes[notes.length - 1].ledger + 1 : fromLedger
    return { notes, nextLedger }
  }

  getLeafByCommitment(commitmentHex: string): number | null {
    const row = this.db.prepare('SELECT leaf_index FROM notes WHERE commitment = ?').get(commitmentHex) as
      { leaf_index: number } | undefined
    return row ? row.leaf_index : null
  }

  isNullifierSpent(nullifierHex: string): boolean {
    const row = this.db.prepare('SELECT 1 FROM nullifiers WHERE nullifier = ?').get(nullifierHex)
    return row !== undefined
  }

  close(): void {
    this.db.close()
  }
}
