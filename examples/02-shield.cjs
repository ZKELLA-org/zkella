const path = require('path')
const { ZKELLAWallet, TESTNET_CONTRACTS, TESTNET_SOROBAN_RPC } = require('../sdk/dist')
const { loadKeys } = require('./_keys.cjs')
const build = path.join(__dirname, '..', 'circuits', 'shield', 'build')

;(async () => {
  const secret = process.env.STELLAR_SECRET
  if (!secret) throw new Error('set STELLAR_SECRET to a funded Testnet account secret')
  const amount = BigInt(process.env.SHIELD_AMOUNT ?? '1000')
  const keys = await loadKeys()
  const wallet = new ZKELLAWallet({
    keys: keys.spendingKey, network: 'testnet', sorobanRpc: TESTNET_SOROBAN_RPC,
    indexerUrl: process.env.INDEXER_URL ?? 'http://unused', tokenAddress: TESTNET_CONTRACTS.token,
    stellarSecret: secret,
    shieldCircuit: { wasmPath: path.join(build, 'shield_js', 'shield.wasm'), zkeyPath: path.join(build, 'shield.zkey') },
  })
  const native = require('child_process').execSync('stellar contract id asset --asset native --network testnet').toString().trim()
  const { note, submit } = await wallet.shield({ asset: native, amount })
  const { leafIndex } = await submit()
  console.log('shielded', amount.toString(), 'into leaf', leafIndex, 'commitment', Buffer.from(note.commitment).toString('hex'))
  console.log('keep this wallet secret safe: the note is spendable only with its spending key')
})().then(() => process.exit(0), e => { console.error(e.message); process.exit(1) })
