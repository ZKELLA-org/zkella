// Live Testnet validation of the swap flow (commit_swap -> execute_swap -> reveal_and_claim),
// exercising a real unshield ownership proof, a real swap-fairness proof and a real shield proof.
// Env: STELLAR_SECRET, TOKEN_ID, SWAP_ID, ASSET_ID. Requires `npm run build --workspace=sdk`.
const path = require('path')
const { rpc, nativeToScVal, scValToNative, xdr } = require('@stellar/stellar-sdk')
const sdk = path.join(__dirname, '../sdk/dist')
const { ZKELLAKeys } = require(sdk + '/keys/keys')
const { ZKELLAWallet } = require(sdk + '/wallet/wallet')
const { generateUnshieldProof } = require(sdk + '/prover/unshield')
const { generateShieldProof } = require(sdk + '/prover/shield')
const { generateSwapFairnessProof } = require(sdk + '/prover/swapFairness')
const { buildNote, computeNullifier } = require(sdk + '/notes/builder')
const { poseidon2, addressToField, bigIntToBuffer } = require(sdk + '/crypto/poseidon')
const { encryptNote } = require(sdk + '/notes/encrypt')

const b = (...p) => path.join(__dirname, '..', 'circuits', ...p)
const orig = rpc.Server.prototype.sendTransaction
rpc.Server.prototype.sendTransaction = async function (tx) {
  const r = await orig.call(this, tx)
  console.log('   tx:', `https://stellar.expert/explorer/testnet/tx/${r.hash}`, r.status)
  return r
}
const bytes = v => nativeToScVal(v, { type: 'bytes' })
const addr = v => nativeToScVal(v, { type: 'address' })
const i128 = v => nativeToScVal(v, { type: 'i128' })
const struct = obj => xdr.ScVal.scvMap(
  Object.entries(obj).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([k, v]) => new xdr.ScMapEntry({ key: xdr.ScVal.scvSymbol(k), val: v })))

;(async () => {
  const { STELLAR_SECRET, TOKEN_ID, SWAP_ID, ASSET_ID } = process.env
  const keys = await ZKELLAKeys.generate()
  const wallet = new ZKELLAWallet({
    keys: keys.spendingKey, network: 'testnet',
    sorobanRpc: 'https://soroban-testnet.stellar.org', indexerUrl: 'http://unused',
    tokenAddress: TOKEN_ID, stellarSecret: STELLAR_SECRET,
    shieldCircuit: { wasmPath: b('shield/build/shield_js/shield.wasm'), zkeyPath: b('shield/build/shield.zkey') },
  })
  const me = wallet.sourceKeypair.publicKey()
  const AMOUNT_IN = 5_000_000n, SLIPPAGE = 100n
  const AMOUNT_OUT = 4_990_000n
  const MIN_OUT = (AMOUNT_IN * (10000n - SLIPPAGE)) / 10000n

  console.log('shield input note')
  const { note, submit } = await wallet.shield({ asset: ASSET_ID, amount: AMOUNT_IN })
  await submit()

  console.log('fairness proof (intent commitment)')
  const nonce = BigInt('0x' + Buffer.from(require('crypto').randomBytes(16)).toString('hex'))
  const fair = await generateSwapFairnessProof(
    { intentNonce: nonce, amountIn: AMOUNT_IN, maxSlippageBps: SLIPPAGE, assetIn: ASSET_ID, assetOut: ASSET_ID,
      amountOut: AMOUNT_OUT, minAmountOut: MIN_OUT },
    b('swap/build/swap_fairness_js/swap_fairness.wasm'), b('swap/build/swap_fairness.zkey'))

  console.log('commit_swap (real unshield ownership proof, bound to this intent)')
  const bindingTag = await poseidon2(fair.intentCommitment, addressToField(me))
  const anchor = await wallet.getMerkleRoot()
  const merklePath = await wallet.getMerklePathBytes(note.leafIndex)
  const own = await generateUnshieldProof(
    { note, nk: keys.spendingKey.nullifierKey, merklePath },
    { anchor, recipient: SWAP_ID, bindingTag },
    b('unshield/build/unshield_js/unshield.wasm'), b('unshield/build/unshield.zkey'))
  const latest = (await new rpc.Server('https://soroban-testnet.stellar.org').getLatestLedger()).sequence
  const swapId = scValToNative(await wallet.submitContractCall(SWAP_ID, 'commit_swap', [
    bytes(own.nullifier), bytes(fair.intentCommitment), addr(ASSET_ID), addr(ASSET_ID),
    i128(AMOUNT_IN), bytes(anchor), addr(me), bytes(own.proof),
    nativeToScVal(latest + 300, { type: 'u32' }),
  ]))
  console.log('   swap_id', Buffer.from(swapId).toString('hex'))

  console.log('execute_swap (relayer fronts asset_out)')
  await wallet.submitContractCall(SWAP_ID, 'execute_swap', [bytes(swapId), i128(AMOUNT_OUT), addr(me)])

  console.log('reveal_and_claim (real fairness proof + real shield proof for the output note)')
  const outNote = await buildNote(AMOUNT_OUT, ASSET_ID, keys.spendingKey.ownerKey)
  const sh = await generateShieldProof(outNote, { commitment: outNote.commitment, asset: ASSET_ID, amount: AMOUNT_OUT },
    b('shield/build/shield_js/shield.wasm'), b('shield/build/shield.zkey'))
  const enc = await encryptNote(outNote, keys.spendingKey.transmissionKey)
  const leaf = scValToNative(await wallet.submitContractCall(SWAP_ID, 'reveal_and_claim', [
    bytes(swapId), bytes(outNote.rho), bytes(outNote.rcm), bytes(outNote.ownerPk),
    bytes(outNote.commitment), bytes(sh.valueCommit), bytes(enc), bytes(fair.proof),
    struct({
      intent_commitment: bytes(fair.intentCommitment), asset_in: addr(ASSET_ID), asset_out: addr(ASSET_ID),
      amount_out: i128(AMOUNT_OUT), min_amount_out: i128(MIN_OUT),
    }),
    bytes(sh.proof),
  ]))
  console.log('   claimed into leaf', leaf)
  console.log('supply', await wallet.callView(TOKEN_ID, 'shielded_supply', [addr(ASSET_ID)]))
  process.exit(0)
})().catch(e => { console.error('FAILED:', e.message || e); process.exit(1) })
