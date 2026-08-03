import * as snarkjs from 'snarkjs'
import { Note } from '../types'
import { computeNullifier } from '../notes/builder'
import { addressToField, bufferToBigInt, bigIntToBuffer, poseidon2 } from '../crypto/poseidon'
import { encodeProof } from './encoding'

const MERKLE_DEPTH = 32

/**
 * Public inputs for the unshield (withdraw) circuit.
 *
 * Circuit: unshield.circom, `component main {public [anchor, nullifier, pub_value, pub_asset_id, recipient_hash]}`
 *   Private: value, asset_id, rho, rcm, nk, path[32], path_index[32]
 *   Public:  anchor, nullifier, pub_value, pub_asset_id, recipient_hash
 *
 * `recipient_hash` isn't circuit-constrained (see unshield.circom's own
 * comment) — `contracts/token::unshield` checks it against `to` directly:
 * `recipient_hash === Poseidon2(address_field(to), 0)`. Get this wrong and
 * the contract call fails with `RecipientMismatch` before it ever reaches
 * proof verification, regardless of whether the proof itself is valid.
 */
export interface UnshieldWitness {
  note: Note          // the note being spent — must have a real leafIndex (>= 0)
  nk:   Uint8Array     // spending key's nullifier key (ZKELLAKeys.spendingKey.nullifierKey)
  /**
   * 32 sibling hashes for `note.leafIndex`, each a 32-byte little-endian
   * field element — from `token.merkle_path(leafIndex)` (a view call; there's
   * no indexer yet to source this from, see `docs/POC_IMPLEMENTATION.md`).
   * Path directions are *not* passed separately — they're `leafIndex`'s own
   * bits, computed here exactly like `contracts/token::merkle::get_path_indices`.
   */
  merklePath: Uint8Array[]
}

export interface UnshieldPublicInputs {
  anchor:    Uint8Array  // current token.merkle_root(), as 32-byte LE
  recipient: string      // Stellar address `to` — the withdrawal destination
}

export interface UnshieldProofResult {
  proof: Uint8Array
  /** Poseidon2(nk, note.rho) — pass this as the `nullifier` contract call arg. */
  nullifier: Uint8Array
  /** Poseidon2(address_field(recipient), 0) — pass this as `pub_inputs.recipient_hash`. */
  recipientHash: Uint8Array
  /**
   * Circuit's public signals as 32-byte LE field elements, in circuit order:
   * [anchor, nullifier, pub_value, pub_asset_id, recipient_hash].
   */
  publicInputsLE: Uint8Array[]
}

/**
 * Generate a real Groth16 proof for the unshield circuit via snarkjs, using
 * the compiled artifacts at `wasmPath`/`zkeyPath` (typically
 * `circuits/unshield/build/unshield_js/unshield.wasm` and
 * `circuits/unshield/build/unshield.zkey`). Same wire format as
 * `generateShieldProof` — see `sdk/src/prover/encoding.ts`.
 */
export async function generateUnshieldProof(
  witness:      UnshieldWitness,
  publicInputs: UnshieldPublicInputs,
  wasmPath:     string,
  zkeyPath:     string,
): Promise<UnshieldProofResult> {
  if (witness.note.leafIndex < 0) {
    throw new Error('unshield proof: note.leafIndex must be a real on-chain leaf index')
  }
  if (witness.merklePath.length !== MERKLE_DEPTH) {
    throw new Error(
      `unshield proof: merklePath must have exactly ${MERKLE_DEPTH} entries, got ${witness.merklePath.length}`
    )
  }

  const pathIndex = pathIndicesFor(witness.note.leafIndex)
  const nullifier = await computeNullifier(witness.nk, witness.note.rho)
  const recipientHash = await poseidon2(
    addressToField(publicInputs.recipient),
    new Uint8Array(32),
  )

  const assetIdField = bufferToBigInt(addressToField(witness.note.assetId)).toString()

  const input = {
    value:          witness.note.value.toString(),
    asset_id:       assetIdField,
    rho:            bufferToBigInt(witness.note.rho).toString(),
    rcm:            bufferToBigInt(witness.note.rcm).toString(),
    nk:             bufferToBigInt(witness.nk).toString(),
    path:           witness.merklePath.map(p => bufferToBigInt(p).toString()),
    path_index:     pathIndex.map(String),
    anchor:         bufferToBigInt(publicInputs.anchor).toString(),
    nullifier:      bufferToBigInt(nullifier).toString(),
    pub_value:      witness.note.value.toString(),
    pub_asset_id:   assetIdField,
    recipient_hash: bufferToBigInt(recipientHash).toString(),
  }

  const { proof, publicSignals } = await snarkjs.groth16.fullProve(input, wasmPath, zkeyPath)

  return {
    proof: encodeProof(proof),
    nullifier,
    recipientHash,
    publicInputsLE: publicSignals.map((s: string) => bigIntToBuffer(BigInt(s))),
  }
}

/**
 * Direction bits for `leafIndex` (0 = left, 1 = right), one per Merkle
 * level. Must match `contracts/token::merkle::get_path_indices` bit-for-bit:
 * bit `i` is `(leafIndex >> i) & 1`.
 */
function pathIndicesFor(leafIndex: number): number[] {
  const bits: number[] = []
  let idx = leafIndex
  for (let i = 0; i < MERKLE_DEPTH; i++) {
    bits.push(idx & 1)
    idx = Math.floor(idx / 2)
  }
  return bits
}
