import * as snarkjs from 'snarkjs'
import { Note } from '../types'
import { computeCommitment, computeNullifier, computeValueCommit } from '../notes/builder'
import { addressToField, bufferToBigInt, bigIntToBuffer } from '../crypto/poseidon'
import { encodeProof } from './encoding'

const MERKLE_DEPTH = 32
const N = 4

/**
 * Public inputs/witness shapes for the 4-in-4-out transfer circuit.
 * Same structure as `sdk/src/prover/transfer.ts`'s 2-in-2-out prover,
 * generalized to 4 input/output notes — see that module's doc comment for
 * the invariants, which are identical here just with `N_IN = N_OUT = 4`.
 *
 * Circuit: circuits/transfer_4in4out/transfer.circom,
 *   `component main {public [anchor, nullifiers, out_commitments,
 *                             in_value_commits, out_value_commits, fee, asset_id]}`
 *   — 19 public signals: [anchor, nullifiers[0..4], out_commitments[0..4],
 *     in_value_commits[0..4], out_value_commits[0..4], fee, asset_id].
 */
export interface Transfer4InputNote {
  /** Must have a real on-chain `leafIndex` (the note being spent). */
  note: Note
  /** 32 sibling hashes for `note.leafIndex`, from `token.merkle_path(leafIndex)`. */
  merklePath: Uint8Array[]
}

export interface Transfer4OutputSpec {
  value:   bigint
  assetId: string
}

export interface Transfer4Witness {
  inputs:  [Transfer4InputNote, Transfer4InputNote, Transfer4InputNote, Transfer4InputNote]
  /** Spending key's nullifier key — shared across all four inputs (one spender). */
  nk:      Uint8Array
  outputs: [Transfer4OutputSpec, Transfer4OutputSpec, Transfer4OutputSpec, Transfer4OutputSpec]
  fee:     bigint
}

export interface Transfer4PublicInputs {
  anchor:  Uint8Array  // current token.merkle_root(), as 32-byte LE
  assetId: string      // must match every input/output note's asset
}

export interface Transfer4ProofResult {
  /** 256-byte Groth16 proof in the contract's wire format. */
  proof: Uint8Array
  nullifiers: [Uint8Array, Uint8Array, Uint8Array, Uint8Array]
  /** Freshly-constructed output notes — encrypt to recipients and pass
   *  `commitment`/`encrypted_note` to `transfer4()`. */
  outputNotes: [Note, Note, Note, Note]
  inValueCommits:  [Uint8Array, Uint8Array, Uint8Array, Uint8Array]
  outValueCommits: [Uint8Array, Uint8Array, Uint8Array, Uint8Array]
  /**
   * Circuit's public signals as 32-byte LE field elements, in circuit order:
   * [anchor, nullifiers[0..4], out_commitments[0..4], in_value_commits[0..4],
   *  out_value_commits[0..4], fee, asset_id].
   */
  publicInputsLE: Uint8Array[]
}

/**
 * Generate a real Groth16 proof for the 4-in-4-out transfer circuit via
 * snarkjs, using the compiled artifacts at `wasmPath`/`zkeyPath` (typically
 * `circuits/transfer_4in4out/build/transfer_js/transfer.wasm` and
 * `circuits/transfer_4in4out/build/transfer.zkey`).
 */
export async function generateTransfer4Proof(
  witness:      Transfer4Witness,
  publicInputs: Transfer4PublicInputs,
  wasmPath:     string,
  zkeyPath:     string,
): Promise<Transfer4ProofResult> {
  for (const [i, input] of witness.inputs.entries()) {
    if (input.note.leafIndex < 0) {
      throw new Error(`transfer4 proof: inputs[${i}].note.leafIndex must be a real on-chain leaf index`)
    }
    if (input.merklePath.length !== MERKLE_DEPTH) {
      throw new Error(
        `transfer4 proof: inputs[${i}].merklePath must have exactly ${MERKLE_DEPTH} entries, got ${input.merklePath.length}`
      )
    }
    if (input.note.assetId !== publicInputs.assetId) {
      throw new Error(`transfer4 proof: inputs[${i}].note.assetId does not match publicInputs.assetId`)
    }
  }
  for (const [i, output] of witness.outputs.entries()) {
    if (output.assetId !== publicInputs.assetId) {
      throw new Error(`transfer4 proof: outputs[${i}].assetId does not match publicInputs.assetId`)
    }
  }

  const sumIn  = witness.inputs.reduce((s, i) => s + i.note.value, 0n)
  const sumOut = witness.outputs.reduce((s, o) => s + o.value, 0n)
  if (sumIn !== sumOut + witness.fee) {
    throw new Error(
      `transfer4 proof: in_value sum (${sumIn}) !== out_value sum (${sumOut}) + fee (${witness.fee})`
    )
  }

  const assetIdField = bufferToBigInt(addressToField(publicInputs.assetId)).toString()

  // ── Inputs: nullifiers + Merkle path bits ──────────────────────────────
  const nullifiers = await Promise.all(
    witness.inputs.map(inp => computeNullifier(witness.nk, inp.note.rho)),
  ) as [Uint8Array, Uint8Array, Uint8Array, Uint8Array]

  for (let i = 0; i < N; i++) {
    for (let j = i + 1; j < N; j++) {
      if (bufferToBigInt(nullifiers[i]) === bufferToBigInt(nullifiers[j])) {
        throw new Error(`transfer4 proof: inputs[${i}] and inputs[${j}] produced the same nullifier (same note spent twice)`)
      }
    }
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
    outputNotes.push({ value: out.value, assetId: out.assetId, rho, rcm, leafIndex: -1, commitment })
    outRcv.push(rcv)
    outValueCommits.push(await computeValueCommit(out.value, rcv))
  }

  for (let i = 0; i < N; i++) {
    for (let j = i + 1; j < N; j++) {
      if (bufferToBigInt(outputNotes[i].commitment) === bufferToBigInt(outputNotes[j].commitment)) {
        // Astronomically unlikely with fresh randomness, but the circuit
        // rejects it (see transfer.circom's cm_distinct checks) — fail the
        // same way rather than build a witness guaranteed to be rejected.
        throw new Error(`transfer4 proof: outputs[${i}] and outputs[${j}] generated duplicate commitments; retry`)
      }
    }
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
    outputNotes: outputNotes as [Note, Note, Note, Note],
    inValueCommits: inValueCommits as [Uint8Array, Uint8Array, Uint8Array, Uint8Array],
    outValueCommits: outValueCommits as [Uint8Array, Uint8Array, Uint8Array, Uint8Array],
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
