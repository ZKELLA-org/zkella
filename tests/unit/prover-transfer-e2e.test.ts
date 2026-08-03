/**
 * End-to-end test of `generateTransferProof` itself (not just the wire-format
 * encoder — see `prover-transfer.test.ts` for that): builds two real input
 * notes in an otherwise-empty depth-32 tree and two fresh output notes, and
 * generates a real Groth16 proof via `snarkjs.groth16.fullProve` against the
 * compiled `transfer_2in2out/transfer.circom` artifacts, going through the
 * actual SDK code path an application would use.
 *
 * As in `prover-unshield-e2e.test.ts`: `circom`'s witness calculator
 * enforces every `<==`/`===` constraint while building the witness and
 * throws if any fail, so a `generateTransferProof` call that resolves
 * without throwing is a real signal that both Merkle paths reconstruct
 * `anchor`, both nullifiers genuinely derive from `(nk, in_rho[i])`, both
 * output commitments genuinely derive from `(value, asset_id, rho, rcm)`,
 * and `sum_in === sum_out + fee` — not just that the code ran.
 */

import * as path from 'path'
import { generateTransferProof } from '../../sdk/src/prover/transfer'
import { computeCommitment, computeNullifier } from '../../sdk/src/notes/builder'
import { poseidon2, bigIntToBuffer, bufferToBigInt } from '../../sdk/src/crypto/poseidon'
import { Note } from '../../sdk/src/types'

const WASM_PATH = path.join(__dirname, '../../circuits/transfer_2in2out/build/transfer_js/transfer.wasm')
const ZKEY_PATH  = path.join(__dirname, '../../circuits/transfer_2in2out/build/transfer.zkey')

const MERKLE_DEPTH = 32
// Real Stellar Testnet native-XLM SAC — same address used in the real
// live-Testnet shield() transactions (see docs/POC_IMPLEMENTATION.md).
const ASSET_ADDR = 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC'

/** Builds a real 2-leaf depth-32 tree: leaf0 at index 0, leaf1 at index 1,
 *  the rest empty — same construction as circuits/*​/build/gen_witness.js. */
async function buildTwoLeafTree(
  leaf0: Uint8Array,
  leaf1: Uint8Array,
): Promise<{ anchor: Uint8Array; path0: Uint8Array[]; path1: Uint8Array[] }> {
  const emptyRoots: Uint8Array[] = [await poseidon2(new Uint8Array(32), new Uint8Array(32))]
  for (let i = 1; i < MERKLE_DEPTH; i++) {
    emptyRoots.push(await poseidon2(emptyRoots[i - 1], emptyRoots[i - 1]))
  }

  // Level 0: leaf0 (index 0, left) combines with leaf1 (index 1, right).
  const node0 = await poseidon2(leaf0, leaf1)
  // path for leaf0: sibling at level 0 is leaf1, then empty roots above.
  const path0 = [leaf1, ...emptyRoots.slice(0, MERKLE_DEPTH - 1)]
  // path for leaf1: sibling at level 0 is leaf0, then empty roots above.
  const path1 = [leaf0, ...emptyRoots.slice(0, MERKLE_DEPTH - 1)]

  let anchor = node0
  for (let lvl = 1; lvl < MERKLE_DEPTH; lvl++) {
    anchor = await poseidon2(anchor, emptyRoots[lvl - 1])
  }

  return { anchor, path0, path1 }
}

describe('generateTransferProof — end-to-end against the real compiled circuit', () => {
  test('builds a self-consistent witness snarkjs accepts, for two real input notes and two real outputs', async () => {
    const rho0 = bigIntToBuffer(111n)
    const rcm0 = bigIntToBuffer(222n)
    const rho1 = bigIntToBuffer(333n)
    const rcm1 = bigIntToBuffer(444n)
    const nk   = bigIntToBuffer(555n)

    const commitment0 = await computeCommitment(600000n, ASSET_ADDR, rho0, rcm0)
    const commitment1 = await computeCommitment(400000n, ASSET_ADDR, rho1, rcm1)
    const { anchor, path0, path1 } = await buildTwoLeafTree(commitment0, commitment1)

    const note0: Note = { value: 600000n, assetId: ASSET_ADDR, rho: rho0, rcm: rcm0, leafIndex: 0, commitment: commitment0 }
    const note1: Note = { value: 400000n, assetId: ASSET_ADDR, rho: rho1, rcm: rcm1, leafIndex: 1, commitment: commitment1 }

    const fee = 1000n
    const result = await generateTransferProof(
      {
        inputs: [
          { note: note0, merklePath: path0 },
          { note: note1, merklePath: path1 },
        ],
        nk,
        outputs: [
          { value: 700000n, assetId: ASSET_ADDR },
          { value: 299000n, assetId: ASSET_ADDR },
        ],
        fee,
      },
      { anchor, assetId: ASSET_ADDR },
      WASM_PATH,
      ZKEY_PATH,
    )

    expect(result.proof.length).toBe(256)
    expect(result.publicInputsLE).toHaveLength(11)

    // anchor is the first public signal.
    expect(result.publicInputsLE[0]).toEqual(anchor)

    // nullifiers[0..1] are the 2nd/3rd public signals.
    const expectedNf0 = await computeNullifier(nk, rho0)
    const expectedNf1 = await computeNullifier(nk, rho1)
    expect(result.nullifiers[0]).toEqual(expectedNf0)
    expect(result.nullifiers[1]).toEqual(expectedNf1)
    expect(result.publicInputsLE[1]).toEqual(expectedNf0)
    expect(result.publicInputsLE[2]).toEqual(expectedNf1)

    // out_commitments[0..1] are the 4th/5th public signals, and match the
    // returned output notes' own commitments.
    expect(result.publicInputsLE[3]).toEqual(result.outputNotes[0].commitment)
    expect(result.publicInputsLE[4]).toEqual(result.outputNotes[1].commitment)
    expect(result.outputNotes[0].value).toBe(700000n)
    expect(result.outputNotes[1].value).toBe(299000n)

    // fee (10th signal) and asset_id (11th signal).
    expect(bufferToBigInt(result.publicInputsLE[9])).toBe(fee)
  }, 90_000)

  test('rejects when in_value sum does not balance out_value sum + fee', async () => {
    const rho0 = bigIntToBuffer(1n)
    const rcm0 = bigIntToBuffer(2n)
    const rho1 = bigIntToBuffer(3n)
    const rcm1 = bigIntToBuffer(4n)
    const nk   = bigIntToBuffer(5n)
    const commitment0 = await computeCommitment(100n, ASSET_ADDR, rho0, rcm0)
    const commitment1 = await computeCommitment(100n, ASSET_ADDR, rho1, rcm1)

    const note0: Note = { value: 100n, assetId: ASSET_ADDR, rho: rho0, rcm: rcm0, leafIndex: 0, commitment: commitment0 }
    const note1: Note = { value: 100n, assetId: ASSET_ADDR, rho: rho1, rcm: rcm1, leafIndex: 1, commitment: commitment1 }
    const emptyPath = new Array(MERKLE_DEPTH).fill(new Uint8Array(32))

    await expect(
      generateTransferProof(
        {
          inputs: [
            { note: note0, merklePath: emptyPath },
            { note: note1, merklePath: emptyPath },
          ],
          nk,
          outputs: [
            { value: 500n, assetId: ASSET_ADDR }, // doesn't balance
            { value: 500n, assetId: ASSET_ADDR },
          ],
          fee: 0n,
        },
        { anchor: new Uint8Array(32), assetId: ASSET_ADDR },
        WASM_PATH,
        ZKEY_PATH,
      ),
    ).rejects.toThrow('in_value sum')
  })

  test('rejects an input note with an unassigned leafIndex', async () => {
    const rho = bigIntToBuffer(1n)
    const rcm = bigIntToBuffer(2n)
    const commitment = await computeCommitment(100n, ASSET_ADDR, rho, rcm)
    const note0: Note = { value: 100n, assetId: ASSET_ADDR, rho, rcm, leafIndex: -1, commitment }
    const note1: Note = { value: 100n, assetId: ASSET_ADDR, rho, rcm, leafIndex: 0, commitment }
    const emptyPath = new Array(MERKLE_DEPTH).fill(new Uint8Array(32))

    await expect(
      generateTransferProof(
        {
          inputs: [
            { note: note0, merklePath: emptyPath },
            { note: note1, merklePath: emptyPath },
          ],
          nk: bigIntToBuffer(5n),
          outputs: [
            { value: 100n, assetId: ASSET_ADDR },
            { value: 100n, assetId: ASSET_ADDR },
          ],
          fee: 0n,
        },
        { anchor: new Uint8Array(32), assetId: ASSET_ADDR },
        WASM_PATH,
        ZKEY_PATH,
      ),
    ).rejects.toThrow('leafIndex')
  })
})
