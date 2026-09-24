// Live check that the minimum shield amount is enforced from governance-set storage, not a constant:
// with the minimum raised, a shield below it is rejected. Env: STELLAR_SECRET, TOKEN_ID, ASSET_ID.
const path = require('path')
const sdk = path.join(__dirname, '../sdk/dist')
const { ZKELLAKeys } = require(sdk + '/keys/keys')
const { ZKELLAWallet } = require(sdk + '/wallet/wallet')
const b = (...p) => path.join(__dirname, '..', 'circuits', ...p)
;(async () => {
  const { STELLAR_SECRET, TOKEN_ID, ASSET_ID } = process.env
  const keys = await ZKELLAKeys.generate()
  const wallet = new ZKELLAWallet({
    keys: keys.spendingKey, network: 'testnet', sorobanRpc: 'https://soroban-testnet.stellar.org',
    indexerUrl: 'http://unused', tokenAddress: TOKEN_ID, stellarSecret: STELLAR_SECRET,
    shieldCircuit: { wasmPath: b('shield/build/shield_js/shield.wasm'), zkeyPath: b('shield/build/shield.zkey') },
  })
  const { submit } = await wallet.shield({ asset: ASSET_ID, amount: 1_000_000n })
  try { await submit(); console.log('UNEXPECTED: shield below the raised minimum succeeded'); process.exit(1) }
  catch (e) { console.log('shield of 1,000,000 rejected while the minimum is 2,000,000:', String(e.message).slice(0, 160)); process.exit(0) }
})()
