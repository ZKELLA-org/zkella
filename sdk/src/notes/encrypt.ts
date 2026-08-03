import { chacha20poly1305 } from '@noble/ciphers/chacha'
import { blake2b }           from '@noble/hashes/blake2b'
import { Note }              from '../types'
import { scalarMultBase, scalarMultPoint, randomScalar } from '../crypto/bn254'

// ── Transmitted note layout (160 bytes) ──────────────────────────────────────
//
//  ephemeral_pk  : 32 bytes  — compressed BN254 G1 point (sender's ephemeral key)
//  ciphertext    : 128 bytes — ChaCha20-Poly1305( 112-byte plaintext ) + 16-byte MAC
//
// Plaintext layout (128 bytes):
//  value    : 8 bytes  (u64, little-endian)
//  assetId  : 56 bytes (UTF-8 zero-padded Stellar StrKey address, always 56 chars)
//  rho      : 32 bytes
//  rcm      : 32 bytes (field element; upper 16 bytes are zero from builder)

const ASSET_ID_LENGTH = 56  // Stellar C.../G... StrKey addresses are exactly 56 chars
const PLAINTEXT_LENGTH = 8 + ASSET_ID_LENGTH + 32 + 32  // = 128 bytes
const CIPHERTEXT_LENGTH = PLAINTEXT_LENGTH + 16          // + Poly1305 MAC = 144 bytes
const ENCRYPTED_NOTE_LENGTH = 32 + CIPHERTEXT_LENGTH     // = 176 bytes

/**
 * Encrypt a note to a recipient's transmission key.
 *
 * Real BN254 ECDH: `transmissionKey` is `vk * basePoint` — either the fixed
 * generator `G` (`ZKELLAKeys.fromSeed`'s top-level `transmissionKey`) or a
 * per-diversifier point `g_d = hashToCurveG1(diversifier)`
 * (`ZKELLAKeys.deriveAddress`'s `pk_d`). The shared secret is the point
 * `ephemeralSk * transmissionKey === vk * ephemeralPk`, computed against
 * the *same* `basePoint` the recipient's address embeds — pass
 * `recipientAddress.diversifier`'s derived `g_d` (via `hashToCurveG1`) when
 * encrypting to a diversified address, or omit `basePoint` to encrypt to a
 * wallet's non-diversified `transmissionKey` (the default, and what every
 * existing caller — including the real Testnet shield flow — already
 * does). `tryDecryptNote` needs no corresponding parameter: `vk * ephemeralPk`
 * lands on the same shared point regardless of which base point the sender
 * used, since `ephemeralPk` already encodes it (`ephemeralPk = esk * g_d`).
 * Hashing the shared *point* (not just concatenating raw coordinates)
 * before using it as key material is standard ECIES practice.
 */
export async function encryptNote(
  note:             Note,
  transmissionKey:  Uint8Array,  // 32-byte compressed BN254 G1 point
  basePoint?:       Uint8Array,  // 32-byte compressed G1 point; defaults to the generator G
): Promise<Uint8Array> {
  const ephemeralSk = await randomScalar()
  const ephemeralPk = basePoint
    ? await scalarMultPoint(ephemeralSk, basePoint)
    : await scalarMultBase(ephemeralSk)
  const sharedPoint = await scalarMultPoint(ephemeralSk, transmissionKey)

  const sharedSecret = blake2b(sharedPoint, { dkLen: 32 })

  // Derive encryption key and nonce from shared secret
  const keyMaterial = blake2b(
    concat(sharedSecret, ephemeralPk),
    { dkLen: 44 },
  )
  const encKey   = keyMaterial.slice(0, 32)
  const nonce    = keyMaterial.slice(32, 44)

  // Encode plaintext
  const plaintext = encodePlaintext(note)

  // Encrypt with ChaCha20-Poly1305
  const cipher     = chacha20poly1305(encKey, nonce)
  const ciphertext = cipher.encrypt(plaintext)

  // Bundle: ephemeral_pk || ciphertext
  const bundle = new Uint8Array(ENCRYPTED_NOTE_LENGTH)
  bundle.set(ephemeralPk,  0)
  bundle.set(ciphertext,  32)
  return bundle
}

/**
 * Attempt to decrypt an encrypted note bundle using a viewing key.
 * Returns the decrypted Note or null if decryption fails (not addressed to us).
 */
export async function tryDecryptNote(
  bundle:     Uint8Array,
  viewingKey: Uint8Array,  // 32-byte viewing key scalar (ZKELLAKeys.spendingKey.viewingKey)
): Promise<Omit<Note, 'leafIndex' | 'commitment'> | null> {
  if (bundle.length !== ENCRYPTED_NOTE_LENGTH) return null

  const ephemeralPk = bundle.slice(0, 32)
  const ciphertext  = bundle.slice(32)

  // Reconstruct the same shared point as encryptNote: vk * ephemeralPk === ephemeralSk * (vk*G).
  const sharedPoint = await scalarMultPoint(viewingKey, ephemeralPk)
  const sharedSecret = blake2b(sharedPoint, { dkLen: 32 })

  const keyMaterial = blake2b(
    concat(sharedSecret, ephemeralPk),
    { dkLen: 44 },
  )
  const encKey = keyMaterial.slice(0, 32)
  const nonce  = keyMaterial.slice(32, 44)

  try {
    const cipher    = chacha20poly1305(encKey, nonce)
    const plaintext = cipher.decrypt(ciphertext)
    return decodePlaintext(plaintext)
  } catch {
    // MAC verification failed — note is not addressed to this key
    return null
  }
}

// ── Encoding helpers ──────────────────────────────────────────────────────────

function encodePlaintext(note: Note): Uint8Array {
  const buf = new Uint8Array(PLAINTEXT_LENGTH)
  const dv  = new DataView(buf.buffer)

  // value: 8 bytes, little-endian u64
  const lo = Number(note.value & 0xffffffffn)
  const hi = Number(note.value >> 32n)
  dv.setUint32(0, lo, true)
  dv.setUint32(4, hi, true)

  // assetId: 56 bytes, UTF-8 zero-padded (Stellar StrKey is exactly 56 ASCII chars)
  const assetBytes = new TextEncoder().encode(note.assetId)
  buf.set(assetBytes.slice(0, ASSET_ID_LENGTH), 8)

  // rho: 32 bytes  (offset = 8 + 56 = 64)
  buf.set(note.rho.slice(0, 32), 8 + ASSET_ID_LENGTH)

  // rcm: 32 bytes  (offset = 8 + 56 + 32 = 96)
  buf.set(note.rcm.slice(0, 32), 8 + ASSET_ID_LENGTH + 32)

  return buf
}

function decodePlaintext(buf: Uint8Array): Omit<Note, 'leafIndex' | 'commitment'> {
  if (buf.length !== PLAINTEXT_LENGTH) throw new Error('invalid plaintext length')

  const dv = new DataView(buf.buffer, buf.byteOffset)

  const lo    = dv.getUint32(0, true)
  const hi    = dv.getUint32(4, true)
  const value = (BigInt(hi) << 32n) | BigInt(lo)

  const assetBytes = buf.slice(8, 8 + ASSET_ID_LENGTH)
  const assetId    = new TextDecoder().decode(assetBytes).replace(/\0+$/, '')

  const rho = buf.slice(8 + ASSET_ID_LENGTH, 8 + ASSET_ID_LENGTH + 32)
  const rcm = new Uint8Array(buf.slice(8 + ASSET_ID_LENGTH + 32, PLAINTEXT_LENGTH))

  return { value, assetId, rho, rcm }
}

// ── Internal ──────────────────────────────────────────────────────────────────

function concat(...arrays: Uint8Array[]): Uint8Array {
  const total = arrays.reduce((sum, a) => sum + a.length, 0)
  const result = new Uint8Array(total)
  let offset = 0
  for (const a of arrays) {
    result.set(a, offset)
    offset += a.length
  }
  return result
}
