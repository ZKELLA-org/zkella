/**
 * Cross-validates the TypeScript SDK's Groth16 wire-format encoder against a
 * real proof from the compiled `transfer_2in2out/transfer.circom` circuit —
 * the same `circuits/transfer_2in2out/build/proof.json` /
 * `verification_key.json` / `public.json` already validated end-to-end in
 * `contracts/verifier/src/lib.rs`'s
 * `verify_accepts_real_transfer_2in2out_circuit_proof` test.
 *
 * Note: like `circuits/unshield/build/wire_format_output.txt`,
 * `circuits/transfer_2in2out/build/wire_format_output.txt` mislabels its
 * printed public-input order as "commitment, value_commit, pub_value,
 * pub_asset_id" (a stale copy-paste from the shield script). The actual
 * order, confirmed against `transfer.circom`'s `component main {public [...]}`
 * declaration and `public.json`, is `[anchor, nullifiers[0], nullifiers[1],
 * out_commitments[0], out_commitments[1], in_value_commits[0],
 * in_value_commits[1], out_value_commits[0], out_value_commits[1], fee,
 * asset_id]` — 11 signals. The hex *values* in that file are correct, only
 * the printed labels are wrong.
 */

import { encodeProof, encodeVerifyingKey } from '../../sdk/src/prover/encoding'
import { bigIntToBuffer } from '../../sdk/src/crypto/poseidon'
import proofJson from '../../circuits/transfer_2in2out/build/proof.json'
import vkJson from '../../circuits/transfer_2in2out/build/verification_key.json'
import publicJson from '../../circuits/transfer_2in2out/build/public.json'

const EXPECTED_VK_HEX =
  '0b291fdaaa28add7553e94df40614c894ca8fb22a2b6b4ed7351d325cad7068e1242afa10511b208e98200b835350f44a0b2641bf06744f87f3960b79f6122880041e3d1d3043bbf9687e1c198b5fe1f3f597c26b7a97127b33b64938c49887e0c65ed7e66ecf358b07f11fc7eb9cb3ecb88ec0dcfb88c12938f1ef0fa330e601c122704e90921beaa1548ea5efcd702fa0689a866360fd874cec4d1f0507f7430573487cca5aaa0c8a3417a831694d86b1171e39d821f5d456f9be5a2d7457f198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa1effac1381d80d2415c69e56f0a628a4dc55f13b090eb3bd86de552ae9fdd68209d75da5ec727bcc8b76590d657c4b5237829ba299c3322a0e6a2b86d585550329b3b2689301966d574b1aa5445816ccdbdd0158743673efd38dbe8489552eca0f48ba739892a9729c21a0708f990a0e7d35d14969e19e5e36a793171ae93020273ab9561cac80aead7e6b6f053cc8fcf4b9446595dd3a3e1f9a444931bce03b1ffcdd49f31f6c80ccd8ffb85aa0ed72b45b0dc616becf8fdb167c2456fca43c060493e0a3b49513753c2d641bb23986e16ed42d2f8c4daa01764a0ca990b3f8157873b3f23167c5e0e7622e1b000197274b0083842bb55d2afbe3c9bea2ba610c2ee8a9f2f18a16688c010fc53f630e04ee576e53cf9729b08318f9bcd9fa5a1814b9e48f1d2a0e4c915d211c82568569ffdb61e94c8716970f22826f8220621b0d7df9e1ba7f82567dd747745625bc2759f95a0ece2421c7dec1682f5323ca28087709c7ba83b6a8d50e0fd195c1bb8e0337a5a11b2c27d3ae49ce66ccf1db09dda3b5e7df8eae2db5b8f693173719cabf64a4ae91d631fab91f2a4d6fd3dc062035efc02a059c13e283812a6febfffdd0ee682a8461d0f9f143fd7bd568b60d702be3daabf1cfd9c90c6cb9b02774d1e40db7a91196866b27a48956c7e32315f9975a0e05fa76e50cff8fbb704e7ab44e2a940bbd83688934a27766fbb7e50a377b4de6e265263a317a3e0894bc58bfa0f13223366f75d931e36cfbc1bfea2e8503a342af59ef67dd679e1ce42fdaf8ced860963b931282028129a6679ccc1f8f5828b13838959e90627bc80afcb89bb72ea25d1b2ce9b32458ea414ce4682fa5ba9b4cc2b85d574ce8cfc3d1a6437dcf269bae92498af48cba31df2739a202785747518aa6baebd7b9d060766161c2c7b3d7cefea33fae02783221d75ad71ae880161a58d9eaa65d929aef9d7a3ddca1788cf4c2981a141df7dedfa1016c137102928554207f56b1fca8871358d0dbc1a1f18ac532081d72f4c4a552e5f510ebd92e091535d619f2fcff060d1fed1899d44fa57d3178a882ea5481d9d80a1061a5bbc9834d107926c40c5149b3a8932e4f61c60e0c5b3a78a99212268bfc1cd747b7973e73cdd8d8824ac0e6472418b7a3c9666407402f34f4c63de0c467086000275e337fe4ec0b6d22f852a46ed480939d611f9e9e15c1d6e03dfb2bcd0f5884cf91fe4b50b826f502ec0d8936984a7819bec268f2678c2154149c8f52'

const EXPECTED_PROOF_HEX =
  '2fbe7b8f55c3f1653606bcff3e96cd736c0482bb3c7620986523ab586921c938171c7917e33830460601fc4b2158cca5340d152c64eac68d8bbadbeaf5ba86d11c29bc6a3224ea93e33d39dcf3bd65aabf7f5d1277d17372125f838e53824fdf02587a6b3437cdc40690a6c4fa6d8d0d634736f9a74cebb17fe63c781757175f1b503278c3e99cdcf3da58a5e787911a05cd060d31aa4e0b777edf0a10395abe1f001998f88a31a0af3cc95890c601fcfc6c4f73f6254d4c10e5f4ff700a1f15233a185f875bee3ed05188af6fa6d47c5159a774c9703e922d95c9659741d611141f536e639216db09e391caa428b491d8760d4c7cbbae4c75bfe425efd81829'

// [anchor, nullifiers[0], nullifiers[1], out_commitments[0], out_commitments[1],
//  in_value_commits[0], in_value_commits[1], out_value_commits[0], out_value_commits[1],
//  fee, asset_id] as 32-byte LE hex.
const EXPECTED_PUBLIC_INPUTS_LE_HEX = [
  '5eacddf54c17bb9aee65b72bf9c5b64bfc8d87b785d2e160d072d95c14df2004',
  '4d8868c8b074d176055d827cfd335b5e2f811a060938c6ed26ed191ccb80e207',
  '3836e7d2d0c9902fe1a488b0b4cfe190b08f347144d1df7ec978ac8259b38f06',
  '9c599bd0cd9a7e17459988299ef52e8fb34df9c81eab2536f07422fc60cc591b',
  '5c9b38d8eab1c4826d7efa7edaca9dd28133063a29af6e788d7629f6af200c0e',
  'c0e8993966a165503afa2c0bf36c7027554086cdfd42576860efe61126eccf17',
  'ac00bbc2abc4158b312b88fe617ea54d47b2c0efdbfcbb1a0a414efc8cc07001',
  '61a689b735be1fa57a76b57f35820a8ef40ea3fa9c34c3499a82eec1c5f41516',
  'f9ad173388a30002a504d1bf17773e57980c9604c63655bc02bdc4ec2e7f5117',
  '0000000000000000000000000000000000000000000000000000000000000000',
  '3930000000000000000000000000000000000000000000000000000000000000',
]

function bytesToHex(buf: Uint8Array): string {
  return Array.from(buf).map(b => b.toString(16).padStart(2, '0')).join('')
}

describe('Transfer (2-in-2-out) proof wire-format encoding (SDK vs. real circuit proof)', () => {
  test('encodeProof matches a real transfer.circom proof', () => {
    const encoded = encodeProof(proofJson as any)
    expect(encoded.length).toBe(256)
    expect(bytesToHex(encoded)).toBe(EXPECTED_PROOF_HEX)
  })

  test('encodeVerifyingKey matches the real transfer.circom VK', () => {
    const encoded = encodeVerifyingKey(vkJson as any)
    expect(encoded.length).toBe(1216)
    expect(bytesToHex(encoded)).toBe(EXPECTED_VK_HEX)
  })

  test('public signals, LE-encoded, match the 11-entry circuit order', () => {
    const signals = publicJson as unknown as string[]
    expect(signals).toHaveLength(11)
    const leHex = signals.map(s => bytesToHex(bigIntToBuffer(BigInt(s))))
    expect(leHex).toEqual(EXPECTED_PUBLIC_INPUTS_LE_HEX)
  })
})
