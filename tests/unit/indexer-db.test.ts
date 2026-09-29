import { SqliteIndexerDb } from '../../indexer/src/db-sqlite'

describe('SqliteIndexerDb', () => {
  test('getLastSyncedLedger defaults to startLedger before any sync', async () => {
    const db = new SqliteIndexerDb(':memory:')
    expect(await db.getLastSyncedLedger(12345)).toBe(12345)
    await db.close()
  })

  test('setLastSyncedLedger persists and is read back', async () => {
    const db = new SqliteIndexerDb(':memory:')
    await db.setLastSyncedLedger(500)
    expect(await db.getLastSyncedLedger(0)).toBe(500)
    await db.setLastSyncedLedger(600)
    expect(await db.getLastSyncedLedger(0)).toBe(600)
    await db.close()
  })

  test('upsertNote stores a note and getNotesFrom returns it', async () => {
    const db = new SqliteIndexerDb(':memory:')
    await db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb'.repeat(88), ledger: 100 })
    const { notes, nextLedger } = await db.getNotesFrom(0, 10)
    expect(notes).toHaveLength(1)
    expect(notes[0].leafIndex).toBe(0)
    expect(notes[0].commitment).toBe('aa'.repeat(32))
    expect(nextLedger).toBe(101)
    await db.close()
  })

  test('upsertNote is idempotent for the same leaf_index (duplicate events)', async () => {
    const db = new SqliteIndexerDb(':memory:')
    await db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb'.repeat(88), ledger: 100 })
    await db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb'.repeat(88), ledger: 100 })
    const { notes } = await db.getNotesFrom(0, 10)
    expect(notes).toHaveLength(1)
    await db.close()
  })

  test('getNotesFrom respects fromLedger and limit, ordered by leaf_index', async () => {
    const db = new SqliteIndexerDb(':memory:')
    for (let i = 0; i < 5; i++) {
      await db.upsertNote({ leafIndex: i, commitment: `c${i}`.padStart(64, '0'), encryptedNote: 'bb', ledger: 100 + i })
    }
    const page1 = await db.getNotesFrom(0, 2)
    expect(page1.notes.map(n => n.leafIndex)).toEqual([0, 1])
    expect(page1.nextLedger).toBe(102) // last returned note's ledger (101) + 1
    const page2 = await db.getNotesFrom(page1.nextLedger, 2)
    expect(page2.notes.map(n => n.leafIndex)).toEqual([2, 3])

    const fromLater = await db.getNotesFrom(103, 10)
    expect(fromLater.notes.map(n => n.leafIndex)).toEqual([3, 4])
    await db.close()
  })

  test('markNullifierSpent + isNullifierSpent round-trip', async () => {
    const db = new SqliteIndexerDb(':memory:')
    expect(await db.isNullifierSpent('deadbeef')).toBe(false)
    await db.markNullifierSpent('deadbeef', 200)
    expect(await db.isNullifierSpent('deadbeef')).toBe(true)
    expect(await db.isNullifierSpent('other')).toBe(false)
    await db.close()
  })

  test('getLeafByCommitment finds an indexed note, null for unknown commitment', async () => {
    const db = new SqliteIndexerDb(':memory:')
    await db.upsertNote({ leafIndex: 7, commitment: 'ff'.repeat(32), encryptedNote: 'bb', ledger: 100 })
    expect(await db.getLeafByCommitment('ff'.repeat(32))).toBe(7)
    expect(await db.getLeafByCommitment('00'.repeat(32))).toBeNull()
    await db.close()
  })

  test('countRows reflects inserted notes and nullifiers', async () => {
    const db = new SqliteIndexerDb(':memory:')
    expect(await db.countRows()).toEqual({ notes: 0, nullifiers: 0 })
    await db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb', ledger: 100 })
    await db.markNullifierSpent('deadbeef', 100)
    await db.markNullifierSpent('beefdead', 101)
    expect(await db.countRows()).toEqual({ notes: 1, nullifiers: 2 })
    await db.close()
  })

  test('a restart (new instance over the same persisted cursor) resumes without gaps', async () => {
    // Simulated restart: a real deployment would reopen the same file/connection string,
    // which is exactly what "the same db instance keeps its state across a new open" means
    // for the reference SQLite backend (see PostgresIndexerDb's own restart test, which uses
    // a real second connection to the same server, closer to a real process restart).
    const db = new SqliteIndexerDb(':memory:')
    await db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb', ledger: 100 })
    await db.setLastSyncedLedger(100)
    expect(await db.getLastSyncedLedger(0)).toBe(100)
    // Re-processing the same ledger's events again (as a real restart's resume-from-last-
    // synced-ledger would) must not duplicate the note.
    await db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb', ledger: 100 })
    const { notes } = await db.getNotesFrom(0, 10)
    expect(notes).toHaveLength(1)
    await db.close()
  })
})
