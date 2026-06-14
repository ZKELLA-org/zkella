import { poseidon2, valueToField, bigIntToBuffer, bufferToBigInt } from '../../sdk/src/crypto/poseidon'

jest.setTimeout(30_000)

describe('Poseidon2', () => {

  test('hash of two zero inputs is deterministic', async () => {
    const a = new Uint8Array(32)
    const b = new Uint8Array(32)
    const h1 = await poseidon2(a, b)
    const h2 = await poseidon2(a, b)
    expect(h1).toEqual(h2)
  })

  test('output is 32 bytes', async () => {
    const a = new Uint8Array(32).fill(1)
    const b = new Uint8Array(32).fill(2)
    const h = await poseidon2(a, b)
    expect(h).toHaveLength(32)
  })

  test('hash is not commutative — order matters', async () => {
    const a = new Uint8Array(32).fill(1)
    const b = new Uint8Array(32).fill(2)
    const h_ab = await poseidon2(a, b)
    const h_ba = await poseidon2(b, a)
    expect(h_ab).not.toEqual(h_ba)
  })

  test('output is a valid BN254 scalar field element', async () => {
    const BN254_R = 21888242871839275222246405745257275088548364400416034343698204186575808495617n
    const a = new Uint8Array(32).fill(3)
    const b = new Uint8Array(32).fill(4)
    const h = await poseidon2(a, b)
    const n = bufferToBigInt(h)
    expect(n).toBeGreaterThan(0n)
    expect(n).toBeLessThan(BN254_R)
  })

  test('valueToField encodes correctly', () => {
    const f = valueToField(100_000_000n)
    expect(f).toHaveLength(32)
    // First 8 bytes are the little-endian u64
    const lo = f[0] | (f[1] << 8) | (f[2] << 16) | (f[3] << 24)
    expect(lo).toBe(100_000_000)
  })

  test('bigIntToBuffer and bufferToBigInt round-trip', () => {
    const n = 12345678901234567890n
    const buf = bigIntToBuffer(n)
    const back = bufferToBigInt(buf)
    expect(back).toBe(n)
  })

})
