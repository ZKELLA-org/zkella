/**
 * Cross-validates the TypeScript SDK's Groth16 wire-format encoder against a
 * real proof from the compiled `unshield.circom` circuit — the same
 * `circuits/unshield/build/proof.json` / `verification_key.json` /
 * `public.json` already validated end-to-end in
 * `contracts/verifier/src/lib.rs`'s `verify_accepts_real_unshield_circuit_proof`
 * test (real `circom` + `snarkjs`, a genuine witness, independently
 * confirmed by `snarkjs groth16 verify`). See `tests/unit/prover.test.ts`
 * for the analogous shield check (validated against a real Testnet
 * transaction rather than just a circuit-level proof, since shield is the
 * one that's actually been submitted on-chain so far).
 *
 * Unshield's public order (confirmed against `unshield.circom`'s
 * `component main {public [...]}` declaration and `input.json`) is
 * `[anchor, nullifier, pub_value, pub_asset_id, recipient_hash,
 * change_commitment, change_value_commit]` — the last two carry the
 * change-note output (Tranche 2 Deliverable 2; see `unshield.circom`'s own
 * doc comment).
 */

import { encodeProof, encodeVerifyingKey } from '../../sdk/src/prover/encoding'
import { bigIntToBuffer } from '../../sdk/src/crypto/poseidon'
import proofJson from '../../circuits/unshield/build/proof.json'
import vkJson from '../../circuits/unshield/build/verification_key.json'
import publicJson from '../../circuits/unshield/build/public.json'

const EXPECTED_VK_HEX =
  '0b291fdaaa28add7553e94df40614c894ca8fb22a2b6b4ed7351d325cad7068e1242afa10511b208e98200b835350f44a0b2641bf06744f87f3960b79f6122880041e3d1d3043bbf9687e1c198b5fe1f3f597c26b7a97127b33b64938c49887e0c65ed7e66ecf358b07f11fc7eb9cb3ecb88ec0dcfb88c12938f1ef0fa330e601c122704e90921beaa1548ea5efcd702fa0689a866360fd874cec4d1f0507f7430573487cca5aaa0c8a3417a831694d86b1171e39d821f5d456f9be5a2d7457f198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa13c97a041f8f39163652c97cc1e867d60c888585e9ec61aa6b5578ba40ca33440f1660b491d99a1f4f4ba94cd7787ec2d39028a29518edabffda1693b20246482406d7efa17f4ea6272b757d6356a61ee5dc4eac9692d1628d4ecd778db5df8022bb0b5da4e33279c693af1392b1fd2ff373506e42fc3b9bc72cb3a2555467bb263bb7d99e03990f318d0eda7881f223919f8193b1c5847fbeb57bc20d29cd2e1fe39d75230c0a5a8e899614a03f8d559d5c978d10612b2323faeb58efa7391f185ee15b0e9f203535f14fa8108d9fac90b0593a10ca09f5d3616bcdc272c8e30bb7a91fc35fe17ce717ede8bd9ad0a3500319b0ea576656cf5151a25a3fcdf00cb1ece5551b9465174699a4478c2f03d3d5c2d50d37ef2def339a16c4cd3c921dd26d8561de1fe7dc539fe43fa2aa3ecb6d7c93357c94724e475b45745a92020ee53a6a41a9fd18fe9ac770c7fd30a795ae2d15d7b0366cc59487bc133ecc1818a9f5fc02c305920ffd2d6147cf2e01293ef0c581d6fcf7fda880f396c08b221af4b37ee35585ef336f78d21f070311789984d1be595d0f5aa7294aa6f41d9e0246a198031068ddd563eedcfa3a043aa32fba49117866cf202f9d19c960eb362792efb975c4d5fd34c15e61da02ff00f5d36f285c619a34e14a07b905c8d6d3198c8e67911bce4d76a41b3d36a7bc4ce436ce65719034e1c991172d96a90b7923cb0dc85014505eaf5880698fdf41a850602ea9917680b6dda66134a3dffc482dd9468b2fc91c7190e10aa845b8ae5f9def2b902b9c2b27e54d64700899ffd1245dec06a5747c9c776e6fb4ef43e99ccdd69c974e3424369e5a40c8bfac61342d2d196d8ea6ffc75d3166236867d16cbd249b9bd92e91446457b80ca55117fc'

const EXPECTED_PROOF_HEX =
  '2b9f7fd00b52856341777fdce6225c6fdd96ad55fa1ce8d3a660ecf0e5447c2a2174545b2d775a643644ac2deebde6128b6815a84729e2561bba19f4024b44481a99d2118612d1b413018810f439806de9ca773e36b4453de24e901f3ed9683114c3a34b2e1fd85c8d885b537db4bab6addaa20495e86cb2331589c6e8c442ca079c7d29b10c0732a84bc12f1abdc01b01f38f39ba22fc2d7627527cda4566d72b0503c76dd5f96ed06f8fee292d2a93540253de99333b4c6a45a38ac424e05c24c08bb628cf9f3d908810b3ae52a3b8d3123d32cdd671a8bf8fe640ef1a5b900c15e93dd61be8d9a145d9663700f87d751b3454bb9fec6cf910711e8e7c016f'

// [anchor, nullifier, pub_value, pub_asset_id, recipient_hash, change_commitment,
// change_value_commit] as 32-byte LE hex.
const EXPECTED_PUBLIC_INPUTS_LE_HEX = [
  '1f0d6b5ede9f49a76e976c141ff30b9e7f5fd57ef77db70c1851d787209d9f28',
  'bb59c47fd18dfa22d7e51c1e2dafc346b60b6a60e39639e15ceb43f3bbe90609',
  'a086010000000000000000000000000000000000000000000000000000000000',
  'cd81010000000000000000000000000000000000000000000000000000000000',
  '2a00000000000000000000000000000000000000000000000000000000000000',
  'dc701db3835aef89d5241c82028e61476f39baefddc08c14f3f6fbbd6bf01513',
  '18f3d3147f1a67b51b2dd7af3e9bccdf5a061fc69678f0388c7a8d510860f32a',
]

function bytesToHex(buf: Uint8Array): string {
  return Array.from(buf).map(b => b.toString(16).padStart(2, '0')).join('')
}

describe('Unshield proof wire-format encoding (SDK vs. real circuit proof)', () => {
  test('encodeProof matches a real unshield.circom proof', () => {
    const encoded = encodeProof(proofJson as any)
    expect(encoded.length).toBe(256)
    expect(bytesToHex(encoded)).toBe(EXPECTED_PROOF_HEX)
  })

  test('encodeVerifyingKey matches the real unshield.circom VK', () => {
    const encoded = encodeVerifyingKey(vkJson as any)
    expect(encoded.length).toBe(960)
    expect(bytesToHex(encoded)).toBe(EXPECTED_VK_HEX)
  })

  test('public signals, LE-encoded, match [anchor, nullifier, pub_value, pub_asset_id, recipient_hash, change_commitment, change_value_commit]', () => {
    const signals = publicJson as unknown as string[]
    const leHex = signals.map(s => bytesToHex(bigIntToBuffer(BigInt(s))))
    expect(leHex).toEqual(EXPECTED_PUBLIC_INPUTS_LE_HEX)
  })
})
