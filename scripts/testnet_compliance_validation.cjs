// Live Testnet validation of the compliance contract with a real non-membership proof over a sorted
// sanctions tree. Also shows that a listed address cannot even build a witness.
// Env: STELLAR_SECRET, COMPLIANCE_ID. Requires `npm run build --workspace=sdk`.
const path = require('path')
const crypto = require('crypto')
const circomlibjs = require('circomlibjs')
const snarkjs = require('snarkjs')
const { rpc, nativeToScVal, scValToNative, xdr } = require('@stellar/stellar-sdk')
const sdk = path.join(__dirname, '../sdk/dist')
const { ZKELLAKeys } = require(sdk + '/keys/keys')
const { ZKELLAWallet } = require(sdk + '/wallet/wallet')
const { encodeProof } = require(sdk + '/prover/encoding')
const { bigIntToBuffer } = require(sdk + '/crypto/poseidon')

const b = (...p) => path.join(__dirname, '..', 'circuits', ...p)
const orig = rpc.Server.prototype.sendTransaction
rpc.Server.prototype.sendTransaction = async function (tx) {
  const r = await orig.call(this, tx)
  console.log('   tx:', `https://stellar.expert/explorer/testnet/tx/${r.hash}`, r.status)
  return r
}
const D = 32
const MAX = (1n << 248n) - 1n

;(async () => {
  const { STELLAR_SECRET, COMPLIANCE_ID } = process.env
  const poseidon = await circomlibjs.buildPoseidon()
  const P2 = (a, c) => poseidon.F.toObject(poseidon([a, c]))
  const empty = [0n]
  for (let i = 1; i <= D; i++) empty.push(P2(empty[i - 1], empty[i - 1]))

  const tree = leaves => {
    const levels = [new Map(leaves.map((v, i) => [i, v]))]
    for (let l = 0; l < D; l++) {
      const next = new Map()
      for (const idx of new Set([...levels[l].keys()].map(i => i >> 1)))
        next.set(idx, P2(levels[l].get(2 * idx) ?? empty[l], levels[l].get(2 * idx + 1) ?? empty[l]))
      levels.push(next)
    }
    const proof = index => {
      const path = [], bits = []
      let i = index
      for (let l = 0; l < D; l++) { path.push(String(levels[l].get(i ^ 1) ?? empty[l])); bits.push(String(i & 1)); i >>= 1 }
      return { path, bits }
    }
    return { root: levels[D].get(0), proof }
  }
  const witness = (sk, leaves, lo, up) => {
    const t = tree(leaves), l = t.proof(lo), u = t.proof(up)
    return { sk: String(sk), lower_leaf: String(leaves[lo]), upper_leaf: String(leaves[up]),
      lower_path: l.path, lower_path_index: l.bits, upper_path: u.path, upper_path_index: u.bits,
      sanctions_root: String(t.root), tk_commitment: String(P2(sk, 0n)) }
  }
  const wasm = b('compliance/build/non_membership_js/non_membership.wasm'), zkey = b('compliance/build/non_membership.zkey')

  const sk = BigInt('0x' + crypto.randomBytes(16).toString('hex'))
  const address = P2(sk, 1n) & MAX
  const rand = () => BigInt('0x' + crypto.randomBytes(8).toString('hex'))

  console.log('a sanctioned address cannot build a witness')
  try {
    await snarkjs.groth16.fullProve(witness(sk, [0n, address, address + rand(), MAX], 1, 2), wasm, zkey)
    throw new Error('UNEXPECTED: a listed address produced a proof')
  } catch (e) {
    if (String(e.message).startsWith('UNEXPECTED')) throw e
    console.log('   rejected, as required')
  }

  console.log('an unlisted address proves non-membership')
  const leaves = [0n, address - 1n - rand(), address + 1n + rand(), MAX]
  const w = witness(sk, leaves, 1, 2)
  const { proof, publicSignals } = await snarkjs.groth16.fullProve(w, wasm, zkey)

  const keys = await ZKELLAKeys.generate()
  const wallet = new ZKELLAWallet({
    keys: keys.spendingKey, network: 'testnet',
    sorobanRpc: 'https://soroban-testnet.stellar.org', indexerUrl: 'http://unused',
    tokenAddress: COMPLIANCE_ID, stellarSecret: STELLAR_SECRET,
  })
  const owner = wallet.sourceKeypair.publicKey()
  const [root, tkc] = publicSignals.map(s => bigIntToBuffer(BigInt(s)))
  const bytes = v => nativeToScVal(v, { type: 'bytes' })
  const inputs = xdr.ScVal.scvMap([
    new xdr.ScMapEntry({ key: xdr.ScVal.scvSymbol('sanctions_root'), val: bytes(root) }),
    new xdr.ScMapEntry({ key: xdr.ScVal.scvSymbol('tk_commitment'), val: bytes(tkc) }),
  ])

  console.log('publish_compliance_proof (real proof verified on-chain)')
  await wallet.submitContractCall(COMPLIANCE_ID, 'publish_compliance_proof',
    [nativeToScVal(owner, { type: 'address' }), bytes(encodeProof(proof)), inputs])

  const rec = await wallet.callView(COMPLIANCE_ID, 'get_compliance_proof', [nativeToScVal(owner, { type: 'address' })])
  console.log('stored record ledger', rec.published_ledger, 'root matches', Buffer.from(rec.sanctions_root).equals(Buffer.from(root)))
  process.exit(0)
})().catch(e => { console.error('FAILED:', e.message || e); process.exit(1) })
