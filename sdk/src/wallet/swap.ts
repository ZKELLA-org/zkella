import { nativeToScVal, scValToNative } from '@stellar/stellar-sdk'
import { ZKELLAWallet, structScVal } from './wallet'
import { Note } from '../types'
import { encryptNote } from '../notes/encrypt'
import { buildNote } from '../notes/builder'
import { generateUnshieldProof } from '../prover/unshield'
import { generateShieldProof } from '../prover/shield'
import { generateSwapFairnessProof, computeSwapBindingTag } from '../prover/swapFairness'

export interface CircuitFiles { wasmPath: string; zkeyPath: string }

export interface SwapCircuits {
  swapFairness: CircuitFiles
  unshield:     CircuitFiles
  shield:       CircuitFiles
}

/** What the creator must keep to reveal and claim: the intent secrets and the proofs bound to it. */
export interface SwapIntent {
  swapId:           string
  intentNonce:      bigint
  intentCommitment: Uint8Array
  assetIn:          string
  assetOut:         string
  amountIn:         bigint
  amountOut:        bigint
  minAmountOut:     bigint
  fairnessProof:    Uint8Array
  expiry:           number
}

/**
 * Creator side of a shielded swap: commit a note into an intent, reveal and
 * claim the output once a relayer has executed it, or cancel an intent that was
 * never executed. The relayer's `execute_swap` is outside this wrapper.
 */
export class ZKELLASwap {
  constructor(private config: {
    wallet:              ZKELLAWallet
    swapContractAddress: string
    circuits:            SwapCircuits
  }) {}

  /** Commits `note` into a swap. The intent nonce and proofs in the result must be kept until reveal. */
  async commitSwap(opts: {
    note:           Note
    assetOut:       string
    amountOut:      bigint
    maxSlippageBps: bigint
    expiry:         number
  }): Promise<SwapIntent> {
    const { wallet, circuits, swapContractAddress } = this.config
    const keys = wallet.spendingKey()
    const me = wallet.accountAddress()
    const amountIn = opts.note.value
    const minAmountOut = (amountIn * (10000n - opts.maxSlippageBps)) / 10000n
    const intentNonce = BigInt('0x' + Buffer.from(globalThis.crypto.getRandomValues(new Uint8Array(16))).toString('hex'))

    const fair = await generateSwapFairnessProof(
      { intentNonce, amountIn, maxSlippageBps: opts.maxSlippageBps, assetIn: opts.note.assetId,
        assetOut: opts.assetOut, amountOut: opts.amountOut, minAmountOut },
      circuits.swapFairness.wasmPath, circuits.swapFairness.zkeyPath)

    const bindingTag = await computeSwapBindingTag(fair.intentCommitment, me, keys.ownerKey, opts.assetOut, opts.expiry)
    const anchor = await wallet.getMerkleRoot()
    const merklePath = await wallet.getMerklePathBytes(opts.note.leafIndex)
    const own = await generateUnshieldProof(
      { note: opts.note, nk: keys.nullifierKey, merklePath },
      { anchor, recipient: swapContractAddress, bindingTag },
      circuits.unshield.wasmPath, circuits.unshield.zkeyPath,
      changeNote => encryptNote(changeNote, keys.transmissionKey))

    const swapId = scValToNative(await wallet.submitContractCall(swapContractAddress, 'commit_swap', [
      nativeToScVal(own.nullifier,                   { type: 'bytes' }),
      nativeToScVal(fair.intentCommitment,           { type: 'bytes' }),
      nativeToScVal(opts.note.assetId,               { type: 'address' }),
      nativeToScVal(opts.assetOut,                   { type: 'address' }),
      nativeToScVal(amountIn,                        { type: 'i128' }),
      nativeToScVal(anchor,                          { type: 'bytes' }),
      nativeToScVal(me,                              { type: 'address' }),
      nativeToScVal(keys.ownerKey,                   { type: 'bytes' }),
      nativeToScVal(minAmountOut,                    { type: 'i128' }),
      nativeToScVal(own.changeNote.commitment,       { type: 'bytes' }),
      nativeToScVal(own.changeValueCommit,           { type: 'bytes' }),
      nativeToScVal(own.encryptedChangeNote,         { type: 'bytes' }),
      nativeToScVal(own.proof,                       { type: 'bytes' }),
      nativeToScVal(opts.expiry,                     { type: 'u32' }),
    ])) as Uint8Array

    return {
      swapId:           Buffer.from(swapId).toString('hex'),
      intentNonce,
      intentCommitment: fair.intentCommitment,
      assetIn:          opts.note.assetId,
      assetOut:         opts.assetOut,
      amountIn,
      amountOut:        opts.amountOut,
      minAmountOut,
      fairnessProof:    fair.proof,
      expiry:           opts.expiry,
    }
  }

  /**
   * Claims the output note after a relayer has executed the intent. The contract
   * rejects the call unless the intent is executed, so calling this early fails
   * with the contract's own error.
   */
  async revealAndClaim(intent: SwapIntent): Promise<{ leafIndex: number }> {
    const { wallet, circuits, swapContractAddress } = this.config
    const keys = wallet.spendingKey()
    const outNote = await buildNote(intent.amountOut, intent.assetOut, keys.ownerKey)
    const shield = await generateShieldProof(
      outNote,
      { commitment: outNote.commitment, asset: intent.assetOut, amount: intent.amountOut },
      circuits.shield.wasmPath, circuits.shield.zkeyPath)
    const enc = await encryptNote(outNote, keys.transmissionKey)

    const leaf = scValToNative(await wallet.submitContractCall(swapContractAddress, 'reveal_and_claim', [
      nativeToScVal(hexToBytes(intent.swapId),        { type: 'bytes' }),
      nativeToScVal(outNote.rho,                      { type: 'bytes' }),
      nativeToScVal(outNote.rcm,                      { type: 'bytes' }),
      nativeToScVal(outNote.ownerPk,                  { type: 'bytes' }),
      nativeToScVal(outNote.commitment,               { type: 'bytes' }),
      nativeToScVal(shield.valueCommit,               { type: 'bytes' }),
      nativeToScVal(enc,                              { type: 'bytes' }),
      nativeToScVal(intent.fairnessProof,             { type: 'bytes' }),
      structScVal({
        intent_commitment: intent.intentCommitment,
        asset_in:          intent.assetIn,
        asset_out:         intent.assetOut,
        amount_out:        intent.amountOut,
        min_amount_out:    intent.minAmountOut,
      }, {
        intent_commitment: 'bytes', asset_in: 'address', asset_out: 'address',
        amount_out: 'i128', min_amount_out: 'i128',
      }),
      nativeToScVal(shield.proof,                     { type: 'bytes' }),
    ])) as number
    return { leafIndex: leaf }
  }

  /** Cancels an intent that no relayer executed, after its expiry. Returns the escrowed input. */
  async cancelSwap(swapId: string): Promise<void> {
    const { wallet, swapContractAddress } = this.config
    await wallet.submitContractCall(swapContractAddress, 'cancel_swap', [
      nativeToScVal(hexToBytes(swapId), { type: 'bytes' }),
    ])
  }
}

function hexToBytes(hex: string): Uint8Array {
  return Uint8Array.from(Buffer.from(hex, 'hex'))
}
