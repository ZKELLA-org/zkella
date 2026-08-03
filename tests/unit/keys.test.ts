import { ZKELLAKeys } from '../../sdk/src/keys/keys'
import { hashToCurveG1, scalarMultPoint } from '../../sdk/src/crypto/bn254'
import { encryptNote, tryDecryptNote } from '../../sdk/src/notes/encrypt'
import { buildNote } from '../../sdk/src/notes/builder'

const BN254_R = 21888242871839275222246405745257275088548364400416034343698204186575808495617n

describe('ZKELLAKeys', () => {

  test('generate produces different keys each time', async () => {
    const k1 = await ZKELLAKeys.generate()
    const k2 = await ZKELLAKeys.generate()
    expect(k1.spendingKey.raw).not.toEqual(k2.spendingKey.raw)
  })

  test('fromSeed is deterministic', async () => {
    const seed = new Uint8Array(32).fill(0x42)
    const k1   = await ZKELLAKeys.fromSeed(seed)
    const k2   = await ZKELLAKeys.fromSeed(seed)
    expect(k1.spendingKey.raw).toEqual(k2.spendingKey.raw)
    expect(k1.spendingKey.nullifierKey).toEqual(k2.spendingKey.nullifierKey)
    expect(k1.spendingKey.viewingKey).toEqual(k2.spendingKey.viewingKey)
    expect(k1.spendingKey.transmissionKey).toEqual(k2.spendingKey.transmissionKey)
  })

  test('all derived keys are different from each other', async () => {
    const seed = new Uint8Array(32).fill(0x99)
    const k    = await ZKELLAKeys.fromSeed(seed)
    expect(k.spendingKey.raw).not.toEqual(k.spendingKey.nullifierKey)
    expect(k.spendingKey.raw).not.toEqual(k.spendingKey.viewingKey)
    expect(k.spendingKey.nullifierKey).not.toEqual(k.spendingKey.viewingKey)
  })

  test('spending key is a valid BN254 scalar field element', async () => {
    const seed = new Uint8Array(32).fill(0x01)
    const k    = await ZKELLAKeys.fromSeed(seed)
    let n = 0n
    for (let i = 31; i >= 0; i--) n = (n << 8n) | BigInt(k.spendingKey.raw[i])
    expect(n).toBeGreaterThan(0n)
    expect(n).toBeLessThan(BN254_R)
  })

  test('fromSeed throws on wrong seed length', async () => {
    await expect(ZKELLAKeys.fromSeed(new Uint8Array(16))).rejects.toThrow('seed must be exactly 32 bytes')
  })

  test('transmission key is a real BN254 point, not the raw viewing key', async () => {
    const seed = new Uint8Array(32).fill(0x07)
    const k    = await ZKELLAKeys.fromSeed(seed)
    expect(k.spendingKey.transmissionKey).toHaveLength(32)
    // The old stub set transmissionKey = viewingKey directly; the real
    // derivation (vk * G) must not equal vk's raw bytes.
    expect(k.spendingKey.transmissionKey).not.toEqual(k.spendingKey.viewingKey)
  })

  test('deriveAddress produces a string starting with zkella1', async () => {
    const k    = await ZKELLAKeys.generate()
    const addr = await k.deriveAddress(0)
    expect(addr.toString()).toMatch(/^zkella1/)
  })

  test('different diversifier indices produce different addresses', async () => {
    const k    = await ZKELLAKeys.generate()
    const a0   = await k.deriveAddress(0)
    const a1   = await k.deriveAddress(1)
    expect(a0.toString()).not.toBe(a1.toString())
  })

  test('pk_d is a real BN254 point derived from the viewing key, not a hash of unrelated inputs', async () => {
    const k    = await ZKELLAKeys.generate()
    const addr = await k.deriveAddress(0)
    const gD   = await hashToCurveG1(addr.diversifier)
    const expectedPkD = await scalarMultPoint(k.spendingKey.viewingKey, gD)
    expect(addr.pkD).toEqual(expectedPkD)
  })

  test('a note encrypted to a diversified address decrypts correctly with the viewing key alone', async () => {
    const k    = await ZKELLAKeys.generate()
    const addr = await k.deriveAddress(3)
    const gD   = await hashToCurveG1(addr.diversifier)

    const note = await buildNote(42_000_000n, 'CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA')
    const bundle = await encryptNote(note, addr.pkD, gD)
    const decrypted = await tryDecryptNote(bundle, k.spendingKey.viewingKey)

    expect(decrypted).not.toBeNull()
    expect(decrypted!.value).toBe(42_000_000n)
    expect(decrypted!.rho).toEqual(note.rho)
  })

  test('exportViewingKey produces correct structure', async () => {
    const k   = await ZKELLAKeys.generate()
    const exp = k.exportViewingKey(12345678, 'testnet')
    expect(exp.version).toBe(1)
    expect(exp.network).toBe('testnet')
    expect(exp.birthday_ledger).toBe(12345678)
    expect(exp.viewing_key).toMatch(/^[0-9a-f]{64}$/)
    expect(exp.transmission_key).toMatch(/^[0-9a-f]{64}$/)
  })

})
