// Regenerates the two extra genuine, distinct shield.circom proofs embedded in
// contracts/verifier/src/lib.rs as SHIELD_PROOF2_HEX/SHIELD_PUBLIC_INPUTS2_LE_HEX and
// SHIELD_PROOF3_HEX/SHIELD_PUBLIC_INPUTS3_LE_HEX, used by
// verify_batch_accepts_three_distinct_genuine_proofs and
// verify_batch_rejects_when_one_of_several_distinct_proofs_is_mismatched to batch-verify
// proofs that actually differ from each other, not the same proof repeated.
//
// Run from this directory: `node gen_batch_test_fixtures.js`, then convert each
// proof_batch_fixture_N.json with the same pattern as convert_to_wire_format.py (or paste
// the printed hex directly) into the Rust constants above.
const circomlibjs = require('circomlibjs')
const snarkjs = require('snarkjs')
const fs = require('fs')

function feToBe32(x) { return Buffer.from(BigInt(x).toString(16).padStart(64, '0'), 'hex') }
function feToLe32Hex(x) { return Buffer.from(feToBe32(x)).reverse().toString('hex') }
function g1ToBytes(p) { return Buffer.concat([feToBe32(p[0]), feToBe32(p[1])]) }
function g2ToBytes(p) {
  const [[xc0, xc1], [yc0, yc1]] = p
  return Buffer.concat([feToBe32(xc1), feToBe32(xc0), feToBe32(yc1), feToBe32(yc0)])
}

;(async () => {
  const poseidon = await circomlibjs.buildPoseidon()
  const F = poseidon.F
  const P2 = (a, b) => F.toObject(poseidon([BigInt(a), BigInt(b)]))

  // Same fixed asset_id and pk (owner key) as the existing SHIELD_PUBLIC_INPUTS_LE_HEX
  // fixture, so all three proofs share the same asset/owner and differ only in
  // value/rho/rcm/rcv — the point is distinct proofs against one VK, not distinct owners.
  const asset_id = 44239764132731213050593584193610954592770732183378613204087781673509415785175n
  const pk = 19915616739393295675676418430675585194654190321188152715167987469410225013966n

  const cases = [
    { value: 7000000n, rho: 111n, rcm: 222n, rcv: 333n },
    { value: 9500000n, rho: 444n, rcm: 555n, rcv: 666n },
  ]

  for (let i = 0; i < cases.length; i++) {
    const { value, rho, rcm, rcv } = cases[i]
    const commitment = P2(P2(P2(value, asset_id), P2(rho, rcm)), pk)
    const value_commit = P2(value, rcv)
    const input = {
      value: String(value), asset_id: String(asset_id), rho: String(rho), rcm: String(rcm),
      rcv: String(rcv), pk: String(pk), commitment: String(commitment), value_commit: String(value_commit),
      pub_value: String(value), pub_asset_id: String(asset_id),
    }
    const { proof, publicSignals } = await snarkjs.groth16.fullProve(
      input, __dirname + '/shield_js/shield.wasm', __dirname + '/shield.zkey',
    )
    const vk = JSON.parse(fs.readFileSync(__dirname + '/verification_key.json'))
    const ok = await snarkjs.groth16.verify(vk, publicSignals, proof)
    const proofBytes = Buffer.concat([g1ToBytes(proof.pi_a), g2ToBytes(proof.pi_b), g1ToBytes(proof.pi_c)])
    console.log(`case ${i + 1}: verifies=${ok}`)
    console.log(`  PROOF${i + 1}_HEX = "${proofBytes.toString('hex')}"`)
    console.log(`  PUBLIC_INPUTS${i + 1}_LE_HEX = [${publicSignals.map((s) => `"${feToLe32Hex(s)}"`).join(', ')}]`)
    fs.writeFileSync(__dirname + `/proof_batch_fixture_${i + 1}.json`, JSON.stringify({ proof, publicSignals }, null, 2))
  }
})().catch((e) => { console.error(e); process.exit(1) })
