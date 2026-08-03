import * as snarkjs from 'snarkjs'
import { Note } from '../types'
import { computeValueCommit } from '../notes/builder'
import { addressToField, bufferToBigInt, bigIntToBuffer } from '../crypto/poseidon'
import { encodeProof } from './encoding'

export { encodeProof, encodeVerifyingKey } from './encoding'

/**
 * Public inputs for the shield (deposit) circuit.
 *
 * Circuit: shield.circom, `component main {public [commitment, value_commit, pub_value, pub_asset_id]}`
 *   Private: value, asset_id, rho, rcm, rcv
 *   Public:  commitment, value_commit, pub_value, pub_asset_id
 *
 * Invariants enforced by circuit (see shield.circom):
 *   commitment   == Poseidon2(Poseidon2(value, asset_id), Poseidon2(rho, rcm))
 *   value_commit == Poseidon2(value, rcv)
 *   value        == pub_value   (prevents value inflation)
 *   asset_id     == pub_asset_id
 *
 * `rcv` is *not* part of `Note` — unlike `rho`/`rcm`, it isn't needed again
 * after this proof is built (it only binds this one shield call's
 * `value_commit`, not the note's long-term identity/spendability — the
 * on-chain `commitment` and future nullifier depend only on `rho`/`rcm`),
 * and folding it into `Note` would also blow past `ct20`'s fixed 176-byte
 * `ENCRYPTED_NOTE_LEN` if it were ever added to the transmitted note
 * plaintext. `generateShieldProof` below generates and discards it.
 */
export interface ShieldPublicInputs {
  commitment: Uint8Array  // 32-byte little-endian BN254 Fr field element
  asset:      string      // SEP-41 contract address
  amount:     bigint      // matches note.value; enforced in-circuit
}

export interface ShieldProofResult {
  /** 256-byte Groth16 proof in the contract's wire format: A(64) || B(128) || C(64). */
  proof: Uint8Array
  /** Poseidon2(amount, rcv) for the freshly-generated `rcv` — pass this as
   *  `shield_pub.value_commit` in the `shield()` contract call. */
  valueCommit: Uint8Array
  /**
   * The circuit's public signals, each as a 32-byte little-endian field
   * element — this is the `BytesN<32>` convention `contracts/verifier`
   * expects, in circuit order: [commitment, value_commit, pub_value, pub_asset_id].
   */
  publicInputsLE: Uint8Array[]
}

/**
 * Generate a real Groth16 proof for the shield circuit via snarkjs, using the
 * compiled circuit artifacts at `wasmPath`/`zkeyPath` (typically
 * `circuits/shield/build/shield_js/shield.wasm` and `circuits/shield/build/shield.zkey`
 * from a real, non-dev trusted-setup ceremony in production — see
 * `docs/POC_IMPLEMENTATION.md` for the current dev-ceremony caveat).
 *
 * This is the same wire format independently validated against
 * `contracts/verifier` by `circuits/shield/build/convert_to_wire_format.py`
 * and exercised in three real Stellar Testnet `shield()` transactions (see
 * `docs/POC_IMPLEMENTATION.md`, "Update: live Testnet run completed") — this
 * function is the TypeScript equivalent of that Python/CLI path, so
 * applications don't need a Python side-channel to construct a real shield
 * call.
 */
export async function generateShieldProof(
  note:         Note,
  publicInputs: ShieldPublicInputs,
  wasmPath:     string,
  zkeyPath:     string,
): Promise<ShieldProofResult> {
  // Validate public input consistency before proof generation — the circuit
  // itself also enforces value === pub_value and asset_id === pub_asset_id,
  // but failing fast here gives a much clearer error than a proving-time one.
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

  const rcv = crypto.getRandomValues(new Uint8Array(32))
  const valueCommit = await computeValueCommit(publicInputs.amount, rcv)

  const input = {
    value:         note.value.toString(),
    asset_id:      bufferToBigInt(addressToField(note.assetId)).toString(),
    rho:           bufferToBigInt(note.rho).toString(),
    rcm:           bufferToBigInt(note.rcm).toString(),
    rcv:           bufferToBigInt(rcv).toString(),
    commitment:    bufferToBigInt(publicInputs.commitment).toString(),
    value_commit:  bufferToBigInt(valueCommit).toString(),
    pub_value:     publicInputs.amount.toString(),
    pub_asset_id:  bufferToBigInt(addressToField(publicInputs.asset)).toString(),
  }

  const { proof, publicSignals } = await snarkjs.groth16.fullProve(input, wasmPath, zkeyPath)

  return {
    proof: encodeProof(proof),
    valueCommit,
    publicInputsLE: publicSignals.map((s: string) => bigIntToBuffer(BigInt(s))),
  }
}
