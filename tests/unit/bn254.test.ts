import { scalarMultBase, scalarMultPoint, randomScalar, hashToCurveG1 } from '../../sdk/src/crypto/bn254'
import { bufferToBigInt } from '../../sdk/src/crypto/poseidon'

function bytesToHex(buf: Uint8Array): string {
  return Array.from(buf).map(b => b.toString(16).padStart(2, '0')).join('')
}

describe('BN254 G1 curve operations', () => {
  test('scalarMultBase produces a 32-byte compressed point', async () => {
    const sk = await randomScalar()
    const pk = await scalarMultBase(sk)
    expect(pk.length).toBe(32)
  })

  test('scalarMultBase is deterministic for the same scalar', async () => {
    const sk = await randomScalar()
    const pk1 = await scalarMultBase(sk)
    const pk2 = await scalarMultBase(sk)
    expect(bytesToHex(pk1)).toBe(bytesToHex(pk2))
  })

  test('different scalars produce different points', async () => {
    const a = await randomScalar()
    const b = await randomScalar()
    expect(bufferToBigInt(a)).not.toBe(bufferToBigInt(b))
    const pa = await scalarMultBase(a)
    const pb = await scalarMultBase(b)
    expect(bytesToHex(pa)).not.toBe(bytesToHex(pb))
  })

  test('ECDH: a*(b*G) === b*(a*G) — both sides derive the same shared point', async () => {
    const a = await randomScalar()
    const b = await randomScalar()
    const aG = await scalarMultBase(a)
    const bG = await scalarMultBase(b)

    const sharedFromA = await scalarMultPoint(a, bG)
    const sharedFromB = await scalarMultPoint(b, aG)

    expect(bytesToHex(sharedFromA)).toBe(bytesToHex(sharedFromB))
  })

  test('ECDH shared secret differs for a different counterparty', async () => {
    const a = await randomScalar()
    const b = await randomScalar()
    const c = await randomScalar()
    const bG = await scalarMultBase(b)
    const cG = await scalarMultBase(c)

    const sharedWithB = await scalarMultPoint(a, bG)
    const sharedWithC = await scalarMultPoint(a, cG)

    expect(bytesToHex(sharedWithB)).not.toBe(bytesToHex(sharedWithC))
  })

  test('randomScalar values are reduced mod the BN254 scalar field order', async () => {
    const r = 21888242871839275222246405745257275088548364400416034343698204186575808495617n
    for (let i = 0; i < 20; i++) {
      const s = await randomScalar()
      expect(bufferToBigInt(s)).toBeLessThan(r)
    }
  })

  test('hashToCurveG1 is deterministic', async () => {
    const seed = new TextEncoder().encode('zkella-diversifier-1')
    const p1 = await hashToCurveG1(seed)
    const p2 = await hashToCurveG1(seed)
    expect(bytesToHex(p1)).toBe(bytesToHex(p2))
  })

  test('hashToCurveG1 produces different points for different seeds', async () => {
    const p1 = await hashToCurveG1(new TextEncoder().encode('diversifier-a'))
    const p2 = await hashToCurveG1(new TextEncoder().encode('diversifier-b'))
    expect(bytesToHex(p1)).not.toBe(bytesToHex(p2))
  })

  test('hashToCurveG1 output is a scalar-multipliable G1 point (ECDH works with it as the base)', async () => {
    const gd = await hashToCurveG1(new TextEncoder().encode('diversifier-x'))
    const vk = await randomScalar()
    const esk = await randomScalar()

    const pkD = await scalarMultPoint(vk, gd)       // recipient's diversified public key
    const epk = await scalarMultPoint(esk, gd)       // sender's ephemeral key, same base
    const sharedFromSender = await scalarMultPoint(esk, pkD)
    const sharedFromRecipient = await scalarMultPoint(vk, epk)

    expect(bytesToHex(sharedFromSender)).toBe(bytesToHex(sharedFromRecipient))
  })

  test('hashToCurveG1 for many seeds always terminates with a valid 32-byte point', async () => {
    for (let i = 0; i < 30; i++) {
      const seed = new TextEncoder().encode(`seed-${i}`)
      const p = await hashToCurveG1(seed)
      expect(p.length).toBe(32)
    }
  })
})
