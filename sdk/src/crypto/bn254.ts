// Real BN254 (alt_bn128) G1 elliptic-curve operations, backing ZKELLA's
// transmission-key derivation and note-encryption ECDH.
//
// Uses `ffjavascript`'s `buildBn128()` — the same BN254 implementation
// `snarkjs` itself is built on (already a direct dependency here for
// `sdk/src/prover/*`), so this doesn't pull in a new curve library with its
// own, independently-reviewed arithmetic; it's the one already exercised by
// every Groth16 proof this SDK generates.
//
// Points are represented as 32-byte compressed G1 elements (matching the
// `Uint8Array // BN254 G1 point, compressed` convention already documented
// on `SpendingKey.transmissionKey`/`ShieldedAddress.pkD` in `sdk/src/types.ts`).
// Scalars are 32-byte little-endian buffers, matching every other field
// element in this SDK (`bufferToBigInt`/`bigIntToBuffer` in
// `sdk/src/crypto/poseidon.ts`) — reduced mod the BN254 scalar field order
// `r` before use, consistent with how every other raw 32-byte value in this
// protocol is canonicalized.

import { buildBn128 } from 'ffjavascript'
import { blake2b } from '@noble/hashes/blake2b'
import { bufferToBigInt, bigIntToBuffer } from './poseidon'

type Bn128 = Awaited<ReturnType<typeof buildBn128>>

let _bn128: Bn128 | null = null
async function getBn128(): Promise<Bn128> {
  if (!_bn128) {
    _bn128 = await buildBn128()
  }
  return _bn128
}

/** Compute `scalar * G` (the BN254 G1 generator), returning a 32-byte
 *  compressed point. Used to derive a transmission/ephemeral public key
 *  from a secret scalar. */
export async function scalarMultBase(scalar: Uint8Array): Promise<Uint8Array> {
  const bn128 = await getBn128()
  const s = bufferToBigInt(scalar) % bn128.Fr.p
  const point = bn128.G1.timesScalar(bn128.G1.g, s)
  const out = new Uint8Array(32)
  bn128.G1.toRprCompressed(out, 0, point)
  return out
}

/** Compute `scalar * point` for an arbitrary compressed G1 `point` — the
 *  core Diffie-Hellman operation: both sides compute the same result from
 *  their own secret scalar and the other side's public point
 *  (`ephemeralSk * (vk * G) === vk * (ephemeralSk * G)`). */
export async function scalarMultPoint(scalar: Uint8Array, point: Uint8Array): Promise<Uint8Array> {
  const bn128 = await getBn128()
  const s = bufferToBigInt(scalar) % bn128.Fr.p
  const p = bn128.G1.fromRprCompressed(point, 0)
  const result = bn128.G1.timesScalar(p, s)
  const out = new Uint8Array(32)
  bn128.G1.toRprCompressed(out, 0, result)
  return out
}

/** Generate a fresh random scalar, reduced mod the BN254 scalar field order. */
export async function randomScalar(): Promise<Uint8Array> {
  const bn128 = await getBn128()
  const raw = crypto.getRandomValues(new Uint8Array(32))
  const reduced = bufferToBigInt(raw) % bn128.Fr.p
  return bigIntToBuffer(reduced)
}

/**
 * Hash arbitrary bytes to a real point on BN254 G1, via the standard
 * try-and-increment construction: hash `seed || counter` to a candidate
 * x-coordinate, and accept it once `x^3 + 3` (G1's curve equation, `b = 3`,
 * `a = 0`) is a quadratic residue in the base field — at which point its
 * square root is a valid y-coordinate. Succeeds after ~2 iterations on
 * average (roughly half of all field elements are residues).
 *
 * Used to derive a per-diversifier base point `g_d` for diversified
 * shielded addresses (`ZKELLAKeys.deriveAddress`): `pk_d = vk * g_d` is a
 * real, one-way-derived public key rather than a hash of unrelated inputs,
 * and — because `g_d` (not the fixed generator) is the DH base point used
 * to encrypt to that specific address — the sender's `encryptNote` and the
 * recipient's `tryDecryptNote` still land on the identical shared point via
 * the same ECDH relation (`ephemeralSk*(vk*g_d) === vk*(ephemeralSk*g_d)`),
 * without the recipient needing to know which diversifier a note used.
 *
 * Returns a 32-byte compressed G1 point.
 */
export async function hashToCurveG1(seed: Uint8Array): Promise<Uint8Array> {
  const bn128 = await getBn128()
  const F1 = bn128.F1
  const b = F1.e(3n)

  for (let counter = 0; counter < 256; counter++) {
    const digest = blake2b(concatWithCounter(seed, counter), { dkLen: 32 })
    const xCandidate = bufferToBigInt(digest) % F1.p
    const x = F1.e(xCandidate)
    const y2 = F1.add(F1.mul(F1.mul(x, x), x), b)
    if (F1.isSquare(y2)) {
      const y = F1.sqrt(y2)
      const point = bn128.G1.fromObject([F1.toObject(x), F1.toObject(y), 1n])
      const out = new Uint8Array(32)
      bn128.G1.toRprCompressed(out, 0, point)
      return out
    }
  }
  // Astronomically unlikely (< 2^-256 chance of 256 consecutive
  // non-residues) — fail loudly rather than return a degenerate point.
  throw new Error('hashToCurveG1: failed to find a valid point after 256 attempts')
}

function concatWithCounter(seed: Uint8Array, counter: number): Uint8Array {
  const out = new Uint8Array(seed.length + 1)
  out.set(seed, 0)
  out[seed.length] = counter
  return out
}
