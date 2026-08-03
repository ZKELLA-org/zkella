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
  '0b291fdaaa28add7553e94df40614c894ca8fb22a2b6b4ed7351d325cad7068e1242afa10511b208e98200b835350f44a0b2641bf06744f87f3960b79f6122880041e3d1d3043bbf9687e1c198b5fe1f3f597c26b7a97127b33b64938c49887e0c65ed7e66ecf358b07f11fc7eb9cb3ecb88ec0dcfb88c12938f1ef0fa330e601c122704e90921beaa1548ea5efcd702fa0689a866360fd874cec4d1f0507f7430573487cca5aaa0c8a3417a831694d86b1171e39d821f5d456f9be5a2d7457f198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa1a022a8c4c2a87d64880aa9e3d089ff748c93703db1e60adaa506dac3b219ae8103ddf891d57fbfa6f0001e2fe6447bd623fa47da6800e781270d715263ea63b10bb43b6b3289dfba438d67552dcffdc17407156ad92dd8e217a74996480492721a6abc90e29003b333699172bd49ae364719dd2b91d470801752fc3c11747ba2166be7231e969fbc3be058f12e91c077a02d8a63f570a7bbf366887ee86a15901ee4fa8c3211e7bf0825e52dc01257b072e290653aa45b5fba100ae4efb4c7a042cc4a4c3313a6d2c30f8530ae70c08488b2e3c06eb5e22044d80c9bd318683089fe41e4a63cfff931618126296f878fbb8e7d02e867f56fb75e0393c05c0ab167cf0e019935756ebbe9331dc785c802f9df453077b96e70a87b1966692443e188c27fb20669a43394269ab05fd77d890855a02bb5a5096431477e25dca5b94125a0224b1a22cdc6f2f24e2a22639b475ae7677e67c10897e798b3477949238145580e95cbb6abe2f70cec376448769b6e656696ca042ac6c3ac5270926062b06d734251c7b7458de1a6b7bdc7dd9ea8ac1c47c11eae2ea376707d0afbed38208f7da7654670501dfb0fef4c430e03977d4583da19541cdb4ed42690ad860a017cdc4f5e8589bca329b168a0d590a5fc28b23e680bd71778aa36726ee3dd77a17e139ea1ca09c0d06a71d0f50199c2066ea11aafcb259ffb01b5fa91786fadf116b0f1cde75d7ee410d995c7c96e1b6575cb6a7e1f6a96266eddb6b9cc497e81a7971dd4e2fae5a68dc090f5506f31ed65c9c1805111e9072b9ec940759e9921245d0a7cee5d344d037cbf9c349e7f595099e60114138878869ebcaa7a3074f2e7cef9f88255989981defc1e21e00d5ef484115604cadc02a9adbe727ae433228921fad056b74bd862d86e834b1f10c1548158f125d05e316b81ca364480d6d0e0b598e9cc80c07bf3f867a413e103ad6361f0bce7f818bc6a09e4cca81ff13022955b6f1b3d444d2bf2a25f9ebf4c78aebb4fcb2795764e62d70eae6002947145252dacd18a1751baa375345a267c951e0602f4a81c98400cc0f66510d47521380a2da41a6226935aef250526140b4354c0df229ad2903f94643d34270ead6175bc3fb9d2a30be19c1b58ef43200184f7379eb8ce66016e29ca8c63f78b56717e864590d2ae0f68a80d944b46647e3c775511daa928a5f5763c880ac5bd8b9243a50209770f518fed800886354d7cf63a54f87b2e88c6c579a87e43edbca09'

const EXPECTED_PROOF_HEX =
  '300877e872a23340f1f0d75082fbbd500ae454a024ad4eb99133de3e564f6343145e0084b6bc59d67a38c333192b49c448e8f068dd1251605deadf29d41a151224fda90eb369051ebd0e52be53140f00013ec7290801c82dae18efaf8039c23d13972c83a3ea0d955765316e9e4c9c3791bd76b723c488db01328cdd3bf3c71f2a0349affd23aac02df48f95731424cd679c3cc76ba4c13e7dd038a04fa8b68a26cac1cb0d791894e73a7ef87a52a6166b666ac60f8aa4c7694a48abc7a23cb80a5343954637267d98cb3a07641407a2e0140a86d183b38e268e1f8379d870d417cbc698a329d9dc33d28717da2cb3e004c287a2d4c5b52c63b28990b53444e5'

// [anchor, nullifiers[0], nullifiers[1], out_commitments[0], out_commitments[1],
//  in_value_commits[0], in_value_commits[1], out_value_commits[0], out_value_commits[1],
//  fee, asset_id] as 32-byte LE hex.
const EXPECTED_PUBLIC_INPUTS_LE_HEX = [
  'b6977f2be1ff8fdfc0cf61be1c876be6433eb0f18610103e17b24104d7713d30',
  '4d8868c8b074d176055d827cfd335b5e2f811a060938c6ed26ed191ccb80e207',
  '3836e7d2d0c9902fe1a488b0b4cfe190b08f347144d1df7ec978ac8259b38f06',
  '49030caed2d8b0e5a610150752867b60844c3e7bb3b147391cc1316d65e33327',
  '768b135894509989d244f21ff91e7d554c0a778434f6ede80fa67f8df38fe12a',
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
