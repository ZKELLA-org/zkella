import { poseidon2, valueToField, addressToField } from '../crypto/poseidon'
import { Note } from '../types'

/**
 * Construct a new note with cryptographically random rho and rcm.
 * The commitment is computed and stored alongside the plaintext.
 */
export async function buildNote(
  value:   bigint,
  assetId: string,  // SEP-41 contract address
): Promise<Note> {
  if (value <= 0n) throw new Error('note value must be positive')
  if (value >= 2n ** 64n) throw new Error('note value exceeds u64 max')

  // Generate fresh randomness — must use cryptographic RNG.
  // Both rho and rcm are BN254 Fr field elements; use full 32 bytes (256 bits)
  // to prevent statistical fingerprinting of notes with partially-random rcm.
  const rho  = crypto.getRandomValues(new Uint8Array(32))
  const rcm32 = crypto.getRandomValues(new Uint8Array(32))

  const commitment = await computeCommitment(value, assetId, rho as Uint8Array, rcm32)

  return {
    value,
    assetId,
    rho:        rho as Uint8Array,
    rcm:        rcm32,
    leafIndex:  -1,   // assigned on-chain after shield()
    commitment,
  }
}

/**
 * Compute note commitment: Poseidon2(Poseidon2(value, assetId), Poseidon2(rho, rcm))
 * Matches the on-chain compute_commitment() in contracts/ct20/src/lib.rs exactly.
 */
export async function computeCommitment(
  value:   bigint,
  assetId: string,
  rho:     Uint8Array,
  rcm:     Uint8Array,
): Promise<Uint8Array> {
  const valueField  = valueToField(value)
  const assetField  = addressToField(assetId)
  const h1          = await poseidon2(valueField, assetField)
  const h2          = await poseidon2(rho, rcm)
  return poseidon2(h1, h2)
}

/**
 * Compute the nullifier for a note given the nullifier key.
 * nf = Poseidon2(nk, rho)
 */
export async function computeNullifier(
  nk:  Uint8Array,
  rho: Uint8Array,
): Promise<Uint8Array> {
  return poseidon2(nk, rho)
}

/**
 * Verify that a note's commitment matches its plaintext fields.
 * Used after syncing from the indexer to detect tampering.
 */
export async function verifyNoteIntegrity(note: Note): Promise<boolean> {
  const expected = await computeCommitment(
    note.value,
    note.assetId,
    note.rho,
    note.rcm,
  )
  for (let i = 0; i < 32; i++) {
    if (expected[i] !== note.commitment[i]) return false
  }
  return true
}
