const path = require('path')
const { ZKELLAWallet, TESTNET_CONTRACTS, TESTNET_SOROBAN_RPC, ZKELLAKeys } = require('../sdk/dist')
const build = path.join(__dirname, '..', 'circuits')

;(async () => {
  const { STELLAR_SECRET, INDEXER_URL, UNSHIELD_AMOUNT, UNSHIELD_TO, ASSET_ID } = process.env
  if (!STELLAR_SECRET || !INDEXER_URL || !UNSHIELD_AMOUNT || !UNSHIELD_TO || !ASSET_ID) {
    throw new Error('set STELLAR_SECRET, INDEXER_URL, UNSHIELD_AMOUNT, UNSHIELD_TO (public address), ASSET_ID')
  }
  const keys = await ZKELLAKeys.generate()
  const wallet = new ZKELLAWallet({
    keys: keys.spendingKey, network: 'testnet', sorobanRpc: TESTNET_SOROBAN_RPC, indexerUrl: INDEXER_URL,
    tokenAddress: TESTNET_CONTRACTS.token, stellarSecret: STELLAR_SECRET,
    unshieldCircuit: { wasmPath: path.join(build, 'unshield/build/unshield_js/unshield.wasm'), zkeyPath: path.join(build, 'unshield/build/unshield.zkey') },
  })
  await wallet.sync()
  const { submit } = await wallet.unshield({ asset: ASSET_ID, amount: BigInt(UNSHIELD_AMOUNT), to: UNSHIELD_TO })
  const { changeLeafIndex } = await submit()
  console.log('unshielded; change note at leaf', changeLeafIndex)
})().catch(e => { console.error(e.message); process.exit(1) })
