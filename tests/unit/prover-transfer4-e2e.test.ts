/**
 * End-to-end test of `generateTransfer4Proof` itself (not just the
 * wire-format encoder — see `prover-transfer4.test.ts` for that): builds
 * four real input notes in an otherwise-empty depth-32 tree and four fresh
 * output notes, and generates a real Groth16 proof via
 * `snarkjs.groth16.fullProve` against the compiled
 * `transfer_4in4out/transfer.circom` artifacts.
 *
 * As in the 2-in-2-out e2e test: `circom`'s witness calculator enforces
 * every `<==`/`===` constraint while building the witness and throws if any
 * fail, so a `generateTransfer4Proof` call that resolves without throwing is
 * a real signal that all four Merkle paths reconstruct `anchor`, all four
 * nullifiers genuinely derive from `(nk, in_rho[i])`, all four output
 * commitments genuinely derive from `(value, asset_id, rho, rcm)`, and
 * `sum_in === sum_out + fee` — not just that the code ran.
 */

import * as path from 'path'
import { generateTransfer4Proof } from '../../sdk/src/prover/transfer4'
import { computeCommitment, computeNullifier } from '../../sdk/src/notes/builder'
import { poseidon2, bigIntToBuffer, bufferToBigInt } from '../../sdk/src/crypto/poseidon'
import { Note } from '../../sdk/src/types'

const WASM_PATH = path.join(__dirname, '../../circuits/transfer_4in4out/build/transfer_js/transfer.wasm')
const ZKEY_PATH  = path.join(__dirname, '../../circuits/transfer_4in4out/build/transfer.zkey')

const MERKLE_DEPTH = 32
const ASSET_ADDR = 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC'

/** Builds a real 4-leaf depth-32 tree at indices 0-3, the rest empty — same
 *  construction as circuits/transfer_4in4out/build/gen_witness.js. */
async function buildFourLeafTree(leaves: [Uint8Array, Uint8Array, Uint8Array, Uint8Array]) {
  const emptyRoots: Uint8Array[] = [await poseidon2(new Uint8Array(32), new Uint8Array(32))]
  for (let i = 1; i < MERKLE_DEPTH; i++) {
    emptyRoots.push(await poseidon2(emptyRoots[i - 1], emptyRoots[i - 1]))
  }

  const node01 = await poseidon2(leaves[0], leaves[1])
  const node23 = await poseidon2(leaves[2], leaves[3])
  let anchor = await poseidon2(node01, node23)
  for (let lvl = 2; lvl < MERKLE_DEPTH; lvl++) {
    anchor = await poseidon2(anchor, emptyRoots[lvl - 1])
  }

  const tail = emptyRoots.slice(1, MERKLE_DEPTH - 1)
  const paths: [Uint8Array[], Uint8Array[], Uint8Array[], Uint8Array[]] = [
    [leaves[1], node23, ...tail],
    [leaves[0], node23, ...tail],
    [leaves[3], node01, ...tail],
    [leaves[2], node01, ...tail],
  ]

  return { anchor, paths }
}

describe('generateTransfer4Proof — end-to-end against the real compiled circuit', () => {
  test('builds a self-consistent witness snarkjs accepts, for four real input notes and four real outputs', async () => {
    const rhos = [111n, 333n, 555n, 777n].map(bigIntToBuffer)
    const rcms = [222n, 444n, 666n, 888n].map(bigIntToBuffer)
    const values = [300000n, 200000n, 150000n, 100000n]
    const nk = bigIntToBuffer(555n)

    const commitments = await Promise.all(
      values.map((v, i) => computeCommitment(v, ASSET_ADDR, rhos[i], rcms[i])),
    ) as [Uint8Array, Uint8Array, Uint8Array, Uint8Array]
    const { anchor, paths } = await buildFourLeafTree(commitments)

    const notes: Note[] = values.map((v, i) => ({
      value: v,
      assetId: ASSET_ADDR,
      rho: rhos[i],
      rcm: rcms[i],
      leafIndex: i,
      commitment: commitments[i],
    }))

    const outValues = [250000n, 200000n, 150000n, 149000n]
    const fee = 1000n

    const result = await generateTransfer4Proof(
      {
        inputs: [0, 1, 2, 3].map(i => ({ note: notes[i], merklePath: paths[i] })) as any,
        nk,
        outputs: outValues.map(v => ({ value: v, assetId: ASSET_ADDR })) as any,
        fee,
      },
      { anchor, assetId: ASSET_ADDR },
      WASM_PATH,
      ZKEY_PATH,
    )

    expect(result.proof.length).toBe(256)
    expect(result.publicInputsLE).toHaveLength(19)
    expect(result.publicInputsLE[0]).toEqual(anchor)

    for (let i = 0; i < 4; i++) {
      const expectedNf = await computeNullifier(nk, rhos[i])
      expect(result.nullifiers[i]).toEqual(expectedNf)
      expect(result.publicInputsLE[1 + i]).toEqual(expectedNf)
    }
    for (let i = 0; i < 4; i++) {
      expect(result.publicInputsLE[5 + i]).toEqual(result.outputNotes[i].commitment)
      expect(result.outputNotes[i].value).toBe(outValues[i])
    }

    expect(bufferToBigInt(result.publicInputsLE[17])).toBe(fee)
  }, 120_000)

  test('rejects when in_value sum does not balance out_value sum + fee', async () => {
    const rho = bigIntToBuffer(1n)
    const rcm = bigIntToBuffer(2n)
    const commitment = await computeCommitment(100n, ASSET_ADDR, rho, rcm)
    const note: Note = { value: 100n, assetId: ASSET_ADDR, rho, rcm, leafIndex: 0, commitment }
    const emptyPath = new Array(MERKLE_DEPTH).fill(new Uint8Array(32))

    await expect(
      generateTransfer4Proof(
        {
          inputs: [0, 1, 2, 3].map(i => ({ note: { ...note, leafIndex: i }, merklePath: emptyPath })) as any,
          nk: bigIntToBuffer(5n),
          outputs: [1000n, 1000n, 1000n, 1000n].map(v => ({ value: v, assetId: ASSET_ADDR })) as any, // doesn't balance
          fee: 0n,
        },
        { anchor: new Uint8Array(32), assetId: ASSET_ADDR },
        WASM_PATH,
        ZKEY_PATH,
      ),
    ).rejects.toThrow('in_value sum')
  })
})
