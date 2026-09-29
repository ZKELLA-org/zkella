import * as snarkjs from 'snarkjs'
import { Note } from '../types'
import { computeNullifier, computeOwnerKey, buildNote, computeValueCommit, computeCommitment } from '../notes/builder'
import { addressToField, bufferToBigInt, bigIntToBuffer, poseidon2 } from '../crypto/poseidon'
import { encodeProof } from './encoding'

const MERKLE_DEPTH = 32

/**
 * Public inputs for the unshield (withdraw) circuit.
 *
 * Circuit: unshield.circom, `component main {public [anchor, nullifier,
 * pub_value, pub_asset_id, recipient_hash, change_commitment,
 * change_value_commit]}`
 *   Private: value, asset_id, rho, rcm, nk, path[32], path_index[32],
 *            change_rho, change_rcm, change_rcv
 *   Public:  anchor, nullifier, pub_value, pub_asset_id, recipient_hash,
 *            change_commitment, change_value_commit
 *
 * `pub_value` may be less than the spent note's full value — the remainder
 * becomes a fresh `changeNote`, owned by the same key, that stays in the
 * shielded pool (Tranche 2 Deliverable 2). Its value is not itself a public
 * input (only its commitment and a value-binding hash are), consistent with
 * every other note-creating entrypoint.
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
  /**
   * How much of the spent note's value leaves the pool publicly. Defaults to
   * the note's full value (a "full" withdrawal, matching the pre-Tranche-2
   * behavior) — the remainder always becomes a real change note, even when
   * that remainder is 0 (see this module's own doc comment for why hiding it
   * always, rather than only when non-zero, matters).
   */
  pubValue?: bigint
  /**
   * 32-byte tag folded into `recipient_hash` (`Poseidon2(address_field(to), tag)`).
   * Zero for a plain withdrawal; `contracts/swap::commit_swap` uses
   * `Poseidon2(intent_commitment, address_field(refund_to))` so the ownership
   * proof only works for that one swap. Must equal the `binding_tag` passed to
   * `unshield()`.
   */
  bindingTag?: Uint8Array
}

export interface UnshieldProofResult {
  proof: Uint8Array
  /** Poseidon2(nk, note.rho) — pass this as the `nullifier` contract call arg. */
  nullifier: Uint8Array
  /** Poseidon2(address_field(recipient), 0) — pass this as `pub_inputs.recipient_hash`. */
  recipientHash: Uint8Array
  /**
   * The change note (value = `note.value - pubValue`, same owner key as the
   * spent note) — save this exactly like any other newly-shielded note. Its
   * real `leafIndex` is `unshield()`'s return value, not set here.
   */
  changeNote: Note
  /** Poseidon2(change value, a fresh blinding factor) — pass as `change_value_commit`. */
  changeValueCommit: Uint8Array
  /** Ciphertext for `changeNote`, to pass as `encrypted_change_note` (same shape as any other note's). */
  encryptedChangeNote: Uint8Array
  /**
   * Circuit's public signals as 32-byte LE field elements, in circuit order:
   * [anchor, nullifier, pub_value, pub_asset_id, recipient_hash,
   * change_commitment, change_value_commit].
   */
  publicInputsLE: Uint8Array[]
}

/**
 * Generate a real Groth16 proof for the unshield circuit via snarkjs, using
 * the compiled artifacts at `wasmPath`/`zkeyPath` (typically
 * `circuits/unshield/build/unshield_js/unshield.wasm` and
 * `circuits/unshield/build/unshield.zkey`). Same wire format as
 * `generateShieldProof` — see `sdk/src/prover/encoding.ts`.
 *
 * `encryptNote` encrypts the change note the same way any other freshly
 * created note is encrypted (e.g. `sdk/src/notes/encrypt.ts`'s
 * `encryptNote`, keyed to the spender's own transmission key since the
 * change note is owned by the same key as the note being spent) — passed in
 * rather than imported directly so this module doesn't need to depend on a
 * specific encryption scheme.
 */
export async function generateUnshieldProof(
  witness:      UnshieldWitness,
  publicInputs: UnshieldPublicInputs,
  wasmPath:     string,
  zkeyPath:     string,
  encryptNote:  (note: Note) => Promise<Uint8Array>,
  // See generateShieldProof's `singleThread` doc comment: pass `true` when
  // calling this from inside a worker_threads.Worker.
  singleThread?: boolean,
): Promise<UnshieldProofResult> {
  if (witness.note.leafIndex < 0) {
    throw new Error('unshield proof: note.leafIndex must be a real on-chain leaf index')
  }
  if (witness.merklePath.length !== MERKLE_DEPTH) {
    throw new Error(
      `unshield proof: merklePath must have exactly ${MERKLE_DEPTH} entries, got ${witness.merklePath.length}`
    )
  }

  const ownerPk = await computeOwnerKey(witness.nk)
  if (bufferToBigInt(witness.note.ownerPk) !== bufferToBigInt(ownerPk)) {
    throw new Error('unshield proof: note.ownerPk is not derived from the supplied nullifier key')
  }

  const pubValue = publicInputs.pubValue ?? witness.note.value
  if (pubValue < 0n || pubValue > witness.note.value) {
    throw new Error('unshield proof: pubValue must be between 0 and the spent note\'s value')
  }
  const changeValue = witness.note.value - pubValue

  const pathIndex = pathIndicesFor(witness.note.leafIndex)
  const nullifier = await computeNullifier(witness.nk, witness.note.rho)
  const recipientHash = await poseidon2(
    addressToField(publicInputs.recipient),
    publicInputs.bindingTag ?? new Uint8Array(32),
  )

  // Same owner as the spent note — the change stays with whoever held the
  // original note, not the withdrawal's public recipient.
  const changeNote = await buildNote(changeValue === 0n ? 0n : changeValue, witness.note.assetId, ownerPk)
    .catch(async () => {
      // buildNote() rejects value <= 0 (a real shield always has positive
      // value); a full withdrawal's change note is legitimately 0, so build
      // it by hand instead of relaxing buildNote's own invariant for every
      // other caller.
      const rho = crypto.getRandomValues(new Uint8Array(32))
      const rcm = crypto.getRandomValues(new Uint8Array(32))
      const commitment = await computeCommitment(0n, witness.note.assetId, rho, rcm, ownerPk)
      return { value: 0n, assetId: witness.note.assetId, rho, rcm, leafIndex: -1, commitment, ownerPk } as Note
    })
  const changeRcv = crypto.getRandomValues(new Uint8Array(32))
  const changeValueCommit = await computeValueCommit(changeValue, changeRcv)

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
    pub_value:      pubValue.toString(),
    pub_asset_id:   assetIdField,
    recipient_hash: bufferToBigInt(recipientHash).toString(),
    change_rho:           bufferToBigInt(changeNote.rho).toString(),
    change_rcm:           bufferToBigInt(changeNote.rcm).toString(),
    change_rcv:           bufferToBigInt(changeRcv).toString(),
    change_commitment:    bufferToBigInt(changeNote.commitment).toString(),
    change_value_commit:  bufferToBigInt(changeValueCommit).toString(),
  }

  const { proof, publicSignals } = await snarkjs.groth16.fullProve(
    input, wasmPath, zkeyPath, undefined, undefined,
    singleThread ? { singleThread: true } : undefined,
  )

  const encryptedChangeNote = await encryptNote(changeNote)

  return {
    proof: encodeProof(proof),
    nullifier,
    recipientHash,
    changeNote,
    changeValueCommit,
    encryptedChangeNote,
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
