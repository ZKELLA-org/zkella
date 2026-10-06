const path = require('path')
const { execSync } = require('child_process')
const { ZKELLAWallet, ZKELLASwap, TESTNET_CONTRACTS, TESTNET_SOROBAN_RPC } = require('../sdk/dist')
const { loadKeys } = require('./_keys.cjs')
const build = path.join(__dirname, '..', 'circuits')

;(async () => {
  const { STELLAR_SECRET, INDEXER_URL, SWAP_AMOUNT } = process.env
  if (!STELLAR_SECRET || !INDEXER_URL || !SWAP_AMOUNT) throw new Error('set STELLAR_SECRET, INDEXER_URL, SWAP_AMOUNT')
  const native = execSync('stellar contract id asset --asset native --network testnet').toString().trim()
  const keys = await loadKeys()
  const wallet = new ZKELLAWallet({
    keys: keys.spendingKey, network: 'testnet', sorobanRpc: TESTNET_SOROBAN_RPC, indexerUrl: INDEXER_URL,
    tokenAddress: TESTNET_CONTRACTS.token, stellarSecret: STELLAR_SECRET,
    shieldCircuit: { wasmPath: path.join(build, 'shield/build/shield_js/shield.wasm'), zkeyPath: path.join(build, 'shield/build/shield.zkey') },
    unshieldCircuit: { wasmPath: path.join(build, 'unshield/build/unshield_js/unshield.wasm'), zkeyPath: path.join(build, 'unshield/build/unshield.zkey') },
  })
  await wallet.sync()
  const amount = BigInt(SWAP_AMOUNT)
  const swap = new ZKELLASwap({ wallet, swapContractAddress: TESTNET_CONTRACTS.swap, circuits: {
    swapFairness: { wasmPath: path.join(build, 'swap/build/swap_fairness_js/swap_fairness.wasm'), zkeyPath: path.join(build, 'swap/build/swap_fairness.zkey') },
    unshield: { wasmPath: path.join(build, 'unshield/build/unshield_js/unshield.wasm'), zkeyPath: path.join(build, 'unshield/build/unshield.zkey') },
    shield: { wasmPath: path.join(build, 'shield/build/shield_js/shield.wasm'), zkeyPath: path.join(build, 'shield/build/shield.zkey') },
  } })
  const note = wallet.spendableNotes(native).find(n => n.value >= amount)
  if (!note) throw new Error('no spendable note of at least SWAP_AMOUNT; shield one first with 02-shield.cjs')
  const latest = (await new (require('@stellar/stellar-sdk').rpc.Server)(TESTNET_SOROBAN_RPC).getLatestLedger()).sequence
  // The SDK sets min_amount_out to the whole note's value minus maxSlippageBps, so the
  // quote must be at least that. Quoting the note's value at parity meets the bound.
  const intent = await swap.commitSwap({ note, assetOut: native, amountOut: note.value, maxSlippageBps: 100n, expiry: latest + 400 })
  console.log('committed swap', intent.swapId, 'expiry', intent.expiry)
  console.log('a relayer must now execute this swap with execute_swap; then call revealAndClaim(intent).')
  console.log('if no relayer executes it before expiry, cancelSwap(intent.swapId) refunds the escrow.')
})().then(() => process.exit(0), e => { console.error(e.message); process.exit(1) })
