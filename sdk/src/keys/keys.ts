import { blake2b }           from '@noble/hashes/blake2b'
import { poseidon2, bigIntToBuffer, bufferToBigInt } from '../crypto/poseidon'
import { scalarMultBase, scalarMultPoint, hashToCurveG1 } from '../crypto/bn254'
import { SpendingKey, ViewingKey, ShieldedAddress, ViewingKeyExport } from '../types'

// BN254 scalar field order
const BN254_R =
  21888242871839275222246405745257275088548364400416034343698204186575808495617n

// Domain separators
const DOMAIN_SPEND   = new TextEncoder().encode('zkella_spend_v1')
const DOMAIN_NULLIFY = new TextEncoder().encode('zkella_nullifier_v1')
const DOMAIN_VIEW    = new TextEncoder().encode('zkella_viewing_v1')

export class ZKELLAKeys {

  private constructor(public readonly spendingKey: SpendingKey) {}

  // ── Constructors ────────────────────────────────────────────────────────────

  /**
   * Generate a new random wallet.
   * Seed is 32 cryptographically random bytes.
   */
  static async generate(): Promise<ZKELLAKeys> {
    const seed = crypto.getRandomValues(new Uint8Array(32))
    return ZKELLAKeys.fromSeed(seed)
  }

  /**
   * Derive a deterministic wallet from a 32-byte seed.
   * Key hierarchy:
   *   sk  = BLAKE2b-256(seed || "zkella_spend_v1")     mod r
   *   nk  = BLAKE2b-256(sk   || "zkella_nullifier_v1") mod r
   *   vk  = BLAKE2b-256(sk   || "zkella_viewing_v1")   mod r
   *   tk  = vk * G  (real BN254 G1 scalar multiplication)
   *
   * `tk` is derived from `vk`, not `sk`: `sdk/src/notes/encrypt.ts`'s
   * `tryDecryptNote` takes a viewing key (not the full spending key) as its
   * decryption secret — auditor viewing keys, without spend authority, are
   * a core ZKELLA feature (see README's compliance positioning) — so the
   * public transmission key embedded in a shielded address must be reachable
   * from `vk` alone via the matching ECDH relation
   * `ephemeralSk * (vk*G) === vk * (ephemeralSk*G)`. Previously `tk` was
   * literally set to `vk`'s raw bytes rather than `vk*G`, which leaked the
   * viewing key itself to anyone who saw a shielded address; `vk*G` is a
   * one-way function of `vk`; unlike the old raw-bytes stub.
   */
  static fromSpendingKey(sk: SpendingKey): ZKELLAKeys {
    return new ZKELLAKeys(sk)
  }

  static async fromSeed(seed: Uint8Array): Promise<ZKELLAKeys> {
    if (seed.length !== 32) throw new Error('seed must be exactly 32 bytes')

    const skRaw = blake2b(concat(seed, DOMAIN_SPEND), { dkLen: 32 })
    const sk    = reduceModR(skRaw)

    const nkRaw = blake2b(concat(sk, DOMAIN_NULLIFY), { dkLen: 32 })
    const nk    = reduceModR(nkRaw)

    const vkRaw = blake2b(concat(sk, DOMAIN_VIEW), { dkLen: 32 })
    const vk    = reduceModR(vkRaw)

    const tk = await scalarMultBase(vk)

    const spendingKey: SpendingKey = {
      raw:             sk,
      nullifierKey:    nk,
      viewingKey:      vk,
      transmissionKey: tk,
    }
    return new ZKELLAKeys(spendingKey)
  }

  // ── Address derivation ──────────────────────────────────────────────────────

  /**
   * Derive a diversified shielded address.
   * Multiple addresses share one spending key; all are unlinkable on-chain.
   *
   * diversifier   = BLAKE2b-32(sk || index)
   * g_d           = hash_to_curve(diversifier)     (real BN254 G1 point)
   * pk_d          = vk * g_d                       (real scalar multiplication)
   * address       = Base58Check(1-byte-version || diversifier || pk_d)
   *
   * `pk_d` is derived from `vk` (the viewing key), not `sk`, matching
   * `transmissionKey`'s same reasoning (see `fromSeed`'s doc comment):
   * decryption only needs the viewing key, not full spend authority. `g_d`
   * doubles as the Diffie-Hellman base point a sender must use when
   * encrypting *to this specific diversified address* — see
   * `sdk/src/notes/encrypt.ts`'s `basePoint` parameter — so that
   * `ephemeralSk * pk_d === vk * ephemeralPk` still holds without the
   * recipient needing to know which diversifier a given note used.
   */
  async deriveAddress(diversifierIndex = 0): Promise<ShieldedAddress> {
    const indexBuf = new Uint8Array(4)
    new DataView(indexBuf.buffer).setUint32(0, diversifierIndex, true)

    const diversifier = blake2b(
      concat(this.spendingKey.raw, indexBuf),
      { dkLen: 11 },
    )

    const gD = await hashToCurveG1(diversifier)
    const pkD = await scalarMultPoint(this.spendingKey.viewingKey, gD)

    const raw  = concat(new Uint8Array([0x01]), diversifier, pkD)  // 1+11+32=44 bytes
    const addr = base58Check(raw)

    return {
      diversifier,
      pkD,
      toString: () => 'zkella1' + addr,
    }
  }

  // ── Export ───────────────────────────────────────────────────────────────────

  exportViewingKey(birthdayLedger: number, network: string): ViewingKeyExport {
    return {
      version:          1,
      network,
      viewing_key:      toHex(this.spendingKey.viewingKey),
      transmission_key: toHex(this.spendingKey.transmissionKey),
      birthday_ledger:  birthdayLedger,
    }
  }

  toViewingKey(birthdayLedger: number): ViewingKey {
    return {
      raw:             this.spendingKey.viewingKey,
      transmissionKey: this.spendingKey.transmissionKey,
      birthdayLedger,
    }
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/** Reduce a 32-byte value modulo the BN254 scalar field order r. */
function reduceModR(bytes: Uint8Array): Uint8Array {
  const n = bufferToBigInt(bytes) % BN254_R
  return bigIntToBuffer(n)
}

function concat(...arrays: Uint8Array[]): Uint8Array {
  const total  = arrays.reduce((s, a) => s + a.length, 0)
  const result = new Uint8Array(total)
  let offset   = 0
  for (const a of arrays) { result.set(a, offset); offset += a.length }
  return result
}

function toHex(buf: Uint8Array): string {
  return Array.from(buf).map(b => b.toString(16).padStart(2, '0')).join('')
}

const BASE58_ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'

// WARNING: no checksum — address typos produce a silently wrong address,
// not an error. A SHA256d 4-byte checksum will be added in M2.
// Until then, always verify shielded addresses out-of-band before sending funds.
function base58Check(payload: Uint8Array): string {
  let n = bufferToBigInt(payload)
  let result = ''
  while (n > 0n) {
    result = BASE58_ALPHABET[Number(n % 58n)] + result
    n = n / 58n
  }
  for (const byte of payload) {
    if (byte !== 0) break
    result = '1' + result
  }
  return result
}
