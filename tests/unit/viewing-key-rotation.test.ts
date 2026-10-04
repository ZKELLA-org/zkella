import { encryptNote }    from '../../sdk/src/notes/encrypt'
import { ZKELLAKeys }      from '../../sdk/src/keys/keys'
import { decryptNoteForAuditor } from '../../sdk/src/wallet/auditor'
import { Note }            from '../../sdk/src/types'

const ASSET = 'CASSET'
const toHex = (b: Uint8Array) => Buffer.from(b).toString('hex')

function noteFor(value: bigint): Note {
  return {
    value,
    assetId:    ASSET,
    rho:        new Uint8Array(32).fill(3),
    rcm:        new Uint8Array(32).fill(4),
    leafIndex:  0,
    commitment: new Uint8Array(32),
    ownerPk:    new Uint8Array(32),
  }
}

async function record(tk: Uint8Array, value: bigint, leafIndex: number) {
  const bundle = await encryptNote(noteFor(value), tk)
  return { leafIndex, commitment: toHex(new Uint8Array(32).fill(leafIndex + 1)), encryptedNote: toHex(bundle), ledger: 50 + leafIndex }
}

describe('viewing-key epochs and revocation by rotation', () => {
  const seed = new Uint8Array(32).fill(5)

  test('epoch 0 viewing key is the original key, so existing notes stay readable', async () => {
    const keys = await ZKELLAKeys.fromSeed(seed)
    expect(toHex(keys.viewingKeyForEpoch(0))).toBe(toHex(keys.spendingKey.viewingKey))
    expect(toHex(await keys.transmissionKeyForEpoch(0))).toBe(toHex(keys.spendingKey.transmissionKey))
  })

  test('after rotation, the old auditor cannot decrypt new notes but still decrypts old ones', async () => {
    const keys = await ZKELLAKeys.fromSeed(seed)
    const oldExport = keys.exportViewingKey(0, 'testnet')

    const beforeRotation = await record(await keys.transmissionKeyForEpoch(0), 100n, 0)
    const newEpochTk     = await keys.transmissionKeyForEpoch(1)
    const afterRotation  = await record(newEpochTk, 250n, 1)

    expect(await decryptNoteForAuditor(oldExport.viewing_key, beforeRotation)).not.toBeNull()
    expect(await decryptNoteForAuditor(oldExport.viewing_key, afterRotation)).toBeNull()
  })

  test('the new epoch auditor reads only new-epoch notes', async () => {
    const keys = await ZKELLAKeys.fromSeed(seed)
    const newExport = await keys.exportViewingKeyForEpoch(0, 'testnet', 1)

    const beforeRotation = await record(await keys.transmissionKeyForEpoch(0), 100n, 0)
    const afterRotation  = await record(await keys.transmissionKeyForEpoch(1), 250n, 1)

    expect(await decryptNoteForAuditor(newExport.viewing_key, beforeRotation)).toBeNull()
    expect((await decryptNoteForAuditor(newExport.viewing_key, afterRotation))?.value).toBe(250n)
  })

  test('each epoch derives an address that differs from every earlier epoch', async () => {
    const keys = await ZKELLAKeys.fromSeed(seed)
    const epoch0 = await keys.deriveAddressForEpoch(0, 0)
    const epoch1 = await keys.deriveAddressForEpoch(1, 0)
    expect(toHex(epoch1.pkD)).not.toBe(toHex(epoch0.pkD))
    expect(epoch1.toString()).not.toBe(epoch0.toString())
    expect(toHex(epoch0.pkD)).toBe(toHex((await keys.deriveAddress(0)).pkD))
  })
})
