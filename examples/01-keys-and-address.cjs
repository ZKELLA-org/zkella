const { ZKELLAKeys, TESTNET_CONTRACTS } = require('../sdk/dist')

;(async () => {
  const keys = await ZKELLAKeys.fromSeed(new Uint8Array(32).fill(1))
  const address = await keys.deriveAddress(0)
  console.log('shielded address:', address.toString())
  console.log('viewing key export:', JSON.stringify(keys.exportViewingKey(0, 'testnet')).slice(0, 80) + '...')
  console.log('Testnet token:', TESTNET_CONTRACTS.token)
})().catch(e => { console.error(e.message); process.exit(1) })
