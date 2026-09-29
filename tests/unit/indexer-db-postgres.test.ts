// Runs the exact same behavior `indexer-db.test.ts` checks against
// `SqliteIndexerDb`, against a real PostgreSQL server instead — proving the
// two backends are interchangeable behind `IndexerDb`, not just structurally
// similar. Needs `DATABASE_URL` pointing at a real, empty-database-capable
// Postgres server; skipped entirely (not failed) when it's unset, since a
// local `npm test` run shouldn't require standing up Postgres just to pass —
// CI provides one via `docker-compose` / a service container (see
// .github/workflows/ci.yml).
import { PostgresIndexerDb } from '../../indexer/src/db-postgres'

const DATABASE_URL = process.env.DATABASE_URL
const maybeDescribe = DATABASE_URL ? describe : describe.skip

maybeDescribe('PostgresIndexerDb (real server, DATABASE_URL set)', () => {
  let db: PostgresIndexerDb

  beforeEach(async () => {
    db = await PostgresIndexerDb.connect(DATABASE_URL as string)
    // Each test gets a clean slate — this suite owns the whole database
    // named in DATABASE_URL, not just a schema within a shared one.
    await (db as any).pool.query('TRUNCATE notes, nullifiers, sync_state')
  })

  afterEach(async () => {
    await db.close()
  })

  test('getLastSyncedLedger defaults to startLedger before any sync', async () => {
    expect(await db.getLastSyncedLedger(12345)).toBe(12345)
  })

  test('setLastSyncedLedger persists and is read back', async () => {
    await db.setLastSyncedLedger(500)
    expect(await db.getLastSyncedLedger(0)).toBe(500)
    await db.setLastSyncedLedger(600)
    expect(await db.getLastSyncedLedger(0)).toBe(600)
  })

  test('upsertNote stores a note and getNotesFrom returns it', async () => {
    await db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb'.repeat(88), ledger: 100 })
    const { notes, nextLedger } = await db.getNotesFrom(0, 10)
    expect(notes).toHaveLength(1)
    expect(notes[0].leafIndex).toBe(0)
    expect(notes[0].commitment).toBe('aa'.repeat(32))
    expect(nextLedger).toBe(101)
  })

  test('upsertNote is idempotent for the same leaf_index (duplicate events)', async () => {
    await db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb'.repeat(88), ledger: 100 })
    await db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb'.repeat(88), ledger: 100 })
    const { notes } = await db.getNotesFrom(0, 10)
    expect(notes).toHaveLength(1)
  })

  test('getNotesFrom respects fromLedger and limit, ordered by leaf_index, with boundary fix-up', async () => {
    for (let i = 0; i < 5; i++) {
      await db.upsertNote({ leafIndex: i, commitment: `c${i}`.padStart(64, '0'), encryptedNote: 'bb', ledger: 100 + i })
    }
    const page1 = await db.getNotesFrom(0, 2)
    expect(page1.notes.map(n => n.leafIndex)).toEqual([0, 1])
    expect(page1.nextLedger).toBe(102)
    const page2 = await db.getNotesFrom(page1.nextLedger, 2)
    expect(page2.notes.map(n => n.leafIndex)).toEqual([2, 3])
  })

  test('markNullifierSpent + isNullifierSpent round-trip', async () => {
    expect(await db.isNullifierSpent('deadbeef')).toBe(false)
    await db.markNullifierSpent('deadbeef', 200)
    expect(await db.isNullifierSpent('deadbeef')).toBe(true)
    expect(await db.isNullifierSpent('other')).toBe(false)
  })

  test('getLeafByCommitment finds an indexed note, null for unknown commitment', async () => {
    await db.upsertNote({ leafIndex: 7, commitment: 'ff'.repeat(32), encryptedNote: 'bb', ledger: 100 })
    expect(await db.getLeafByCommitment('ff'.repeat(32))).toBe(7)
    expect(await db.getLeafByCommitment('00'.repeat(32))).toBeNull()
  })

  test('countRows reflects inserted notes and nullifiers', async () => {
    expect(await db.countRows()).toEqual({ notes: 0, nullifiers: 0 })
    await db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb', ledger: 100 })
    await db.markNullifierSpent('deadbeef', 100)
    await db.markNullifierSpent('beefdead', 101)
    expect(await db.countRows()).toEqual({ notes: 1, nullifiers: 2 })
  })

  test('a real process restart (fresh connection to the same server) resumes from the persisted cursor with no missed or duplicated events', async () => {
    await db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb', ledger: 100 })
    await db.upsertNote({ leafIndex: 1, commitment: 'bb'.repeat(32), encryptedNote: 'cc', ledger: 101 })
    await db.setLastSyncedLedger(101)

    // A genuinely new connection/process, not the same in-memory object —
    // this is what actually distinguishes this test from the SQLite
    // in-process equivalent. `db` (the original connection) is left open;
    // `afterEach` closes it as usual.
    const restarted = await PostgresIndexerDb.connect(DATABASE_URL as string)
    expect(await restarted.getLastSyncedLedger(0)).toBe(101)
    const { notes } = await restarted.getNotesFrom(0, 10)
    expect(notes.map(n => n.leafIndex)).toEqual([0, 1])

    // Reprocessing ledger 101's events again, as a real resume-from-last-
    // synced-ledger restart does, must not duplicate anything.
    await restarted.upsertNote({ leafIndex: 1, commitment: 'bb'.repeat(32), encryptedNote: 'cc', ledger: 101 })
    const after = await restarted.getNotesFrom(0, 10)
    expect(after.notes).toHaveLength(2)
    await restarted.close()
  })
})
