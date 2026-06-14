import { encryptNote, tryDecryptNote } from '../../sdk/src/notes/encrypt'
import { buildNote }                   from '../../sdk/src/notes/builder'
import { ZKELLAKeys }                  from '../../sdk/src/keys/keys'

const MOCK_ASSET = 'CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA'

jest.setTimeout(30_000)

describe('Note encryption', () => {

  test('encrypted note is 136 bytes', async () => {
    const keys = ZKELLAKeys.generate()
    const note = await buildNote(100_000_000n, MOCK_ASSET)
    const bundle = encryptNote(note, keys.spendingKey.transmissionKey)
    expect(bundle).toHaveLength(176)
  })

  test('encrypted note decrypts correctly with the right viewing key', async () => {
    const keys = ZKELLAKeys.generate()
    const note = await buildNote(100_000_000n, MOCK_ASSET)

    const bundle    = encryptNote(note, keys.spendingKey.transmissionKey)
    const decrypted = tryDecryptNote(bundle, keys.spendingKey.viewingKey)

    expect(decrypted).not.toBeNull()
    expect(decrypted!.value).toBe(100_000_000n)
    expect(decrypted!.assetId).toBe(MOCK_ASSET)
    expect(decrypted!.rho).toEqual(note.rho)
    expect(decrypted!.rcm).toEqual(note.rcm)
  })

  test('decryption fails with a wrong viewing key', async () => {
    const keys1 = ZKELLAKeys.generate()
    const keys2 = ZKELLAKeys.generate()
    const note  = await buildNote(100_000_000n, MOCK_ASSET)

    const bundle    = encryptNote(note, keys1.spendingKey.transmissionKey)
    const decrypted = tryDecryptNote(bundle, keys2.spendingKey.viewingKey)

    expect(decrypted).toBeNull()
  })

  test('decryption fails with tampered ciphertext', async () => {
    const keys  = ZKELLAKeys.generate()
    const note  = await buildNote(50_000_000n, MOCK_ASSET)
    const bundle = encryptNote(note, keys.spendingKey.transmissionKey)

    // Flip a bit in the ciphertext
    const tampered = new Uint8Array(bundle)
    tampered[50] ^= 0xff

    const decrypted = tryDecryptNote(tampered, keys.spendingKey.viewingKey)
    expect(decrypted).toBeNull()
  })

  test('two encryptions of the same note produce different ciphertexts', async () => {
    const keys  = ZKELLAKeys.generate()
    const note  = await buildNote(100_000_000n, MOCK_ASSET)
    const b1    = encryptNote(note, keys.spendingKey.transmissionKey)
    const b2    = encryptNote(note, keys.spendingKey.transmissionKey)
    // Ephemeral keys are different each time
    expect(b1).not.toEqual(b2)
  })

})
