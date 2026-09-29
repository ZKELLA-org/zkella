// Registers every circuit's verifying key on a freshly-deployed `verifier` contract,
// converting each circuit's `verification_key.json` to the wire format
// `contracts/verifier` expects directly from the compiled artifacts (no external
// pre-converted files needed).
// Env: VERIFIER (contract id), DEPLOYER_SECRET.
const path = require('path')
const { rpc, Keypair, Contract, TransactionBuilder, Networks, nativeToScVal } = require('@stellar/stellar-sdk')

const RPC_URL = 'https://soroban-testnet.stellar.org'
const CIRCUITS_DIR = path.join(__dirname, '..', 'circuits')

function feToBe32(x) {
  return Buffer.from(BigInt(x).toString(16).padStart(64, '0'), 'hex')
}
function g1ToBytes(p) {
  return Buffer.concat([feToBe32(p[0]), feToBe32(p[1])])
}
function g2ToBytes(p) {
  const [[xc0, xc1], [yc0, yc1]] = p
  return Buffer.concat([feToBe32(xc1), feToBe32(xc0), feToBe32(yc1), feToBe32(yc0)])
}
function vkBytes(vk) {
  let b = Buffer.concat([g1ToBytes(vk.vk_alpha_1), g2ToBytes(vk.vk_beta_2), g2ToBytes(vk.vk_gamma_2), g2ToBytes(vk.vk_delta_2)])
  for (const ic of vk.IC) b = Buffer.concat([b, g1ToBytes(ic)])
  return b
}

;(async () => {
  const { VERIFIER, DEPLOYER_SECRET } = process.env
  const kp = Keypair.fromSecret(DEPLOYER_SECRET)
  const server = new rpc.Server(RPC_URL)
  // (circuit dir, build-artifact prefix, CircuitType discriminant — see
  // contracts/verifier-interface/src/lib.rs's CircuitType enum)
  const circuits = [
    ['shield', 'shield', 0],
    ['transfer_2in2out', 'transfer', 1],
    ['unshield', 'unshield', 2],
    ['transfer_4in4out', 'transfer', 4],
    ['swap', 'swap_fairness', 5],
  ]
  for (const [dir, prefix, code] of circuits) {
    const vk = require(path.join(CIRCUITS_DIR, dir, 'build', 'verification_key.json'))
    const bytes = vkBytes(vk)
    const account = await server.getAccount(kp.publicKey())
    const tx = new TransactionBuilder(account, { fee: '10000000', networkPassphrase: Networks.TESTNET })
      .addOperation(new Contract(VERIFIER).call(
        'register_verifying_key',
        nativeToScVal(code, { type: 'u32' }),
        nativeToScVal(bytes, { type: 'bytes' }),
      ))
      .setTimeout(30)
      .build()
    const prepared = await server.prepareTransaction(tx)
    prepared.sign(kp)
    const response = await server.sendTransaction(prepared)
    console.log(dir, 'tx', response.hash, response.status)
    if (response.status === 'ERROR') { console.error(JSON.stringify(response.errorResult)); process.exit(1) }
    const deadline = Date.now() + 60000
    let done = false
    while (Date.now() < deadline) {
      await new Promise(r => setTimeout(r, 2000))
      const r = await server.getTransaction(response.hash)
      if (r.status !== 'NOT_FOUND') {
        console.log('  ', r.status)
        if (r.status !== 'SUCCESS') { console.error(JSON.stringify(r)); process.exit(1) }
        done = true
        break
      }
    }
    if (!done) { console.error('timeout'); process.exit(1) }
  }
  console.log('all VKs registered')
})().catch(e => { console.error(e); process.exit(1) })
