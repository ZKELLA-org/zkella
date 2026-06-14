import { buildNote, computeCommitment, computeNullifier, verifyNoteIntegrity } from '../../sdk/src/notes/builder'
import vectors from '../../circuits/shield/shield_test_vectors.json'

const MOCK_ASSET = 'CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA' // USDC testnet

function hexToBytes(hex: string): Uint8Array {
  const buf = new Uint8Array(hex.length / 2)
  for (let i = 0; i < buf.length; i++) buf[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16)
  return buf
}

function bytesToHex(buf: Uint8Array): string {
  return Array.from(buf).map(b => b.toString(16).padStart(2, '0')).join('')
}

describe('Note commitment', () => {

  test('buildNote produces a note with a valid commitment', async () => {
    const note = await buildNote(100_000_000n, MOCK_ASSET)
    expect(note.value).toBe(100_000_000n)
    expect(note.assetId).toBe(MOCK_ASSET)
    expect(note.rho).toHaveLength(32)
    expect(note.rcm).toHaveLength(32)
    expect(note.commitment).toHaveLength(32)
    expect(note.leafIndex).toBe(-1)
  })

  test('commitment is deterministic for same inputs', async () => {
    const rho = new Uint8Array(32).fill(0xaa)
    const rcm = new Uint8Array(32).fill(0xbb)
    const c1 = await computeCommitment(50_000_000n, MOCK_ASSET, rho, rcm)
    const c2 = await computeCommitment(50_000_000n, MOCK_ASSET, rho, rcm)
    expect(c1).toEqual(c2)
  })

  test('commitment changes when value changes', async () => {
    const rho = new Uint8Array(32).fill(0x01)
    const rcm = new Uint8Array(32).fill(0x02)
    const c1 = await computeCommitment(100n, MOCK_ASSET, rho, rcm)
    const c2 = await computeCommitment(200n, MOCK_ASSET, rho, rcm)
    expect(c1).not.toEqual(c2)
  })

  test('commitment changes when asset changes', async () => {
    const rho = new Uint8Array(32).fill(0x03)
    const rcm = new Uint8Array(32).fill(0x04)
    const other = 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC'
    const c1 = await computeCommitment(100n, MOCK_ASSET, rho, rcm)
    const c2 = await computeCommitment(100n, other,      rho, rcm)
    expect(c1).not.toEqual(c2)
  })

  test('commitment changes when rho changes', async () => {
    const rcm  = new Uint8Array(32).fill(0x05)
    const rho1 = new Uint8Array(32).fill(0x06)
    const rho2 = new Uint8Array(32).fill(0x07)
    const c1 = await computeCommitment(100n, MOCK_ASSET, rho1, rcm)
    const c2 = await computeCommitment(100n, MOCK_ASSET, rho2, rcm)
    expect(c1).not.toEqual(c2)
  })

  test('verifyNoteIntegrity passes for a freshly built note', async () => {
    const note = await buildNote(999_000_000n, MOCK_ASSET)
    const valid = await verifyNoteIntegrity(note)
    expect(valid).toBe(true)
  })

  test('verifyNoteIntegrity fails when commitment is tampered', async () => {
    const note = await buildNote(999_000_000n, MOCK_ASSET)
    const tampered = { ...note, commitment: new Uint8Array(32).fill(0xff) }
    const valid = await verifyNoteIntegrity(tampered)
    expect(valid).toBe(false)
  })

})

describe('Shield test vectors (SDK matches circomlibjs reference)', () => {

  for (const vec of vectors.vectors) {
    test(`${vec.id}: ${vec.description}`, async () => {
      const value  = BigInt(vec.inputs.value)
      const asset  = vec.inputs.asset
      const rho    = hexToBytes(vec.inputs.rho)
      const rcm    = hexToBytes(vec.inputs.rcm)

      const commitment = await computeCommitment(value, asset, rho, rcm)
      expect(bytesToHex(commitment)).toBe(vec.expected.commitment)
    })
  }

})

describe('Nullifier', () => {

  test('nullifier is deterministic', async () => {
    const nk  = new Uint8Array(32).fill(0xcc)
    const rho = new Uint8Array(32).fill(0xdd)
    const nf1 = await computeNullifier(nk, rho)
    const nf2 = await computeNullifier(nk, rho)
    expect(nf1).toEqual(nf2)
  })

  test('nullifier changes with different nk', async () => {
    const rho = new Uint8Array(32).fill(0xee)
    const nk1 = new Uint8Array(32).fill(0x11)
    const nk2 = new Uint8Array(32).fill(0x22)
    const nf1 = await computeNullifier(nk1, rho)
    const nf2 = await computeNullifier(nk2, rho)
    expect(nf1).not.toEqual(nf2)
  })

  test('nullifier changes with different rho', async () => {
    const nk   = new Uint8Array(32).fill(0x33)
    const rho1 = new Uint8Array(32).fill(0x44)
    const rho2 = new Uint8Array(32).fill(0x55)
    const nf1  = await computeNullifier(nk, rho1)
    const nf2  = await computeNullifier(nk, rho2)
    expect(nf1).not.toEqual(nf2)
  })

})
