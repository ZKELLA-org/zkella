import { Note } from '../types'
import { addressToField, bufferToBigInt } from '../crypto/poseidon'

/**
 * Public inputs for the shield (deposit) circuit.
 *
 * Circuit: shield.circom
 *   Private: value, assetId, rho, rcm
 *   Public:  commitment, asset, amount
 *
 * Invariant enforced by circuit:
 *   commitment == Poseidon2(Poseidon2(value, assetId), Poseidon2(rho, rcm))
 *   value      == amount  (prevents value inflation)
 */
export interface ShieldPublicInputs {
  commitment: string  // hex-encoded BN254 Fr field element
  asset:      string  // SEP-41 contract address
  amount:     bigint  // u64 in base units
}

export interface ShieldProofResult {
  proof:         Uint8Array   // 256-byte Groth16 proof (π_A || π_B || π_C)
  publicSignals: string[]     // [commitment, asset_field, amount] as decimal strings
}

/**
 * Generate a Groth16 proof for the shield circuit.
 *
 * In M1 this is a stub that returns a zero-filled proof byte array.
 * The on-chain verifier is also a stub (accepts any proof whose public inputs
 * match the commitment stored in the Merkle tree).
 *
 * M2 replaces this with:
 *   const { proof, publicSignals } = await snarkjs.groth16.fullProve(
 *     witness, WASM_PATH, ZKEY_PATH
 *   )
 *   return { proof: encodeProof(proof), publicSignals }
 */
export async function generateShieldProof(
  note:         Note,
  publicInputs: ShieldPublicInputs,
): Promise<ShieldProofResult> {
  // Validate public input consistency before proof generation
  if (note.value !== publicInputs.amount) {
    throw new Error(
      `shield proof: note.value (${note.value}) !== amount (${publicInputs.amount})`
    )
  }
  if (note.assetId !== publicInputs.asset) {
    throw new Error(
      `shield proof: note.assetId (${note.assetId}) !== asset (${publicInputs.asset})`
    )
  }

  // TODO(M2): load compiled circuit artifacts and call snarkjs
  // const wasm = await loadWasm(SHIELD_WASM_URL)
  // const zkey = await loadZkey(SHIELD_ZKEY_URL)
  // const witness = {
  //   value:      note.value.toString(),
  //   assetId:    fieldFromAddress(note.assetId).toString(),
  //   rho:        bufferToBigInt(note.rho).toString(),
  //   rcm:        bufferToBigInt(note.rcm).toString(),
  //   commitment: BigInt('0x' + publicInputs.commitment).toString(),
  // }
  // const { proof, publicSignals } = await snarkjs.groth16.fullProve(witness, wasm, zkey)
  // return { proof: encodeProof(proof), publicSignals }

  // Stub: 256 zero bytes (π_A: 64B, π_B: 128B, π_C: 64B)
  const proof = new Uint8Array(256)

  const publicSignals = [
    publicInputs.commitment,
    bufferToBigInt(addressToField(publicInputs.asset)).toString(),
    publicInputs.amount.toString(),
  ]

  return { proof, publicSignals }
}

/**
 * Encode a snarkjs proof object into a flat 256-byte array.
 * Layout: π_A (64B, G1 uncompressed) || π_B (128B, G2 uncompressed) || π_C (64B, G1 uncompressed)
 * Used in M2 when snarkjs returns the real proof.
 */
export function encodeProof(snarkjsProof: {
  pi_a: string[]
  pi_b: string[][]
  pi_c: string[]
}): Uint8Array {
  const buf = new Uint8Array(256)
  let off = 0

  const writeG1 = (pt: string[]) => {
    writeField(buf, off,      BigInt(pt[0]))
    writeField(buf, off + 32, BigInt(pt[1]))
    off += 64
  }

  const writeG2 = (pt: string[][]) => {
    // G2 point: (x0, x1, y0, y1) each 32 bytes
    writeField(buf, off,       BigInt(pt[0][0]))
    writeField(buf, off + 32,  BigInt(pt[0][1]))
    writeField(buf, off + 64,  BigInt(pt[1][0]))
    writeField(buf, off + 96,  BigInt(pt[1][1]))
    off += 128
  }

  writeG1(snarkjsProof.pi_a)
  writeG2(snarkjsProof.pi_b)
  writeG1(snarkjsProof.pi_c)

  return buf
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function writeField(buf: Uint8Array, offset: number, value: bigint): void {
  for (let i = 0; i < 32; i++) {
    buf[offset + i] = Number(value & 0xffn)
    value >>= 8n
  }
}
