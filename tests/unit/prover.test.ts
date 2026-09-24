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
  '004a10ae973df76d18cc7282cb3fd87d293ac1521d0f8c3fe1b7a8bf2cc1cd3d1c1a9a34341a0bbae006c778fbfdf7e5d0ae8cac871ecf71f17f4673e5b1f56b062d17924ef71132b66fc4695c553433e7cede3848a8bda9332958709c984a0b13bc9e495aa7512ed247262baefd60f73226f7017843977bc797462aa88034970cdc3f64b84088c7343b736148da94beb5b9ed7c19d2397c25ecb783132af1292bcac674e74ffd994b4152e1347afe87ea763ffde54a274b6e0f5653ab94de91198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa06e7e22546bfcda39cc15848d8e4a41251db89eff6fa9c9452bf2807489b70971c260eee5728432e75b3394dedf096dccaceab6f37b9796f4be6721b5cd713bd1227dcce30f5763480b5eac1e2559d33a06bf084f7c384a4afc6e39b631a6222057fda0b99afccb0dc00f26ac4ef82741d910f5a2d9cb5fe4acb3d0a9c158c5607b18a6ccb9b32f0672aca99481dffe428cff645603fca1bde5574e72fa6f5d80f57f770fdd11420efcaf2695edd2c6cbba69bb7916a152509c97707b4f900f905ce89c96bd6b54d1238b2354188a5c977fae12a2c8bca7417d83e77886bcfa41e8ba1c97e5e88bec74e98ebcad73d79a74bc39dc0bd0fced5ce4a875bbcacf02c9ba37d6555f2fcfa71a91be16824000dfdb37ab858d626b9e8874f3ab58c5418a06e2d491746dbdaf92e9b7a765ca89c6eac487ac53c2e59707b6421a744e7268eca039336571751ce3735ee30868b0d55e56d701ce86736fe837f041c834225d5ab71f8c23576cb4345708dc8c165d4feb6e18164655c363eccdf53034553058181165f38851ddd4a63579e6d7fa538ba5cd317694f3acc7e9df82bddd33301892b5bf46d45777bd2ed927749b3f3c14a478a60a3fb21effdf01f0a3981c3'

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
