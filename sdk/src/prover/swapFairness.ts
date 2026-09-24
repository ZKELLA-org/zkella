import * as snarkjs from 'snarkjs'
import { poseidon2, bufferToBigInt, bigIntToBuffer, addressToField } from '../crypto/poseidon'
import { encodeProof } from './encoding'

export { encodeProof, encodeVerifyingKey } from './encoding'

/**
 * Witness/public inputs for the shielded-swap fairness circuit.
 *
 * Circuit: swap_fairness.circom, `component main {public [intent_commitment,
 * asset_in, asset_out, amount_out, min_amount_out]}`
 *   Private: intent_nonce, amount_in, max_slippage_bps
 *   Public:  intent_commitment, asset_in, asset_out, amount_out, min_amount_out
 *
 * Invariants enforced in-circuit (see swap_fairness.circom):
 *   intent_commitment == Poseidon2(Poseidon2(asset_in, asset_out), Poseidon2(amount_in * 2^32 + max_slippage_bps, intent_nonce))
 *   min_amount_out    == floor(amount_in * (10000 - max_slippage_bps) / 10000)
 *   amount_out        >= min_amount_out
 *
 * `contracts/swap::reveal_and_claim` binds `asset_in`/`asset_out`/`amount_out`
 * to the swap's on-chain state before calling the verifier, and
 * `intent_commitment` back to the value `commit_swap` was called with — this
 * proof is what lets the claimant reveal `amount_out` without ever having
 * committed to it (or `min_amount_out`) up front.
 */
export interface SwapFairnessWitness {
  intentNonce:    bigint
  amountIn:       bigint
  maxSlippageBps: bigint
  assetIn:        string  // SEP-41 contract address (field-encoded, same as token asset_id)
  assetOut:       string
  amountOut:      bigint
  minAmountOut:   bigint
}

export interface SwapFairnessProofResult {
  proof: Uint8Array
  /** Poseidon2(Poseidon2(asset_in,asset_out), Poseidon2(amount_in*2^32+slippage, nonce)) —
   *  pass this as `commit_swap`'s `intent_commitment` and
   *  `reveal_and_claim`'s `fairness_pub.intent_commitment`. */
  intentCommitment: Uint8Array
  /** Circuit's public signals as 32-byte LE field elements, in circuit
   *  order: [intent_commitment, asset_in, asset_out, amount_out, min_amount_out]. */
  publicInputsLE: Uint8Array[]
}

/**
 * Generate a real Groth16 proof for the swap-fairness circuit via snarkjs,
 * using the compiled artifacts at `wasmPath`/`zkeyPath` (typically
 * `circuits/swap/build/swap_fairness_js/swap_fairness.wasm` and
 * `circuits/swap/build/swap_fairness.zkey`).
 */
export async function generateSwapFairnessProof(
  witness:  SwapFairnessWitness,
  wasmPath: string,
  zkeyPath: string,
  // See generateShieldProof's `singleThread` doc comment: pass `true` when
  // calling this from inside a worker_threads.Worker.
  singleThread?: boolean,
): Promise<SwapFairnessProofResult> {
  if (witness.amountOut < witness.minAmountOut) {
    throw new Error(
      `swap fairness proof: amount_out (${witness.amountOut}) < min_amount_out (${witness.minAmountOut})`
    )
  }
  const expectedMin = (witness.amountIn * (10000n - witness.maxSlippageBps)) / 10000n
  if (expectedMin !== witness.minAmountOut) {
    throw new Error(
      `swap fairness proof: min_amount_out (${witness.minAmountOut}) != floor(amount_in*(10000-bps)/10000) (${expectedMin})`
    )
  }

  const assetInField  = bufferToBigInt(addressToField(witness.assetIn))
  const assetOutField = bufferToBigInt(addressToField(witness.assetOut))

  const packed = witness.amountIn * (2n ** 32n) + witness.maxSlippageBps
  const h1 = await poseidon2(bigIntToBuffer(assetInField), bigIntToBuffer(assetOutField))
  const h2 = await poseidon2(bigIntToBuffer(packed), bigIntToBuffer(witness.intentNonce))
  const intentCommitment = await poseidon2(h1, h2)

  const input = {
    intent_nonce:      witness.intentNonce.toString(),
    amount_in:         witness.amountIn.toString(),
    max_slippage_bps:  witness.maxSlippageBps.toString(),
    intent_commitment: bufferToBigInt(intentCommitment).toString(),
    asset_in:          assetInField.toString(),
    asset_out:         assetOutField.toString(),
    amount_out:        witness.amountOut.toString(),
    min_amount_out:    witness.minAmountOut.toString(),
  }

  const { proof, publicSignals } = await snarkjs.groth16.fullProve(
    input, wasmPath, zkeyPath, undefined, undefined,
    singleThread ? { singleThread: true } : undefined,
  )

  return {
    proof: encodeProof(proof),
    intentCommitment,
    publicInputsLE: publicSignals.map((s: string) => bigIntToBuffer(BigInt(s))),
  }
}

/**
 * The tag `contracts/swap::commit_swap` folds into the ownership proof's
 * `recipient_hash`: Poseidon2(Poseidon2(intent, refund_to), Poseidon2(owner_pk,
 * Poseidon2(asset_out, expiry))). Binding the claimant's owner key, the output
 * asset and the expiry (not just the intent and refund address) stops anyone who
 * copies a pending ownership proof from redirecting the output or squatting the
 * swap slot with different terms. Pass it as `bindingTag` to
 * `generateUnshieldProof`.
 */
export async function computeSwapBindingTag(
  intentCommitment: Uint8Array,
  refundTo:         string,
  outOwnerPk:       Uint8Array,
  assetOut:         string,
  expiryLedger:     number,
): Promise<Uint8Array> {
  const expiryField = new Uint8Array(32)
  new DataView(expiryField.buffer).setUint32(0, expiryLedger, true)
  const base = await poseidon2(intentCommitment, addressToField(refundTo))
  const assetAndExpiry = await poseidon2(addressToField(assetOut), expiryField)
  const terms = await poseidon2(outOwnerPk, assetAndExpiry)
  return poseidon2(base, terms)
}
