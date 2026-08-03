/**
 * Cross-validates the TypeScript SDK's Groth16 wire-format encoder
 * (`sdk/src/prover/shield.ts`) against the exact proof and verifying key
 * actually submitted in a real, successful Stellar Testnet `shield()`
 * transaction (tx hash
 * `7969b08549258d1f4f2431d8c9655ff9a4c351614276f51b195e7f69fc20e2cb`,
 * see `docs/POC_IMPLEMENTATION.md`, "Update: live Testnet run completed").
 *
 * That proof's wire-format bytes were originally derived by
 * `circuits/shield/build/convert_to_wire_format.py` (a Python reference
 * implementation) and confirmed correct by the network actually accepting
 * the transaction. This test feeds the *same* `proof_testnet.json` /
 * `verification_key.json` / `public_testnet.json` into the TypeScript
 * encoder and asserts byte-identical output — the strongest correctness
 * check available short of submitting another live transaction, since the
 * expected bytes are known-good by construction (a real network already
 * accepted them).
 */

import { encodeProof, encodeVerifyingKey } from '../../sdk/src/prover/encoding'
import { bigIntToBuffer } from '../../sdk/src/crypto/poseidon'
import proofJson from '../../circuits/shield/build/proof_testnet.json'
import vkJson from '../../circuits/shield/build/verification_key.json'
import publicJson from '../../circuits/shield/build/public_testnet.json'

const EXPECTED_VK_HEX =
  '004a10ae973df76d18cc7282cb3fd87d293ac1521d0f8c3fe1b7a8bf2cc1cd3d1c1a9a34341a0bbae006c778fbfdf7e5d0ae8cac871ecf71f17f4673e5b1f56b062d17924ef71132b66fc4695c553433e7cede3848a8bda9332958709c984a0b13bc9e495aa7512ed247262baefd60f73226f7017843977bc797462aa88034970cdc3f64b84088c7343b736148da94beb5b9ed7c19d2397c25ecb783132af1292bcac674e74ffd994b4152e1347afe87ea763ffde54a274b6e0f5653ab94de91198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa2df369fb1a80fe5f43ce5815c9b3d0fef629d854609fde7a59490390c92214641c4a9e5de0ce34f02e9e7bd8fcb0a020f9b772727549769a13c1aa2a896c988e1fd2e8d20f42512629dbe9cd409682377127537e0b522bd586149822ca640a08124ccc6c94f84e623f6edd2ac87bc88df3c41d3abc05ffa297a1ef152accd75d15daa321aa936187619b84941b53be84c1b578522c21fdd325098db19d1eb3391b0f6021904d94598ee99139ffd5b19c192eed86af5ec5c3a119ca19688429c022ab8b7c9d621b1d4e9de842f1038c76ce2e5a5bed63961bc8bc7b19d386ccc21ed47d27a78edf33611e9bcdae16333a0af3f12236105be8f3e047d8b7081fa4114750d6ecf3c05dafe55c30c1814760e2bc9287e1734a74963c90fdf19afac81b4bb078cf97e0bc1d0498b93ad71ef3f41ec1a75fc509d8ab59e632b6a088c01c30e0a53f367cf515da53417ad5caff8e52d85d56e90129b0d2fa8ea086c4140f5e194b1fd8c120f112207e71426ac80447337645a162e1e1b116f746ea9b202fa04c7145c3b274229a2129bbbb3d165372d2fd876e66167286b430198317f310c240e4e7097770adf18390dccd4d934b42cf5ad47d390b6766b3ac5f522cf7'

const EXPECTED_PROOF_HEX =
  '167230e26db023fd5f829e04080b90108c8c39ae859a8ccf2205c74fa8800d5d0cc8f355ce0642701a4ef1d10b1dc02f9c080cc29d6c37b722e9d2b40699820e2e048ada888117d49b4fcf254a9bc50c622176bc162e58f77741c20f7a1ad1c10ffdd2779aea32cc4d758ca1872c198125b26603e274e13058d7260714904ae308f91986c9097b5ef3dd9902aebdc221a28bc35ad2c071c2d5eff488dd7f09421b35c476c4a0637088d443a34a2108a52c296c708550853cf17723b3248d03b203772b498f72e1ae5097d31f6e06c98c4d1aeeac6b759948bf2af92339696c912c150cbbdcf0aeada74a0cbc292d6e2dc4c7170bb62d4551c458a75a9e9d7889'

const EXPECTED_PUBLIC_INPUTS_LE_HEX = [
  'fe1a40c422850b8b97022d66d23575d1182e3f5350ac90080c6d9b6a24b73b07',
  '633a01f91b5bbbd5982cba842ed5780a6c5f123d724a203993de13c8057f6f05',
  '8096980000000000000000000000000000000000000000000000000000000000',
  'd5928b929a857847c81679ac631fe6ff8fa4a5b60c71fbd4ba616580ce340601',
]

function bytesToHex(buf: Uint8Array): string {
  return Array.from(buf).map(b => b.toString(16).padStart(2, '0')).join('')
}

describe('Shield proof wire-format encoding (SDK vs. real Testnet transaction)', () => {
  test('encodeProof matches the proof actually submitted on-chain', () => {
    const encoded = encodeProof(proofJson as any)
    expect(encoded.length).toBe(256)
    expect(bytesToHex(encoded)).toBe(EXPECTED_PROOF_HEX)
  })

  test('encodeVerifyingKey matches the VK actually registered on-chain', () => {
    const encoded = encodeVerifyingKey(vkJson as any)
    expect(encoded.length).toBe(768)
    expect(bytesToHex(encoded)).toBe(EXPECTED_VK_HEX)
  })

  test('public signals, LE-encoded, match the inputs actually submitted on-chain', () => {
    const signals = publicJson as unknown as string[]
    const leHex = signals.map(s => bytesToHex(bigIntToBuffer(BigInt(s))))
    expect(leHex).toEqual(EXPECTED_PUBLIC_INPUTS_LE_HEX)
  })
})
