// Poseidon2 hash over the BN254 scalar field.
// Uses circomlibjs which provides the same implementation as circomlib circuits.
// This ensures SDK-computed commitments match on-chain (Soroban) commitments.

import { buildPoseidon } from 'circomlibjs'

// Singleton — building the Poseidon instance is async and expensive once
let _poseidon: Awaited<ReturnType<typeof buildPoseidon>> | null = null

async function getPoseidon() {
  if (!_poseidon) {
    _poseidon = await buildPoseidon()
  }
  return _poseidon
}

/**
 * Hash two 32-byte field elements using Poseidon2 (t=3, BN254 scalar field).
 * Returns a 32-byte field element.
 * Matches circomlib Poseidon(2) template output exactly.
 */
export async function poseidon2(a: Uint8Array, b: Uint8Array): Promise<Uint8Array> {
  if (a.length !== 32 || b.length !== 32) {
    throw new Error('poseidon2: inputs must be 32 bytes each')
  }
  const poseidon = await getPoseidon()

  // circomlibjs takes BigInt inputs
  const fa = bufferToBigInt(a)
  const fb = bufferToBigInt(b)

  const result = poseidon([fa, fb])
  return bigIntToBuffer(poseidon.F.toObject(result))
}

/**
 * Hash a single field element using Poseidon sponge (for nullifier derivation).
 * poseidon1([x]) = Poseidon([x, 0]) effectively.
 */
export async function poseidon1(a: Uint8Array): Promise<Uint8Array> {
  const poseidon = await getPoseidon()
  const fa = bufferToBigInt(a)
  const result = poseidon([fa])
  return bigIntToBuffer(poseidon.F.toObject(result))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

export function bufferToBigInt(buf: Uint8Array): bigint {
  let result = 0n
  for (let i = buf.length - 1; i >= 0; i--) {
    result = (result << 8n) | BigInt(buf[i])
  }
  return result
}

export function bigIntToBuffer(n: bigint): Uint8Array {
  const buf = new Uint8Array(32)
  let tmp = n
  for (let i = 0; i < 32; i++) {
    buf[i] = Number(tmp & 0xffn)
    tmp >>= 8n
  }
  return buf
}

/**
 * Encode a value (bigint) as a 32-byte little-endian field element.
 * Values must be < BN254 scalar field order.
 */
export function valueToField(value: bigint): Uint8Array {
  return bigIntToBuffer(value)
}

/**
 * Encode a Stellar contract address (StrKey G... / C...) as a 32-byte field element.
 * Uses the raw 28-byte binary payload of the StrKey, zero-padded to 32 bytes.
 */
export function addressToField(address: string): Uint8Array {
  // StrKey decode: base32 → remove checksum → extract payload
  const raw = strKeyDecode(address)
  const buf = new Uint8Array(32)
  buf.set(raw.slice(0, Math.min(raw.length, 32)))
  return buf
}

// Minimal StrKey decoder (base32, no external dependency)
function strKeyDecode(key: string): Uint8Array {
  const ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'
  const clean = key.toUpperCase().replace(/=+$/, '')
  let bits = 0
  let bitsCount = 0
  const bytes: number[] = []
  for (const c of clean) {
    const idx = ALPHABET.indexOf(c)
    if (idx === -1) continue
    bits = (bits << 5) | idx
    bitsCount += 5
    if (bitsCount >= 8) {
      bytes.push((bits >> (bitsCount - 8)) & 0xff)
      bitsCount -= 8
    }
  }
  // Skip version byte (first) and 2-byte checksum (last 2)
  return new Uint8Array(bytes.slice(1, bytes.length - 2))
}
