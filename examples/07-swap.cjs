const path = require('path')
const { execSync } = require('child_process')
const { ZKELLAWallet, ZKELLASwap, TESTNET_CONTRACTS, TESTNET_SOROBAN_RPC } = require('../sdk/dist')
const { loadKeys } = require('./_keys.cjs')
const build = path.join(__dirname, '..', 'circuits')

;(async () => {
  const {
    STELLAR_SECRET, INDEXER_URL, SWAP_AMOUNT,
    // Optional: swap into a *different* asset instead of the default
    // same-asset (native-to-native) demo below. When set, SWAP_MIN_AMOUNT_OUT
    // must be set too — see the comment further down for why the SDK can't
    // compute it for you across two different assets.
    SWAP_ASSET_OUT, SWAP_AMOUNT_OUT, SWAP_MIN_AMOUNT_OUT,
  } = process.env
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

  const assetOut = SWAP_ASSET_OUT || native
  const crossAsset = assetOut !== native
  const amountOut = SWAP_AMOUNT_OUT ? BigInt(SWAP_AMOUNT_OUT) : note.value

  // minAmountOut is the lowest amount of assetOut the creator will accept —
  // a direct, asset_out-denominated value bound into the swap's on-chain
  // commitment, not something the SDK derives from amountIn by a flat
  // slippage percentage (that derivation mixed asset_in-denominated and
  // asset_out-denominated units with no exchange rate between them, which
  // only happened to look sound for a same-asset pair; see
  // circuits/swap/swap_fairness.circom's doc comment). For a cross-asset
  // swap there's no rate the SDK could safely invent, so the caller must
  // supply a real quoted minimum explicitly via SWAP_MIN_AMOUNT_OUT.
  let minAmountOut
  if (SWAP_MIN_AMOUNT_OUT) {
    minAmountOut = BigInt(SWAP_MIN_AMOUNT_OUT)
  } else if (crossAsset) {
    throw new Error('cross-asset swap (SWAP_ASSET_OUT set): also set SWAP_MIN_AMOUNT_OUT to a real quoted minimum for that asset')
  } else {
    minAmountOut = (amountOut * 9900n) / 10000n // same-asset demo default: 1% slippage tolerance
  }

  const intent = await swap.commitSwap({ note, assetOut, amountOut, minAmountOut, expiry: latest + 400 })
  console.log(crossAsset ? 'committed cross-asset swap' : 'committed swap', intent.swapId, 'expiry', intent.expiry)
  console.log(`asset_in ${note.assetId} -> asset_out ${assetOut}, amount_out ${amountOut}, min_amount_out ${minAmountOut}`)
  console.log('a relayer must now execute this swap with execute_swap; then call revealAndClaim(intent).')
  console.log('if no relayer executes it before expiry, cancelSwap(intent.swapId) refunds the escrow.')
})().then(() => process.exit(0), e => { console.error(e.message); process.exit(1) })
