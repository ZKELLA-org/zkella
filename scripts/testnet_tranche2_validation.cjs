// Live Testnet validation for Tranche 2's new success criteria, against the
// fresh stack deployed for this tranche (see docs/TESTNET_DEPLOYMENT.md's
// Tranche 2 section for addresses):
//   D1.4 — transfer()'s proof-declared fee is actually paid to an approved
//          relayer, confirmed by a transaction the RELAYER submits (not the
//          note owner), which the relayer's own key signs.
//   D2.4 — unshield() accepts a change-note output: a partial withdrawal
//          leaves a real new note in the shielded pool.
//   D3.6 — the stalled-swap recovery path (cancel_swap) is exercised live,
//          not only in unit tests.
//
// Env: STELLAR_SECRET (sender/note-owner), RELAYER_SECRET, TOKEN_ID, SWAP_ID,
// VERIFIER_ID, ASSET_ID. Requires `npm run build --workspace=sdk`.
const path = require('path')
const { rpc, Keypair, Contract, TransactionBuilder, nativeToScVal, scValToNative, xdr } = require('@stellar/stellar-sdk')
const sdk = path.join(__dirname, '../sdk/dist')
const { ZKELLAKeys } = require(sdk + '/keys/keys')
const { ZKELLAWallet } = require(sdk + '/wallet/wallet')
const { generateTransferProof } = require(sdk + '/prover/transfer')
const { generateUnshieldProof } = require(sdk + '/prover/unshield')
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
const vec = items => xdr.ScVal.scvVec(items.map(i => nativeToScVal(i, { type: 'bytes' })))
const struct = obj => xdr.ScVal.scvMap(
  Object.entries(obj).sort(([a], [c]) => (a < c ? -1 : a > c ? 1 : 0))
    .map(([k, v]) => new xdr.ScMapEntry({ key: xdr.ScVal.scvSymbol(k), val: v })))

const RPC_URL = 'https://soroban-testnet.stellar.org'

async function submitAs(keypair, contractId, method, args) {
  const server = new rpc.Server(RPC_URL)
  const account = await server.getAccount(keypair.publicKey())
  const tx = new TransactionBuilder(account, { fee: '10000000', networkPassphrase: 'Test SDF Network ; September 2015' })
    .addOperation(new Contract(contractId).call(method, ...args))
    .setTimeout(30)
    .build()
  const prepared = await server.prepareTransaction(tx)
  prepared.sign(keypair)
  const response = await server.sendTransaction(prepared)
  if (response.status === 'ERROR') throw new Error(`${method} submission error: ${JSON.stringify(response.errorResult)}`)
  const deadline = Date.now() + 60_000
  while (Date.now() < deadline) {
    await new Promise(r => setTimeout(r, 2000))
    const r = await server.getTransaction(response.hash)
    if (r.status !== 'NOT_FOUND') {
      if (r.status !== 'SUCCESS') throw new Error(`${method} failed: ${JSON.stringify(r.resultXdr ?? r)}`)
      return { returnValue: r.returnValue, hash: response.hash }
    }
  }
  throw new Error(`${method} tx ${response.hash} not confirmed within timeout`)
}

/** The real Stellar network fee actually charged for `hash` (stroops), from Horizon. */
async function feeChargedFor(hash) {
  const res = await fetch(`https://horizon-testnet.stellar.org/transactions/${hash}`)
  const json = await res.json()
  return BigInt(json.fee_charged)
}

async function balanceOf(assetId, who) {
  const server = new rpc.Server(RPC_URL)
  const keypair = Keypair.random()
  const { Account, TransactionBuilder: TB, Networks } = require('@stellar/stellar-sdk')
  const account = new Account(keypair.publicKey(), '0')
  const tx = new TB(account, { fee: '100', networkPassphrase: Networks.TESTNET })
    .addOperation(new Contract(assetId).call('balance', addr(who)))
    .setTimeout(10)
    .build()
  const sim = await server.simulateTransaction(tx)
  if (rpc.Api.isSimulationError(sim)) throw new Error(sim.error)
  return scValToNative(sim.result.retval)
}

;(async () => {
  const { STELLAR_SECRET, RELAYER_SECRET, TOKEN_ID, SWAP_ID, ASSET_ID } = process.env
  const relayerKeypair = Keypair.fromSecret(RELAYER_SECRET)
  const relayerAddress = relayerKeypair.publicKey()

  const keys = await ZKELLAKeys.generate()
  const wallet = new ZKELLAWallet({
    keys: keys.spendingKey, network: 'testnet', sorobanRpc: RPC_URL, indexerUrl: 'http://unused',
    tokenAddress: TOKEN_ID, stellarSecret: STELLAR_SECRET,
    shieldCircuit:   { wasmPath: b('shield/build/shield_js/shield.wasm'), zkeyPath: b('shield/build/shield.zkey') },
    unshieldCircuit: { wasmPath: b('unshield/build/unshield_js/unshield.wasm'), zkeyPath: b('unshield/build/unshield.zkey') },
  })

  console.log('=== D1.4: transfer() pays its proof-declared fee to an approved relayer ===')
  console.log('shield note A')
  const a = await wallet.shield({ asset: ASSET_ID, amount: 3_000_000n })
  await a.submit()
  console.log('shield note B')
  const bb = await wallet.shield({ asset: ASSET_ID, amount: 2_000_000n })
  await bb.submit()

  const FEE = 10_000n
  const SEND_AMOUNT = 1_000_000n
  const [inA, inB] = wallet.notes.slice(-2)
  const sumIn = inA.value + inB.value
  const changeAmount = sumIn - SEND_AMOUNT - FEE
  const anchor = await wallet.getMerkleRoot()
  const [pathA, pathB] = await Promise.all([
    wallet.getMerklePathBytes(inA.leafIndex), wallet.getMerklePathBytes(inB.leafIndex),
  ])
  const recipientOwnerPk = keys.spendingKey.ownerKey // send to self for this smoke test
  const result = await generateTransferProof(
    { inputs: [{ note: inA, merklePath: pathA }, { note: inB, merklePath: pathB }], nk: keys.spendingKey.nullifierKey,
      outputs: [
        { value: SEND_AMOUNT, assetId: ASSET_ID, ownerPk: recipientOwnerPk },
        { value: changeAmount, assetId: ASSET_ID, ownerPk: keys.spendingKey.ownerKey },
      ],
      fee: FEE },
    { anchor, assetId: ASSET_ID },
    b('transfer_2in2out/build/transfer_js/transfer.wasm'), b('transfer_2in2out/build/transfer.zkey'))
  const [outRecipient, outChange] = result.outputNotes
  const [encRecipient, encChange] = await Promise.all([
    encryptNote(outRecipient, keys.spendingKey.transmissionKey), encryptNote(outChange, keys.spendingKey.transmissionKey),
  ])

  const relayerBalanceBefore = await balanceOf(ASSET_ID, relayerAddress)
  console.log('relayer balance before:', relayerBalanceBefore)

  console.log('transfer() submitted BY THE RELAYER (relayer signs, relayer pays the network fee)')
  const pubInputs = struct({
    anchor: bytes(anchor), nullifiers: vec(result.nullifiers), out_commitments: vec(result.outputNotes.map(n => n.commitment)),
    in_value_commits: vec(result.inValueCommits), out_value_commits: vec(result.outValueCommits),
    fee: i128(FEE), asset_id: addr(ASSET_ID),
  })
  const { returnValue: returned, hash: transferTxHash } = await submitAs(relayerKeypair, TOKEN_ID, 'transfer', [
    vec(result.nullifiers), vec(result.outputNotes.map(n => n.commitment)), vec([encRecipient, encChange]),
    bytes(result.proof), pubInputs, addr(relayerAddress),
  ])
  const leaves = scValToNative(returned)
  console.log('   new leaves', leaves)
  outRecipient.leafIndex = leaves[0]; outChange.leafIndex = leaves[1]
  wallet.notes = wallet.notes.filter(n => n !== inA && n !== inB)
  wallet.notes.push(outRecipient, outChange)

  const relayerBalanceAfter = await balanceOf(ASSET_ID, relayerAddress)
  const networkFeeCharged = await feeChargedFor(transferTxHash)
  const netDelta = BigInt(relayerBalanceAfter) - BigInt(relayerBalanceBefore)
  // The relayer's on-chain balance nets two real, opposite-direction transfers
  // in the same transaction: it pays the real Stellar network fee (charged
  // against the submitting account regardless of the contract call's own
  // logic) and separately receives the proof-declared application-level fee
  // via token::transfer's own internal SEP-41 transfer. Isolating the latter
  // means adding back the network fee actually charged, from Horizon.
  console.log('relayer balance after:', relayerBalanceAfter,
    `(net ${netDelta}, network fee charged ${networkFeeCharged}, app-level fee received ${netDelta + networkFeeCharged})`)
  if (netDelta + networkFeeCharged !== FEE) {
    throw new Error(`relayer was not paid exactly the proof-declared fee: got ${netDelta + networkFeeCharged}, expected ${FEE}`)
  }
  console.log('D1.4 CONFIRMED: relayer submitted the tx, signed it, paid the real network fee, and was paid the proof-declared application-level fee.\n')

  console.log('=== D2.4: unshield() accepts a partial withdrawal and creates a real change note ===')
  const noteToSplit = outRecipient // worth SEND_AMOUNT
  const withdrawAmount = noteToSplit.value / 2n
  const leafCountBefore = await wallet.callView(TOKEN_ID, 'leaf_count', [])
  console.log('leaf_count before:', leafCountBefore)
  const u = await wallet.unshield({ asset: ASSET_ID, amount: withdrawAmount, to: wallet.sourceKeypair.publicKey() })
  const { changeLeafIndex } = await u.submit()
  const leafCountAfter = await wallet.callView(TOKEN_ID, 'leaf_count', [])
  console.log('leaf_count after: ', leafCountAfter, '(change note at leaf', changeLeafIndex + ')')
  if (Number(leafCountAfter) !== Number(leafCountBefore) + 1) {
    throw new Error('leaf_count did not grow by exactly 1 for the change note')
  }
  console.log('D2.4 CONFIRMED: partial withdrawal of', withdrawAmount.toString(), 'left a real change note on-chain.\n')

  console.log('=== D3.6: the stalled-swap recovery path (cancel_swap) exercised live ===')
  const swapNote = await wallet.shield({ asset: ASSET_ID, amount: 1_500_000n })
  await swapNote.submit()
  const inputNote = wallet.notes[wallet.notes.length - 1]
  const swapAnchor = await wallet.getMerkleRoot()
  const swapPath = await wallet.getMerklePathBytes(inputNote.leafIndex)
  const nullifierIn = await require(sdk + '/notes/builder').computeNullifier(keys.spendingKey.nullifierKey, inputNote.rho)
  const intentCommitment = require('crypto').randomBytes(32)
  const refundTo = wallet.sourceKeypair.publicKey()
  const latest = (await new rpc.Server(RPC_URL).getLatestLedger()).sequence
  // cancel_swap() requires the current ledger to be *past* expiry_ledger
  // (contracts/swap::cancel_swap: "cannot cancel" otherwise) — a short
  // expiry here means a short, bounded wait below, not an immediate cancel.
  const expiry = latest + 10
  const own = await generateUnshieldProof(
    { note: inputNote, nk: keys.spendingKey.nullifierKey, merklePath: swapPath },
    { anchor: swapAnchor, recipient: SWAP_ID, bindingTag: await (async () => {
        const { computeSwapBindingTag } = require(sdk + '/prover/swapFairness')
        return computeSwapBindingTag(intentCommitment, refundTo, keys.spendingKey.ownerKey, ASSET_ID, expiry)
      })() },
    b('unshield/build/unshield_js/unshield.wasm'), b('unshield/build/unshield.zkey'),
    changeNote => encryptNote(changeNote, keys.spendingKey.transmissionKey))
  const swapId = scValToNative(await wallet.submitContractCall(SWAP_ID, 'commit_swap', [
    bytes(own.nullifier), bytes(intentCommitment), addr(ASSET_ID), addr(ASSET_ID),
    i128(1_500_000n), bytes(swapAnchor), addr(refundTo), bytes(keys.spendingKey.ownerKey),
    i128(0n), bytes(own.changeNote.commitment), bytes(own.changeValueCommit), bytes(own.encryptedChangeNote),
    bytes(own.proof), nativeToScVal(expiry, { type: 'u32' }),
  ]))
  console.log('   swap_id', Buffer.from(swapId).toString('hex'), 'committed, escrowing 1,500,000')

  const refundBalanceBefore = await balanceOf(ASSET_ID, refundTo)
  console.log(`waiting for ledger to pass expiry (${expiry}) before cancel_swap is callable...`)
  for (;;) {
    const cur = (await new rpc.Server(RPC_URL).getLatestLedger()).sequence
    if (cur > expiry) break
    await new Promise(r => setTimeout(r, 5000))
  }
  console.log('cancel_swap (relayer never executed; the committer reclaims after expiry)')
  const { hash: cancelTxHash } = await submitAs(wallet.sourceKeypair, SWAP_ID, 'cancel_swap', [bytes(swapId)])
  const refundBalanceAfter = await balanceOf(ASSET_ID, refundTo)
  const cancelNetworkFee = await feeChargedFor(cancelTxHash)
  const refundNetDelta = BigInt(refundBalanceAfter) - BigInt(refundBalanceBefore)
  console.log('refund_to balance:', refundBalanceBefore, '->', refundBalanceAfter,
    `(net ${refundNetDelta}, network fee charged ${cancelNetworkFee}, actual refund ${refundNetDelta + cancelNetworkFee})`)
  if (refundNetDelta + cancelNetworkFee !== 1_500_000n) {
    throw new Error(`cancel_swap did not refund the escrowed amount_in: got ${refundNetDelta + cancelNetworkFee}`)
  }
  console.log('D3.6 CONFIRMED: cancel_swap refunded the full escrowed amount live on Testnet.\n')

  console.log('ALL TRANCHE 2 LIVE VALIDATIONS PASSED')
  process.exit(0)
})().catch(e => { console.error('FAILED:', e.message || e); process.exit(1) })
