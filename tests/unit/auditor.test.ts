import { encryptNote } from '../../sdk/src/notes/encrypt'
import { ZKELLAKeys }   from '../../sdk/src/keys/keys'
import { ZKELLAAuditor, decryptNoteForAuditor } from '../../sdk/src/wallet/auditor'
import { Note, ViewingKeyExport } from '../../sdk/src/types'
import { IndexerNote } from '../../sdk/src/indexer/client'

const ASSET = 'CASSET'
const toHex = (b: Uint8Array) => Buffer.from(b).toString('hex')

function noteFor(value: bigint): Note {
  return {
    value,
    assetId:    ASSET,
    rho:        new Uint8Array(32).fill(1),
    rcm:        new Uint8Array(32).fill(2),
    leafIndex:  0,
    commitment: new Uint8Array(32),
    ownerPk:    new Uint8Array(32),
  }
}

async function indexerRecord(
  owner: ZKELLAKeys,
  value: bigint,
  leafIndex: number,
  ledger: number,
): Promise<IndexerNote> {
  const bundle = await encryptNote(noteFor(value), owner.spendingKey.transmissionKey)
  return {
    leafIndex,
    commitment:    toHex(new Uint8Array(32).fill(leafIndex + 1)),
    encryptedNote: toHex(bundle),
    ledger,
  }
}

describe('viewing-key decryption (auditor)', () => {
  test('a granted viewing key recovers the note value and its position', async () => {
    const owner = await ZKELLAKeys.fromSeed(new Uint8Array(32).fill(7))
    const raw   = await indexerRecord(owner, 1_500_000n, 4, 300)
    const note  = await decryptNoteForAuditor(toHex(owner.spendingKey.viewingKey), raw)
    expect(note).not.toBeNull()
    expect(note!.value).toBe(1_500_000n)
    expect(note!.assetId).toBe(ASSET)
    expect(note!.leafIndex).toBe(4)
    expect(note!.ledger).toBe(300)
  })

  test('a viewing key for a different wallet recovers nothing', async () => {
    const owner = await ZKELLAKeys.fromSeed(new Uint8Array(32).fill(7))
    const other = await ZKELLAKeys.fromSeed(new Uint8Array(32).fill(9))
    const raw   = await indexerRecord(owner, 1_500_000n, 4, 300)
    expect(await decryptNoteForAuditor(toHex(other.spendingKey.viewingKey), raw)).toBeNull()
  })
})

describe('ZKELLAAuditor sync', () => {
  const owner = () => ZKELLAKeys.fromSeed(new Uint8Array(32).fill(7))

  function exportFor(keys: ZKELLAKeys, birthday: number): ViewingKeyExport {
    return keys.exportViewingKey(birthday, 'testnet')
  }

  afterEach(() => { jest.restoreAllMocks() })

  test('sync collects every note encrypted to the granted key and ignores others', async () => {
    const keys  = await owner()
    const other = await ZKELLAKeys.fromSeed(new Uint8Array(32).fill(9))
    const page1 = [
      await indexerRecord(keys, 100n, 0, 10),
      await indexerRecord(other, 999n, 1, 11),
    ]
    const responses = [
      { notes: page1, nextLedger: 20 },
      { notes: [],    nextLedger: 20 },
    ]
    jest.spyOn(global, 'fetch').mockImplementation(async () => {
      const body = responses.shift()!
      return { ok: true, status: 200, json: async () => body } as unknown as Response
    })

    const auditor = new ZKELLAAuditor({ viewingKeyExport: exportFor(keys, 0), indexerUrl: 'http://x' })
    await auditor.sync()
    expect(auditor.transactionHistory(ASSET)).toEqual([{ type: 'receive', amount: 100n, ledger: 10 }])
  })

  test('zero-value padding notes are not reported as receipts', async () => {
    const keys = await owner()
    const page = [
      await indexerRecord(keys, 0n, 0, 10),
      await indexerRecord(keys, 25n, 1, 11),
    ]
    const responses = [
      { notes: page, nextLedger: 20 },
      { notes: [],   nextLedger: 20 },
    ]
    jest.spyOn(global, 'fetch').mockImplementation(async () => {
      const body = responses.shift()!
      return { ok: true, status: 200, json: async () => body } as unknown as Response
    })

    const auditor = new ZKELLAAuditor({ viewingKeyExport: exportFor(keys, 0), indexerUrl: 'http://x' })
    await auditor.sync()
    expect(auditor.transactionHistory(ASSET)).toEqual([{ type: 'receive', amount: 25n, ledger: 11 }])
  })

  test('sync stops when the indexer does not advance its cursor', async () => {
    const keys = await owner()
    const page = [await indexerRecord(keys, 5n, 0, 1)]
    const spy = jest.spyOn(global, 'fetch').mockImplementation(async () =>
      ({ ok: true, status: 200, json: async () => ({ notes: page, nextLedger: 0 }) }) as unknown as Response,
    )
    const auditor = new ZKELLAAuditor({ viewingKeyExport: exportFor(keys, 0), indexerUrl: 'http://x' })
    await auditor.sync()
    expect(spy).toHaveBeenCalledTimes(1)
  })
})
