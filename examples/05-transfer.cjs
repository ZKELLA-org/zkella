const path = require('path')
const { ZKELLAWallet, TESTNET_CONTRACTS, TESTNET_SOROBAN_RPC } = require('../sdk/dist')
const { loadKeys } = require('./_keys.cjs')
const build = path.join(__dirname, '..', 'circuits')

;(async () => {
  const { STELLAR_SECRET, INDEXER_URL, RECIPIENT_TK, RECIPIENT_OWNER_KEY, TRANSFER_AMOUNT, ASSET_ID } = process.env
  if (!STELLAR_SECRET || !INDEXER_URL || !RECIPIENT_TK || !RECIPIENT_OWNER_KEY || !TRANSFER_AMOUNT || !ASSET_ID) {
    throw new Error('set STELLAR_SECRET, INDEXER_URL, RECIPIENT_TK and RECIPIENT_OWNER_KEY (both hex, 32 bytes), TRANSFER_AMOUNT, ASSET_ID')
  }
  const keys = await loadKeys()
  const wallet = new ZKELLAWallet({
    keys: keys.spendingKey, network: 'testnet', sorobanRpc: TESTNET_SOROBAN_RPC, indexerUrl: INDEXER_URL,
    tokenAddress: TESTNET_CONTRACTS.token, stellarSecret: STELLAR_SECRET,
    transferCircuit: { wasmPath: path.join(build, 'transfer_2in2out/build/transfer_js/transfer.wasm'), zkeyPath: path.join(build, 'transfer_2in2out/build/transfer.zkey') },
  })
  await wallet.sync()
  const { submit } = await wallet.transfer({
    asset: ASSET_ID, amount: BigInt(TRANSFER_AMOUNT), to: RECIPIENT_TK, toOwnerKey: RECIPIENT_OWNER_KEY,
  })
  const { leafIndices } = await submit()
  console.log('transferred; new leaves', leafIndices)
})().then(() => process.exit(0), e => { console.error(e.message); process.exit(1) })
