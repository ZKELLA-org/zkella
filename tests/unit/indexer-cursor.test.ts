import { nativeToScVal } from '@stellar/stellar-sdk'
import { Syncer } from '../../indexer/src/sync'

function memoryDb() {
  const state = { last: 5000, notes: [] as unknown[], nullifiers: [] as unknown[] }
  return {
    state,
    getLastSyncedLedger: async () => state.last,
    setLastSyncedLedger: async (l: number) => { state.last = l },
    upsertNote: async (n: unknown) => { state.notes.push(n) },
    markNullifierSpent: async (nf: unknown) => { state.nullifiers.push(nf) },
  }
}

const note = (leaf: number, ledger: number) => ({
  id: `${ledger}-${leaf}`, ledger,
  topic: [nativeToScVal('zkella'), nativeToScVal('note')],
  value: nativeToScVal({ leaf_index: leaf, commitment: new Uint8Array(32), encrypted_note: new Uint8Array(176) }),
})

describe('indexer cursor advances past quiet ledgers', () => {
  test('a short page moves the cursor to the RPC latest ledger, not the last event ledger', async () => {
    const db = memoryDb()
    const syncer = new Syncer({ rpcUrl: 'https://soroban-testnet.stellar.org', tokenAddress: 'CAAAA', db, startLedger: 5000, pollIntervalMs: 1 } as never)
    ;(syncer as never as { server: unknown }).server = {
      getEvents: async () => ({ events: [note(0, 5001)], latestLedger: 5050 }),
    }
    await (syncer as never as { syncOnce(): Promise<void> }).syncOnce()
    expect(db.state.last).toBe(5051)
    expect(db.state.notes.length).toBe(1)
  });
})
