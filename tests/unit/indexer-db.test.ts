import { IndexerDb } from '../../indexer/src/db'

describe('IndexerDb', () => {
  test('getLastSyncedLedger defaults to startLedger before any sync', () => {
    const db = new IndexerDb(':memory:')
    expect(db.getLastSyncedLedger(12345)).toBe(12345)
    db.close()
  })

  test('setLastSyncedLedger persists and is read back', () => {
    const db = new IndexerDb(':memory:')
    db.setLastSyncedLedger(500)
    expect(db.getLastSyncedLedger(0)).toBe(500)
    db.setLastSyncedLedger(600)
    expect(db.getLastSyncedLedger(0)).toBe(600)
    db.close()
  })

  test('upsertNote stores a note and getNotesFrom returns it', () => {
    const db = new IndexerDb(':memory:')
    db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb'.repeat(88), ledger: 100 })
    const { notes, nextLedger } = db.getNotesFrom(0, 10)
    expect(notes).toHaveLength(1)
    expect(notes[0].leafIndex).toBe(0)
    expect(notes[0].commitment).toBe('aa'.repeat(32))
    expect(nextLedger).toBe(101)
    db.close()
  })

  test('upsertNote is idempotent for the same leaf_index (duplicate events)', () => {
    const db = new IndexerDb(':memory:')
    db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb'.repeat(88), ledger: 100 })
    db.upsertNote({ leafIndex: 0, commitment: 'aa'.repeat(32), encryptedNote: 'bb'.repeat(88), ledger: 100 })
    const { notes } = db.getNotesFrom(0, 10)
    expect(notes).toHaveLength(1)
    db.close()
  })

  test('getNotesFrom respects fromLedger and limit, ordered by leaf_index', () => {
    const db = new IndexerDb(':memory:')
    for (let i = 0; i < 5; i++) {
      db.upsertNote({ leafIndex: i, commitment: `c${i}`.padStart(64, '0'), encryptedNote: 'bb', ledger: 100 + i })
    }
    const page1 = db.getNotesFrom(0, 2)
    expect(page1.notes.map(n => n.leafIndex)).toEqual([0, 1])
    expect(page1.nextLedger).toBe(102) // last returned note's ledger (101) + 1
    const page2 = db.getNotesFrom(page1.nextLedger, 2)
    expect(page2.notes.map(n => n.leafIndex)).toEqual([2, 3])

    const fromLater = db.getNotesFrom(103, 10)
    expect(fromLater.notes.map(n => n.leafIndex)).toEqual([3, 4])
    db.close()
  })

  test('markNullifierSpent + isNullifierSpent round-trip', () => {
    const db = new IndexerDb(':memory:')
    expect(db.isNullifierSpent('deadbeef')).toBe(false)
    db.markNullifierSpent('deadbeef', 200)
    expect(db.isNullifierSpent('deadbeef')).toBe(true)
    expect(db.isNullifierSpent('other')).toBe(false)
    db.close()
  })

  test('getLeafByCommitment finds an indexed note, null for unknown commitment', () => {
    const db = new IndexerDb(':memory:')
    db.upsertNote({ leafIndex: 7, commitment: 'ff'.repeat(32), encryptedNote: 'bb', ledger: 100 })
    expect(db.getLeafByCommitment('ff'.repeat(32))).toBe(7)
    expect(db.getLeafByCommitment('00'.repeat(32))).toBeNull()
    db.close()
  })
})
