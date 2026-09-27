// Live Testnet run of shield_batch at its maximum size: eight real shield proofs deposited in ONE
// transaction with one aggregated token transfer. Env: STELLAR_SECRET, TOKEN_ID, ASSET_ID. Requires
// `npm run build --workspace=sdk`.
const path = require('path')
const { rpc, nativeToScVal, scValToNative, xdr } = require('@stellar/stellar-sdk')
const sdk = path.join(__dirname, '../sdk/dist')
const { ZKELLAKeys } = require(sdk + '/keys/keys')
const { ZKELLAWallet } = require(sdk + '/wallet/wallet')
const { buildNote } = require(sdk + '/notes/builder')
const { generateShieldProof } = require(sdk + '/prover/shield')
const { encryptNote } = require(sdk + '/notes/encrypt')

const b = (...p) => path.join(__dirname, '..', 'circuits', ...p)
const orig = rpc.Server.prototype.sendTransaction
rpc.Server.prototype.sendTransaction = async function (tx) {
  const r = await orig.call(this, tx)
  console.log('   tx:', `https://stellar.expert/explorer/testnet/tx/${r.hash}`, r.status)
  return r
}
const bytes = v => nativeToScVal(v, { type: 'bytes' })
const struct = obj => xdr.ScVal.scvMap(
  Object.entries(obj).sort(([a], [c]) => (a < c ? -1 : a > c ? 1 : 0))
    .map(([k, v]) => new xdr.ScMapEntry({ key: xdr.ScVal.scvSymbol(k), val: v })))

;(async () => {
  const { STELLAR_SECRET, TOKEN_ID, ASSET_ID } = process.env
  const N = Number(process.env.BATCH_SIZE || 8)
  const keys = await ZKELLAKeys.generate()
  const wallet = new ZKELLAWallet({
    keys: keys.spendingKey, network: 'testnet', sorobanRpc: 'https://soroban-testnet.stellar.org',
    indexerUrl: 'http://unused', tokenAddress: TOKEN_ID, stellarSecret: STELLAR_SECRET,
  })
  const from = wallet.sourceKeypair.publicKey()
  const items = []
  let total = 0n
  for (let i = 0; i < N; i++) {
    const amount = 2_000_000n + BigInt(i) * 100_000n
    const note = await buildNote(amount, ASSET_ID, keys.spendingKey.ownerKey)
    const { proof, valueCommit } = await generateShieldProof(
      note, { commitment: note.commitment, asset: ASSET_ID, amount },
      b('shield/build/shield_js/shield.wasm'), b('shield/build/shield.zkey'))
    const enc = await encryptNote(note, keys.spendingKey.transmissionKey)
    total += amount
    items.push(struct({
      amount: nativeToScVal(amount, { type: 'i128' }), commitment: bytes(note.commitment),
      encrypted_note: bytes(enc), owner_pk: bytes(note.ownerPk), rcm: bytes(note.rcm), rho: bytes(note.rho),
      shield_proof: bytes(proof),
      shield_pub: struct({
        commitment: bytes(note.commitment), pub_asset_id: nativeToScVal(ASSET_ID, { type: 'address' }),
        pub_value: nativeToScVal(amount, { type: 'i128' }), value_commit: bytes(valueCommit),
      }),
    }))
    console.log(`proof ${i + 1}/${N} generated`)
  }
  console.log(`shield_batch with ${N} items, total ${total}`)
  const ret = await wallet.submitContractCall(TOKEN_ID, 'shield_batch', [
    nativeToScVal(from, { type: 'address' }), nativeToScVal(ASSET_ID, { type: 'address' }), xdr.ScVal.scvVec(items),
  ])
  console.log('   leaf indices', scValToNative(ret))
  process.exit(0)
})().catch(e => { console.error('FAILED:', e.message || e); process.exit(1) })
