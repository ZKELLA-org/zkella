// Regression coverage for a Critical-severity audit finding in
// `contracts/swap`: `reveal_and_claim` used to accept any `out_rho`/`out_rcm`
// under the committed `out_owner_pk`, so whoever observed a pending reveal
// in the mempool could resubmit the same fairness proof with their own
// output-note randomness and permanently strand the claimant's funds (see
// `contracts/swap/src/lib.rs`'s `SwapState::out_note_binding` doc comment).
// The fix binds the output note's randomness at `commit_swap` time
// (`out_note_binding = sha256(out_rho || out_rcm)`) and re-checks it at
// `reveal_and_claim`. This file checks the SDK side of that fix: the hash
// matches the contract's byte-for-byte, and `revealAndClaim` reveals the
// exact note `commitSwap` committed to rather than building a fresh one.
//
// The circuit provers are mocked out (real Groth16 proving is slow and
// unrelated to this wiring bug) so this test runs fast and focuses on the
// commit/reveal note-identity invariant itself.

import { createHash } from 'crypto'

jest.mock('../../sdk/src/prover/swapFairness', () => ({
  generateSwapFairnessProof: jest.fn().mockResolvedValue({
    proof: new Uint8Array(0), intentCommitment: new Uint8Array(32).fill(9), publicInputsLE: [],
  }),
  computeSwapBindingTag: jest.fn().mockResolvedValue(new Uint8Array(32).fill(1)),
}))
jest.mock('../../sdk/src/prover/unshield', () => ({
  generateUnshieldProof: jest.fn().mockResolvedValue({
    proof: new Uint8Array(0), nullifier: new Uint8Array(32).fill(2),
    changeNote: { commitment: new Uint8Array(32).fill(3) }, changeValueCommit: new Uint8Array(32).fill(4),
    encryptedChangeNote: new Uint8Array(0),
  }),
}))
jest.mock('../../sdk/src/prover/shield', () => ({
  generateShieldProof: jest.fn().mockResolvedValue({ proof: new Uint8Array(0), valueCommit: new Uint8Array(32).fill(5) }),
}))
jest.mock('../../sdk/src/notes/encrypt', () => ({
  encryptNote: jest.fn().mockResolvedValue(new Uint8Array(0)),
}))

import { nativeToScVal, scValToNative } from '@stellar/stellar-sdk'
import { ZKELLASwap } from '../../sdk/src/wallet/swap'
import { ZKELLAWallet } from '../../sdk/src/wallet/wallet'
import { ZKELLAKeys } from '../../sdk/src/keys/keys'

const ASSET = 'CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA'
const SWAP_CONTRACT = 'CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB4IF'

async function makeSwapWrapper() {
  const keys = await ZKELLAKeys.fromSeed(new Uint8Array(32).fill(21))
  const wallet = new ZKELLAWallet({
    keys: keys.spendingKey, network: 'testnet', sorobanRpc: 'http://localhost:0',
    indexerUrl: 'http://localhost:0', tokenAddress: ASSET,
    stellarSecret: (await import('@stellar/stellar-sdk')).Keypair.random().secret(),
  })
  jest.spyOn(wallet, 'getMerkleRoot').mockResolvedValue(new Uint8Array(32).fill(6))
  jest.spyOn(wallet, 'getMerklePathBytes').mockResolvedValue(Array.from({ length: 32 }, () => new Uint8Array(32)))
  jest.spyOn(wallet, 'markNoteSpent').mockImplementation(() => {})
  let capturedArgs: unknown[] = []
  const swapIdReturn = new Uint8Array(32).fill(7)
  jest.spyOn(wallet, 'submitContractCall').mockImplementation(async (_addr, fn, args) => {
    if (fn === 'commit_swap') { capturedArgs = args; return nativeToScVal(swapIdReturn, { type: 'bytes' }) }
    return nativeToScVal(11, { type: 'u32' }) // reveal_and_claim's leaf index
  })
  const swap = new ZKELLASwap({
    wallet, swapContractAddress: SWAP_CONTRACT,
    circuits: {
      swapFairness: { wasmPath: 'x', zkeyPath: 'x' },
      unshield:     { wasmPath: 'x', zkeyPath: 'x' },
      shield:       { wasmPath: 'x', zkeyPath: 'x' },
    },
  })
  return { swap, wallet, getCapturedCommitArgs: () => capturedArgs }
}

function sha256Concat(a: Uint8Array, b: Uint8Array): Uint8Array {
  return new Uint8Array(createHash('sha256').update(Buffer.concat([a, b])).digest())
}

describe('ZKELLASwap output-note binding', () => {
  test('commitSwap sends out_note_binding = sha256(out_rho || out_rcm) for the note it returns', async () => {
    const { swap, getCapturedCommitArgs } = await makeSwapWrapper()
    const note = { value: 1000n, assetId: ASSET, rho: new Uint8Array(32).fill(8), rcm: new Uint8Array(32).fill(9), leafIndex: 0, commitment: new Uint8Array(32), ownerPk: new Uint8Array(32) }

    const intent = await swap.commitSwap({ note, assetOut: ASSET, amountOut: 990n, maxSlippageBps: 100n, expiry: 999999 })

    expect(intent.outNote).toBeDefined()
    const expectedBinding = sha256Concat(intent.outNote.rho, intent.outNote.rcm)

    // commit_swap's args, in contract order: nullifier, intent_commitment, asset_in, asset_out,
    // amount_in, anchor, refund_to, out_owner_pk, out_note_binding, min_amount_out, ...
    const args = getCapturedCommitArgs()
    expect(args.length).toBe(15)
    const outNoteBinding = scValToNative(args[8] as never) as Uint8Array
    expect(Buffer.from(outNoteBinding).toString('hex')).toBe(Buffer.from(expectedBinding).toString('hex'))
  })

  test('revealAndClaim reveals the exact note committed at commitSwap time, not a freshly built one', async () => {
    const { swap, wallet } = await makeSwapWrapper()
    const note = { value: 1000n, assetId: ASSET, rho: new Uint8Array(32).fill(8), rcm: new Uint8Array(32).fill(9), leafIndex: 0, commitment: new Uint8Array(32), ownerPk: new Uint8Array(32) }

    const intent = await swap.commitSwap({ note, assetOut: ASSET, amountOut: 990n, maxSlippageBps: 100n, expiry: 999999 })

    const submitSpy = jest.spyOn(wallet, 'submitContractCall')
    await swap.revealAndClaim(intent)

    const revealCall = submitSpy.mock.calls.find(c => c[1] === 'reveal_and_claim')
    expect(revealCall).toBeDefined()
    const revealArgs = revealCall![2]
    // reveal_and_claim's args: swap_id, out_rho, out_rcm, out_owner_pk, out_commitment, ...
    const revealedRho = scValToNative(revealArgs[1] as never) as Uint8Array
    const revealedRcm = scValToNative(revealArgs[2] as never) as Uint8Array
    expect(Buffer.from(revealedRho).toString('hex')).toBe(Buffer.from(intent.outNote.rho).toString('hex'))
    expect(Buffer.from(revealedRcm).toString('hex')).toBe(Buffer.from(intent.outNote.rcm).toString('hex'))
  })
})
