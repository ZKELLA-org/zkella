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
  '0b291fdaaa28add7553e94df40614c894ca8fb22a2b6b4ed7351d325cad7068e1242afa10511b208e98200b835350f44a0b2641bf06744f87f3960b79f6122880041e3d1d3043bbf9687e1c198b5fe1f3f597c26b7a97127b33b64938c49887e0c65ed7e66ecf358b07f11fc7eb9cb3ecb88ec0dcfb88c12938f1ef0fa330e601c122704e90921beaa1548ea5efcd702fa0689a866360fd874cec4d1f0507f7430573487cca5aaa0c8a3417a831694d86b1171e39d821f5d456f9be5a2d7457f198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c21800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa2d9cca5351120b63aa991da6bec4b807d543867c2ebc51da8ee2eba181a0d478040e060e1fedac5c6567b436890917c9f21093c0c6db108e446371a6dab2d4a419d2a654617d5bfb4f9aa088c7cfc33780d00aba13dc5e59af91ace654fcab320defa3acf67254c86dc6627f1a28a1624aa685129c26e697f3b1368c146998921488716f57136a9c6e0aea900c443e8bf4851a3b4a9de095b2bbe6ee45a0928c04fcc15f28278f536deca9a58745100a454426ad38a1ef86532dbc9150ab3c5721d875b836271817bf90fa35fc178f4e34a755dc95fb801df7917ac67871669609b798ca5c6e03e3be8e3f39305f451a4e7b6605aab3718b26a468506657b21a1fe02728ff258d1f58b1c8dec33ba170a5f4ac4adbf50ad72b4378cb8e17ee000d48c9210286aed1e336f8d33b25f02c1c91ef2681c8d70cbab0dd69aa2e0cfb1ffba34d56dd06b0a47830cf0d5687b18771651409aeed5b3530ad124ac656891fd63eef08b7d24f442835884a08a6db7d5f8cc4336a06c4b658e35b7e698f070aa3095674f8d47ff59423e7563055bff31554bab4e1a4c31fbb4ad433ba34fc1ce0f8189093bcab3693812d9c92cae91cafb077622e004b81d46e784de4bac02416127a41885c66ff7d8a98af5b52042b2f84d8883ee2593a5195a376ec2600241347b2dcb3c525784408da8d2c6012ac0ab456c68f0c5efd25e30edfcf0d3c'

const EXPECTED_PROOF_HEX =
  '0b98d2e8e4d3ed5c236deff9434f5e8e51523806efd3c31969ed2342d1a2a11a17599b42085920008a36a1d36756abcb3feca74838284e4b318eb94fbb7039071310a9439c13f4fbfdc28ebc72bf32e1b09bed4a2c6186ff2f264a125e219abf2cc93c6ca91dd49a7e21d86c4696d1af62ddf9ba6ea491fdeb20beaca93f323121274fc006e90d0fa05c3e004f877fc4b71ca7773382a31db6b161fbfc6a20f62b6b68e9eed20e906f286a99c2482999056377cb79b9a12260473eb7d62fed070aa19630eed42756d60b1c3c6dd6b3e36a19ffd1cd9d57ba211c0359136add371d0491473b8176fb4b78368cd9f8854d60c5dbdc7819d168527b1f95be4390f4'

// [anchor, nullifier, pub_value, pub_asset_id, recipient_hash] as 32-byte LE hex.
const EXPECTED_PUBLIC_INPUTS_LE_HEX = [
  '17618668a8b4ec213f956a90c17f10a496fba87093233bcb4db389ee756b1409',
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
