import * as snarkjs from 'snarkjs'
import { Note } from '../types'
import { computeCommitment, computeNullifier, computeValueCommit } from '../notes/builder'
import { addressToField, bufferToBigInt, bigIntToBuffer } from '../crypto/poseidon'
import { encodeProof } from './encoding'

const MERKLE_DEPTH = 32

/**
 * Public inputs/witness shapes for the 2-in-2-out transfer circuit.
 *
 * Circuit: circuits/transfer_2in2out/transfer.circom,
 *   `component main {public [anchor, nullifiers, out_commitments,
 *                             in_value_commits, out_value_commits, fee, asset_id]}`
 *   — arrays flatten in declaration order, so the 11 public signals are:
 *   [anchor, nullifiers[0], nullifiers[1], out_commitments[0], out_commitments[1],
 *    in_value_commits[0], in_value_commits[1], out_value_commits[0], out_value_commits[1],
 *    fee, asset_id]
 *
 * Invariants enforced by circuit (see transfer.circom):
 *   for each input i:  note commitment reconstructs, Merkle path reaches `anchor`,
 *                       nullifier == Poseidon2(nk, in_rho[i]), in_asset_id[i] == asset_id
 *   for each output i: out_commitments[i] == Poseidon2(Poseidon2(out_value[i], asset_id), Poseidon2(out_rho[i], out_rcm[i])),
 *                       out_asset_id[i] == asset_id
 *   in_value[0] + in_value[1] == out_value[0] + out_value[1] + fee
 *   nullifiers[0] != nullifiers[1], out_commitments[0] != out_commitments[1]
 *     (same-call double-spend / duplicate-commitment defense-in-depth —
 *     `contracts/token::transfer_internal` also checks this independently)
 */
export interface TransferInputNote {
  /** Must have a real on-chain `leafIndex` (the note being spent). */
  note: Note
  /** 32 sibling hashes for `note.leafIndex`, from `token.merkle_path(leafIndex)`. */
  merklePath: Uint8Array[]
}

export interface TransferOutputSpec {
  value:   bigint
  assetId: string
}

export interface TransferWitness {
  inputs:  [TransferInputNote, TransferInputNote]
  /** Spending key's nullifier key — shared across both inputs (one spender). */
  nk:      Uint8Array
  outputs: [TransferOutputSpec, TransferOutputSpec]
  fee:     bigint
}

export interface TransferPublicInputs {
  anchor:  Uint8Array  // current token.merkle_root(), as 32-byte LE
  assetId: string      // must match every input/output note's asset
}

export interface TransferProofResult {
  /** 256-byte Groth16 proof in the contract's wire format. */
  proof: Uint8Array
  nullifiers: [Uint8Array, Uint8Array]
  /**
   * Freshly-constructed output notes (fresh `rho`/`rcm`, real `commitment`,
   * `leafIndex: -1` until the contract call assigns real ones) — encrypt
   * these to their recipients and pass `commitment`/`encrypted_note` to
   * `transfer()`.
   */
  outputNotes: [Note, Note]
  inValueCommits:  [Uint8Array, Uint8Array]
  outValueCommits: [Uint8Array, Uint8Array]
  /**
   * Circuit's public signals as 32-byte LE field elements, in circuit order:
   * [anchor, nullifiers[0], nullifiers[1], out_commitments[0], out_commitments[1],
   *  in_value_commits[0], in_value_commits[1], out_value_commits[0], out_value_commits[1],
   *  fee, asset_id].
   */
  publicInputsLE: Uint8Array[]
}

/**
 * Generate a real Groth16 proof for the 2-in-2-out transfer circuit via
 * snarkjs, using the compiled artifacts at `wasmPath`/`zkeyPath` (typically
 * `circuits/transfer_2in2out/build/transfer_js/transfer.wasm` and
 * `circuits/transfer_2in2out/build/transfer.zkey`).
 */
export async function generateTransferProof(
  witness:      TransferWitness,
  publicInputs: TransferPublicInputs,
  wasmPath:     string,
  zkeyPath:     string,
): Promise<TransferProofResult> {
  for (const [i, input] of witness.inputs.entries()) {
    if (input.note.leafIndex < 0) {
      throw new Error(`transfer proof: inputs[${i}].note.leafIndex must be a real on-chain leaf index`)
    }
    if (input.merklePath.length !== MERKLE_DEPTH) {
      throw new Error(
        `transfer proof: inputs[${i}].merklePath must have exactly ${MERKLE_DEPTH} entries, got ${input.merklePath.length}`
      )
    }
    if (input.note.assetId !== publicInputs.assetId) {
      throw new Error(`transfer proof: inputs[${i}].note.assetId does not match publicInputs.assetId`)
    }
  }
  for (const [i, output] of witness.outputs.entries()) {
    if (output.assetId !== publicInputs.assetId) {
      throw new Error(`transfer proof: outputs[${i}].assetId does not match publicInputs.assetId`)
    }
  }

  const sumIn  = witness.inputs[0].note.value + witness.inputs[1].note.value
  const sumOut = witness.outputs[0].value + witness.outputs[1].value
  if (sumIn !== sumOut + witness.fee) {
    throw new Error(
      `transfer proof: in_value sum (${sumIn}) !== out_value sum (${sumOut}) + fee (${witness.fee})`
    )
  }

  const assetIdField = bufferToBigInt(addressToField(publicInputs.assetId)).toString()

  // ── Inputs: nullifiers + Merkle path bits ──────────────────────────────
  const nullifiers = await Promise.all(
    witness.inputs.map(inp => computeNullifier(witness.nk, inp.note.rho)),
  ) as [Uint8Array, Uint8Array]

  if (bufferToBigInt(nullifiers[0]) === bufferToBigInt(nullifiers[1])) {
    throw new Error('transfer proof: both input notes produced the same nullifier (same note spent twice)')
  }

  const inValueCommits: Uint8Array[] = []
  const inRcv: Uint8Array[] = []
  for (const inp of witness.inputs) {
    const rcv = crypto.getRandomValues(new Uint8Array(32))
    inRcv.push(rcv)
    inValueCommits.push(await computeValueCommit(inp.note.value, rcv))
  }

  // ── Outputs: fresh notes ────────────────────────────────────────────────
  const outputNotes: Note[] = []
  const outRcv: Uint8Array[] = []
  const outValueCommits: Uint8Array[] = []
  for (const out of witness.outputs) {
    const rho = crypto.getRandomValues(new Uint8Array(32))
    const rcm = crypto.getRandomValues(new Uint8Array(32))
    const rcv = crypto.getRandomValues(new Uint8Array(32))
    const commitment = await computeCommitment(out.value, out.assetId, rho, rcm)
    outputNotes.push({
      value: out.value,
      assetId: out.assetId,
      rho,
      rcm,
      leafIndex: -1,
      commitment,
    })
    outRcv.push(rcv)
    outValueCommits.push(await computeValueCommit(out.value, rcv))
  }

  if (bufferToBigInt(outputNotes[0].commitment) === bufferToBigInt(outputNotes[1].commitment)) {
    // Astronomically unlikely with fresh randomness, but the circuit itself
    // rejects it (see transfer.circom's cm_distinct check) — fail the same
    // way rather than build a witness that's guaranteed to be rejected.
    throw new Error('transfer proof: generated duplicate output commitments; retry')
  }

  const input = {
    in_value:      witness.inputs.map(i => i.note.value.toString()),
    in_asset_id:   witness.inputs.map(() => assetIdField),
    in_rho:        witness.inputs.map(i => bufferToBigInt(i.note.rho).toString()),
    in_rcm:        witness.inputs.map(i => bufferToBigInt(i.note.rcm).toString()),
    in_path:       witness.inputs.map(i => i.merklePath.map(p => bufferToBigInt(p).toString())),
    in_path_index: witness.inputs.map(i => pathIndicesFor(i.note.leafIndex).map(String)),
    in_rcv:        inRcv.map(r => bufferToBigInt(r).toString()),
    nk:            bufferToBigInt(witness.nk).toString(),

    out_value:    witness.outputs.map(o => o.value.toString()),
    out_asset_id: witness.outputs.map(() => assetIdField),
    out_rho:      outputNotes.map(n => bufferToBigInt(n.rho).toString()),
    out_rcm:      outputNotes.map(n => bufferToBigInt(n.rcm).toString()),
    out_rcv:      outRcv.map(r => bufferToBigInt(r).toString()),

    anchor:            bufferToBigInt(publicInputs.anchor).toString(),
    nullifiers:        nullifiers.map(n => bufferToBigInt(n).toString()),
    out_commitments:   outputNotes.map(n => bufferToBigInt(n.commitment).toString()),
    in_value_commits:  inValueCommits.map(c => bufferToBigInt(c).toString()),
    out_value_commits: outValueCommits.map(c => bufferToBigInt(c).toString()),
    fee:               witness.fee.toString(),
    asset_id:          assetIdField,
  }

  const { proof, publicSignals } = await snarkjs.groth16.fullProve(input, wasmPath, zkeyPath)

  return {
    proof: encodeProof(proof),
    nullifiers,
    outputNotes: outputNotes as [Note, Note],
    inValueCommits: inValueCommits as [Uint8Array, Uint8Array],
    outValueCommits: outValueCommits as [Uint8Array, Uint8Array],
    publicInputsLE: publicSignals.map((s: string) => bigIntToBuffer(BigInt(s))),
  }
}

/**
 * Direction bits for `leafIndex` (0 = left, 1 = right), one per Merkle
 * level — matches `contracts/token::merkle::get_path_indices` bit-for-bit.
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
