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
 * Note: `circuits/unshield/build/wire_format_output.txt` mislabels its
 * printed public-input order as "commitment, value_commit, pub_value,
 * pub_asset_id" — a stale copy-paste from the shield script. Unshield's
 * actual public order (confirmed against `unshield.circom`'s
 * `component main {public [...]}` declaration and `input.json`) is
 * `[anchor, nullifier, pub_value, pub_asset_id, recipient_hash]`; the hex
 * *values* in that file are correct, only the printed labels are wrong.
 */

import { encodeProof, encodeVerifyingKey } from '../../sdk/src/prover/encoding'
import { bigIntToBuffer } from '../../sdk/src/crypto/poseidon'
import proofJson from '../../circuits/unshield/build/proof.json'
import vkJson from '../../circuits/unshield/build/verification_key.json'
import publicJson from '../../circuits/unshield/build/public.json'

const EXPECTED_VK_HEX =
  '0b291fdaaa28add7553e94df40614c894ca8fb22a2b6b4ed7351d325cad7068e1242afa10511b208e98200b835350f44a0b2641bf06744f87f3960b79f6122880041e3d1d3043bbf9687e1c198b5fe1f3f597c26b7a97127b33b64938c49887e0c65ed7e66ecf358b07f11fc7eb9cb3ecb88ec0dcfb88c12938f1ef0fa330e601c122704e90921beaa1548ea5efcd702fa0689a866360fd874cec4d1f0507f7430573487cca5aaa0c8a3417a831694d86b1171e39d821f5d456f9be5a2d7457f198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa028dbc6d50946ff9437d5313c639902caa6ea3f0d9ee6e29bdd62b4753dee4e91190f122364363a5752b4bc8fd7aec8ec0c236eaf099e64dbb7fb60bcb05455320fcb6b768bb1d9b35ac7613310b44f94656ed5abb2760866e202634aacc882a041cdd332c6d893997436e0a9bbd4807af2cb2ab2cf125f216adce9bdc7cfa4202c369c7679f429a103a052efcab3313203a6f6b8e82c6a2ce19add77bdfc3e8206d8b89632770915133cab480bf130759db86f2569b681d4b3432cbaca9c7f22e9e9f8523925e7868673857aba734c2ca50aa02c8deb44c40792da4032affe61bc0d8c3662587ef9035e2e83f03f735af725c1700e9d36744baf92951f2cb800bad3a059a1dae4575ab11a0d943bf41304aeeca86e460aeee3857d070052af60d527d70793f946f63336b23c896d4779382858f1c52355b6768084bebc3fef22658432b1db72185e7c4501680bb94641cf70a8af2e861f19eabc5f958729f1526cbb504cc08d49409c630800e14c3b61069cf408172b807f5f362ded2d2b865170adf28043ac603ea0d3ecec766069f8d303ee75703a7cca32b494301b173ec2f798d1d6be3e3cbf1ba6298214801e727edab845a3d3a01b52ef86508e1e82e2e59431282e194d04c8a0da95a7e3c9a3c4bb897e0527ee1ced4f2c11051b56c11e28c1f1545895bd7538ad71bc99a429b32ca95df1b118c24f2537a6efb9be8'

const EXPECTED_PROOF_HEX =
  '12df8a60cfe4b8345cc56a5b00d65c7c455678526daefbcf9db86fc8ee841a9e2e234bb7a4b606d5ca6566daa8a1a1895b39ad1505d8ccc44fe089a7ed49fc99229feef33db3d57c064053bc0e99a7128f36f7fd80f5578660ab76868dbfa5f814eb9576fd0219e143e20ecba4d073def09ae9f01b26093bd0210a92e7c038eb1219827645ff0d0fef18b759d3efd47ff8da25abd5fa47f688e771398655a9e518458c8973c4d15400772a08df1abfe6f1dc732093aaaf09d556c16bc9d9de7c1ea57b7b134b14bc911907b22170190fcbfdcc31084bff2ec9e8f15eb3f9457d243283289af9809d1e7d5a001a7016af6fe6f7646a86ded24ed5a0ae162b36e5'

// [anchor, nullifier, pub_value, pub_asset_id, recipient_hash] as 32-byte LE hex.
const EXPECTED_PUBLIC_INPUTS_LE_HEX = [
  '1f0d6b5ede9f49a76e976c141ff30b9e7f5fd57ef77db70c1851d787209d9f28',
  'bb59c47fd18dfa22d7e51c1e2dafc346b60b6a60e39639e15ceb43f3bbe90609',
  '90d0030000000000000000000000000000000000000000000000000000000000',
  'cd81010000000000000000000000000000000000000000000000000000000000',
  '2a00000000000000000000000000000000000000000000000000000000000000',
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
    expect(encoded.length).toBe(832)
    expect(bytesToHex(encoded)).toBe(EXPECTED_VK_HEX)
  })

  test('public signals, LE-encoded, match [anchor, nullifier, pub_value, pub_asset_id, recipient_hash]', () => {
    const signals = publicJson as unknown as string[]
    const leHex = signals.map(s => bytesToHex(bigIntToBuffer(BigInt(s))))
    expect(leHex).toEqual(EXPECTED_PUBLIC_INPUTS_LE_HEX)
  })
})
