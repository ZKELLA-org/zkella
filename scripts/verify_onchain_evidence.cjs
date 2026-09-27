// Independently re-checks, from the public Testnet RPC alone, every transaction the Tranche 1 docs cite for the
// current stack: each must exist, have succeeded, and have invoked the documented function on the documented
// contract. Also reads live contract state (leaf count, shielded supply, registered verifying keys, minimum
// shield amount, the stored compliance record). Exits non-zero on any mismatch. No secrets needed.
const { rpc, Address, Contract, TransactionBuilder, Networks, Keypair, Account, nativeToScVal, scValToNative } = require('@stellar/stellar-sdk')

const STACK = {
  verifier:   'CBHQUNPD42ZODQWCEK2SKLAARHHY75SGCVWHW6QLWLGLXWJ5JS2QORUY',
  token:      'CDDM46ZV3KLULXUGUOWSCR5BGZ6BC5XJDDMVTV4JXOLZBJXD6EQCJ75Q',
  swap:       'CB7TRLNTX6G3QNVDTHQHL46VNDQMMUUE4ZM5O6AIFFU6PWKGPKIQ7PYY',
  compliance: 'CAUZB3RTW23QQ5CT6W7KLINZDYVO56DSUZQ5AHKL56KWBH64QD5LNA3Q',
}
// hash, contract role, function, what it evidences
const TXS = [
  ['e51f3be33e335917b663cb1969a7d9812cbac1c5e97e797c7fb2cdcb43abd2b1', 'token', 'shield', 'D1/D3/D5 shield #0 (real Groth16)'],
  ['048f02332f51a0f5efe99c40e01f4b80b073d61755328630c4b28637d150b084', 'token', 'shield', 'shield #1'],
  ['0ac97d14310a692a35a7a3c9da71bc84e03e96eed0dcf0c97120a52df2183700', 'token', 'shield', 'shield #2'],
  ['244b995070978701a382202355d76c352d63e1b150092541da77d8c9a4910b1c', 'token', 'shield', 'shield #3 (3 repeats in a fresh environment)'],
  ['15cbeef9533724df6ea96d3e96152255039664b11dac8640dd3d4c01370678ba', 'token', 'transfer4', 'D5 generateTransfer4Proof'],
  ['c99b6b23dd068d3c12c77697cb614efa06c805349233716851efc85a22f80891', 'token', 'unshield', 'D5 generateUnshieldProof'],
  ['22e3c4e31121a045f319e965a04761edca3d527f3dfb077423aaf0e5eac5964d', 'token', 'shield_batch', 'D1 batched multi-deposit (8 items)'],
  ['ff8756d3320ae98a03abf76562e3b7fb980283624ca50700e1979c939e1d527d', 'token', 'shield', 'swap input note'],
  ['96ac0a773395a31b36521abe81fa2ff933e6cadf0502399a467e5ec3738b1340', 'swap', 'commit_swap', 'swap ownership proof'],
  ['994f97fdf6b73dbcb2fd4bf8467a49be63c6d3dd2a5a7483aacfd6c314949797', 'swap', 'execute_swap', 'swap execute'],
  ['56cf20e1bed210acc1548e32514ccfc59b8f6dc31ffd788d5c609e9054321297', 'swap', 'reveal_and_claim', 'D5 swap-fairness generator'],
  ['514b9abca55beeb41d56f739f11d83ee9cb8d3a5be90f33cb0736318e3eb5385', 'compliance', 'publish_compliance_proof', 'compliance non-membership proof'],
]
const server = new rpc.Server('https://soroban-testnet.stellar.org')
let bad = 0
const fail = m => { bad++; console.log('  MISMATCH:', m) }

async function view(contract, fn, args = []) {
  const acct = new Account(Keypair.random().publicKey(), '0')
  const tx = new TransactionBuilder(acct, { fee: '100', networkPassphrase: Networks.TESTNET })
    .addOperation(new Contract(contract).call(fn, ...args)).setTimeout(10).build()
  const sim = await server.simulateTransaction(tx)
  if (rpc.Api.isSimulationError(sim)) throw new Error(`${fn}: ${sim.error}`)
  return scValToNative(sim.result.retval)
}

;(async () => {
  console.log('== transactions')
  for (const [hash, role, fn, what] of TXS) {
    const t = await server.getTransaction(hash)
    const op = t.envelopeXdr.v1.tx.operations[0]
    const ic = op.body.value.hostFunction.value
    const gotFn = ic.functionName.toString()
    const gotContract = Address.fromScAddress(ic.contractAddress).toString()
    const ok = t.status === 'SUCCESS' && gotFn === fn && gotContract === STACK[role]
    console.log(`${ok ? 'OK  ' : 'FAIL'} ${hash.slice(0, 8)} ledger ${t.ledger} ${t.status} ${role}.${gotFn}  (${what})`)
    if (!ok) fail(`${hash}: expected ${role}.${fn} on ${STACK[role]}, got ${gotContract}.${gotFn} status ${t.status}`)
  }

  console.log('== contract state')
  const asset = 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC'
  const leaves = await view(STACK.token, 'leaf_count')
  const supply = await view(STACK.token, 'shielded_supply', [nativeToScVal(asset, { type: 'address' })])
  const minAmt = await view(STACK.token, 'min_shield_amount')
  const approved = await view(STACK.token, 'is_asset_approved', [nativeToScVal(asset, { type: 'address' })])
  console.log(`token leaf_count=${leaves} shielded_supply=${supply} min_shield_amount=${minAmt} asset_approved=${approved}`)
  // 4 + 4 (transfer4 outputs) + 8 (batch) + 1 (swap input) + 1 (swap output) = 18 leaves
  if (Number(leaves) !== 18) fail(`expected 18 leaves, found ${leaves}`)
  for (const [c, name] of [[0, 'Shield'], [2, 'Unshield'], [3, 'NonMembership'], [4, 'Transfer4x4'], [5, 'SwapFairness']]) {
    const vk = await view(STACK.verifier, 'get_verifying_key', [nativeToScVal(c, { type: 'u32' })])
    console.log(`verifier key ${name}: ${vk.length} bytes registered`)
    if (!vk.length) fail(`no verifying key for ${name}`)
  }
  console.log('done;', bad ? `${bad} mismatch(es)` : 'all evidence matches')
  process.exit(bad ? 1 : 0)
})().catch(e => { console.error('FAILED:', e.message || e); process.exit(2) })
