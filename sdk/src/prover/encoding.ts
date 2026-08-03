/**
 * Groth16 proof/verifying-key wire-format encoding shared by every circuit's
 * prover (`shield.ts`, `unshield.ts`, ...) — circuit-agnostic, since the
 * point encoding is the same regardless of which circuit produced the proof.
 *
 * Point encoding matches Soroban's native `crypto::bn254` host types exactly
 * (see `contracts/verifier`'s module doc): each coordinate is 32
 * **big-endian** bytes, and a G2 point's coordinates are themselves `Fp2`
 * elements encoded `be(c1) || be(c0)` — snarkjs stores each `Fp2` coordinate
 * as `[c0, c1]`, so this swaps order on the way out. Cross-validated against
 * `circuits/shield/build/convert_to_wire_format.py` in `tests/unit/prover.test.ts`,
 * which asserts byte-identical output to the exact proof/VK submitted in a
 * real, successful Stellar Testnet `shield()` transaction (see
 * `docs/POC_IMPLEMENTATION.md`, "Update: live Testnet run completed").
 */

/**
 * Encode a snarkjs Groth16 proof object into the contract's flat 256-byte
 * wire format: A (64B, G1) || B (128B, G2) || C (64B, G1).
 */
export function encodeProof(snarkjsProof: {
  pi_a: string[]
  pi_b: string[][]
  pi_c: string[]
}): Uint8Array {
  const buf = new Uint8Array(256)
  let off = 0

  const writeG1 = (pt: string[]) => {
    writeFieldBE(buf, off,      BigInt(pt[0]))
    writeFieldBE(buf, off + 32, BigInt(pt[1]))
    off += 64
  }

  const writeG2 = (pt: string[][]) => {
    // snarkjs: pt = [[x_c0, x_c1], [y_c0, y_c1], ["1","0"]] — swap to c1||c0.
    writeFieldBE(buf, off,      BigInt(pt[0][1])) // x_c1
    writeFieldBE(buf, off + 32, BigInt(pt[0][0])) // x_c0
    writeFieldBE(buf, off + 64, BigInt(pt[1][1])) // y_c1
    writeFieldBE(buf, off + 96, BigInt(pt[1][0])) // y_c0
    off += 128
  }

  writeG1(snarkjsProof.pi_a)
  writeG2(snarkjsProof.pi_b)
  writeG1(snarkjsProof.pi_c)

  return buf
}

/**
 * Encode a snarkjs `verification_key.json` object into the contract's wire
 * format: alpha_g1(64) || beta_g2(128) || gamma_g2(128) || delta_g2(128) ||
 * IC[0](64) || IC[1](64) || ... — what `contracts/verifier`'s
 * `register_verifying_key`/`update_verifying_key` expect. Same point
 * encoding as `encodeProof`.
 */
export function encodeVerifyingKey(vk: {
  protocol: string
  curve:    string
  vk_alpha_1: string[]
  vk_beta_2:  string[][]
  vk_gamma_2: string[][]
  vk_delta_2: string[][]
  IC:         string[][]
}): Uint8Array {
  if (vk.protocol !== 'groth16') throw new Error(`unsupported protocol: ${vk.protocol}`)
  if (vk.curve !== 'bn128') throw new Error(`unsupported curve: ${vk.curve}`)

  const g1 = (pt: string[]) => {
    const out = new Uint8Array(64)
    writeFieldBE(out, 0,  BigInt(pt[0]))
    writeFieldBE(out, 32, BigInt(pt[1]))
    return out
  }
  const g2 = (pt: string[][]) => {
    const out = new Uint8Array(128)
    writeFieldBE(out, 0,   BigInt(pt[0][1])) // x_c1
    writeFieldBE(out, 32,  BigInt(pt[0][0])) // x_c0
    writeFieldBE(out, 64,  BigInt(pt[1][1])) // y_c1
    writeFieldBE(out, 96,  BigInt(pt[1][0])) // y_c0
    return out
  }

  const parts: Uint8Array[] = [
    g1(vk.vk_alpha_1),
    g2(vk.vk_beta_2),
    g2(vk.vk_gamma_2),
    g2(vk.vk_delta_2),
    ...vk.IC.map(g1),
  ]

  const total = parts.reduce((n, p) => n + p.length, 0)
  const out = new Uint8Array(total)
  let off = 0
  for (const p of parts) {
    out.set(p, off)
    off += p.length
  }
  return out
}

export function writeFieldBE(buf: Uint8Array, offset: number, value: bigint): void {
  for (let i = 31; i >= 0; i--) {
    buf[offset + i] = Number(value & 0xffn)
    value >>= 8n
  }
}
