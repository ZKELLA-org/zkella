import { ZKELLAKeys } from '../../sdk/src/keys/keys'

const BN254_R = 21888242871839275222246405745257275088548364400416034343698204186575808495617n

describe('ZKELLAKeys', () => {

  test('generate produces different keys each time', () => {
    const k1 = ZKELLAKeys.generate()
    const k2 = ZKELLAKeys.generate()
    expect(k1.spendingKey.raw).not.toEqual(k2.spendingKey.raw)
  })

  test('fromSeed is deterministic', () => {
    const seed = new Uint8Array(32).fill(0x42)
    const k1   = ZKELLAKeys.fromSeed(seed)
    const k2   = ZKELLAKeys.fromSeed(seed)
    expect(k1.spendingKey.raw).toEqual(k2.spendingKey.raw)
    expect(k1.spendingKey.nullifierKey).toEqual(k2.spendingKey.nullifierKey)
    expect(k1.spendingKey.viewingKey).toEqual(k2.spendingKey.viewingKey)
  })

  test('all derived keys are different from each other', () => {
    const seed = new Uint8Array(32).fill(0x99)
    const k    = ZKELLAKeys.fromSeed(seed)
    expect(k.spendingKey.raw).not.toEqual(k.spendingKey.nullifierKey)
    expect(k.spendingKey.raw).not.toEqual(k.spendingKey.viewingKey)
    expect(k.spendingKey.nullifierKey).not.toEqual(k.spendingKey.viewingKey)
  })

  test('spending key is a valid BN254 scalar field element', () => {
    const seed = new Uint8Array(32).fill(0x01)
    const k    = ZKELLAKeys.fromSeed(seed)
    let n = 0n
    for (let i = 31; i >= 0; i--) n = (n << 8n) | BigInt(k.spendingKey.raw[i])
    expect(n).toBeGreaterThan(0n)
    expect(n).toBeLessThan(BN254_R)
  })

  test('fromSeed throws on wrong seed length', () => {
    expect(() => ZKELLAKeys.fromSeed(new Uint8Array(16))).toThrow('seed must be exactly 32 bytes')
  })

  test('deriveAddress produces a string starting with zkella1', () => {
    const k    = ZKELLAKeys.generate()
    const addr = k.deriveAddress(0)
    expect(addr.toString()).toMatch(/^zkella1/)
  })

  test('different diversifier indices produce different addresses', () => {
    const k    = ZKELLAKeys.generate()
    const a0   = k.deriveAddress(0)
    const a1   = k.deriveAddress(1)
    expect(a0.toString()).not.toBe(a1.toString())
  })

  test('exportViewingKey produces correct structure', () => {
    const k   = ZKELLAKeys.generate()
    const exp = k.exportViewingKey(12345678, 'testnet')
    expect(exp.version).toBe(1)
    expect(exp.network).toBe('testnet')
    expect(exp.birthday_ledger).toBe(12345678)
    expect(exp.viewing_key).toMatch(/^[0-9a-f]{64}$/)
    expect(exp.transmission_key).toMatch(/^[0-9a-f]{64}$/)
  })

})
