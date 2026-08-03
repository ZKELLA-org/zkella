/**
 * End-to-end test of `generateUnshieldProof` itself (not just the wire-format
 * encoder — see `prover-unshield.test.ts` for that): builds a real witness
 * for a single note in an otherwise-empty depth-32 tree and generates a real
 * Groth16 proof via `snarkjs.groth16.fullProve` against the compiled
 * `unshield.circom` artifacts, going through the actual SDK code path an
 * application would use.
 *
 * `circom`'s witness calculator enforces every `<==`/`===` constraint while
 * building the witness and throws if any fail — so a `generateUnshieldProof`
 * call that resolves without throwing is already a strong signal that the
 * Merkle path genuinely reconstructs `anchor`, the nullifier genuinely
 * derives from `(nk, rho)`, and the note commitment genuinely derives from
 * `(value, asset_id, rho, rcm)`, not just that the code ran.
 */

import * as path from 'path'
import { generateUnshieldProof } from '../../sdk/src/prover/unshield'
import { computeCommitment, computeNullifier } from '../../sdk/src/notes/builder'
import { poseidon2, bigIntToBuffer, bufferToBigInt, addressToField } from '../../sdk/src/crypto/poseidon'
import { Note } from '../../sdk/src/types'

const WASM_PATH = path.join(__dirname, '../../circuits/unshield/build/unshield_js/unshield.wasm')
const ZKEY_PATH  = path.join(__dirname, '../../circuits/unshield/build/unshield.zkey')

const MERKLE_DEPTH = 32
// Real Stellar Testnet native-XLM SAC — the same address used in the three
// real live-Testnet shield() transactions (see docs/POC_IMPLEMENTATION.md).
const ASSET_ADDR = 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC'

/** Same construction as the circuits' own gen_witness.js scripts: a single
 *  real leaf at index 0 of an otherwise-empty depth-32 tree, anchor = leaf
 *  chained through the empty-subtree roots. */
async function buildSingleLeafTree(leaf: Uint8Array): Promise<{ anchor: Uint8Array; path: Uint8Array[] }> {
  const emptyRoots: Uint8Array[] = [await poseidon2(new Uint8Array(32), new Uint8Array(32))]
  for (let i = 1; i < MERKLE_DEPTH; i++) {
    emptyRoots.push(await poseidon2(emptyRoots[i - 1], emptyRoots[i - 1]))
  }
  let anchor = leaf
  for (let lvl = 0; lvl < MERKLE_DEPTH; lvl++) {
    anchor = await poseidon2(anchor, emptyRoots[lvl])
  }
  return { anchor, path: emptyRoots }
}

describe('generateUnshieldProof — end-to-end against the real compiled circuit', () => {
  test('builds a self-consistent witness snarkjs accepts, for a real note and real address', async () => {
    const value = 250000n
    const rho   = bigIntToBuffer(2222n)
    const rcm   = bigIntToBuffer(3333n)
    const nk    = bigIntToBuffer(4444n)

    const commitment = await computeCommitment(value, ASSET_ADDR, rho, rcm)
    const { anchor, path } = await buildSingleLeafTree(commitment)

    const note: Note = {
      value,
      assetId:    ASSET_ADDR,
      rho,
      rcm,
      leafIndex:  0, // single leaf, inserted first
      commitment,
    }

    const result = await generateUnshieldProof(
      { note, nk, merklePath: path },
      { anchor, recipient: ASSET_ADDR },
      WASM_PATH,
      ZKEY_PATH,
    )

    expect(result.proof.length).toBe(256)
    expect(result.publicInputsLE).toHaveLength(5)

    // nullifier = Poseidon2(nk, rho) — independently recomputed.
    const expectedNullifier = await computeNullifier(nk, rho)
    expect(result.nullifier).toEqual(expectedNullifier)
    expect(result.publicInputsLE[1]).toEqual(expectedNullifier)

    // anchor is the first public signal.
    expect(result.publicInputsLE[0]).toEqual(anchor)

    // recipient_hash = Poseidon2(address_field(recipient), 0) — independently recomputed.
    const expectedRecipientHash = await poseidon2(addressToField(ASSET_ADDR), new Uint8Array(32))
    expect(result.recipientHash).toEqual(expectedRecipientHash)
    expect(result.publicInputsLE[4]).toEqual(expectedRecipientHash)

    // pub_value / pub_asset_id are the 3rd/4th public signals.
    expect(bufferToBigInt(result.publicInputsLE[2])).toBe(value)
  }, 60_000)

  test('rejects a note with an unassigned leafIndex', async () => {
    const note: Note = {
      value: 100n,
      assetId: ASSET_ADDR,
      rho: bigIntToBuffer(1n),
      rcm: bigIntToBuffer(2n),
      leafIndex: -1, // never shielded / not yet assigned an on-chain index
      commitment: new Uint8Array(32),
    }
    await expect(
      generateUnshieldProof(
        { note, nk: bigIntToBuffer(3n), merklePath: new Array(MERKLE_DEPTH).fill(new Uint8Array(32)) },
        { anchor: new Uint8Array(32), recipient: ASSET_ADDR },
        WASM_PATH,
        ZKEY_PATH,
      ),
    ).rejects.toThrow('leafIndex')
  })

  test('rejects a malformed Merkle path length', async () => {
    const note: Note = {
      value: 100n,
      assetId: ASSET_ADDR,
      rho: bigIntToBuffer(1n),
      rcm: bigIntToBuffer(2n),
      leafIndex: 0,
      commitment: new Uint8Array(32),
    }
    await expect(
      generateUnshieldProof(
        { note, nk: bigIntToBuffer(3n), merklePath: [new Uint8Array(32)] }, // wrong length
        { anchor: new Uint8Array(32), recipient: ASSET_ADDR },
        WASM_PATH,
        ZKEY_PATH,
      ),
    ).rejects.toThrow('merklePath')
  })
})
