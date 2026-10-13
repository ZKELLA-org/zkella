import * as fs   from 'fs'
import * as path from 'path'
import * as snarkjs from 'snarkjs'
import {
  generateNonMembershipProof,
  buildSanctionsTree,
  nonMembershipAddress,
} from '../../sdk/src/prover/compliance'

const BUILD = path.resolve(__dirname, '../../circuits/compliance/build')
const WASM  = path.join(BUILD, 'non_membership_js/non_membership.wasm')
const ZKEY  = path.join(BUILD, 'non_membership.zkey')
const VK    = JSON.parse(fs.readFileSync(path.join(BUILD, 'verification_key.json'), 'utf8'))

const SK = 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdefn

describe('non-membership proof for the compliance circuit', () => {
  test('a prover off the sanctions list gets a proof that verifies against the published root', async () => {
    const sanctioned = [111n, 222_222n, 333_333_333n]
    const result = await generateNonMembershipProof({ sk: SK, sanctioned, wasmPath: WASM, zkeyPath: ZKEY })

    const tree = await buildSanctionsTree(sanctioned)
    expect(result.publicSignals[0]).toBe(tree.root.toString())

    const ok = await snarkjs.groth16.verify(VK, result.publicSignals, result.snarkProof as never)
    expect(ok).toBe(true)
    // Two G1 points and one G2 point, matching the verifier's PROOF_LEN.
    expect(result.proof.length).toBe(256)
  })

  test('a prover whose address is on the list cannot produce a proof', async () => {
    const address = await nonMembershipAddress(SK)
    await expect(
      generateNonMembershipProof({ sk: SK, sanctioned: [address], wasmPath: WASM, zkeyPath: ZKEY }),
    ).rejects.toThrow('sanctions list')
  })
})
