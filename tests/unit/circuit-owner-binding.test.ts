/**
 * Regression tests for the audit finding that the nullifier key `nk` was not
 * bound to the note being spent. Before the owner key was committed into
 * every note, `nk` was a free private input: a prover could pick a fresh `nk`
 * for each spend, getting a fresh nullifier `Poseidon2(nk, rho)` for the SAME
 * note (unbounded double spend), and anyone who knew a note's plaintext could
 * spend it. Each note now commits to `pk = Poseidon2(nk, DOMAIN_PK)` and the
 * spend circuits derive `pk` from `nk`, so the only `nk` that opens a note's
 * Merkle leaf is its owner's.
 */
import { readFileSync } from 'fs'
import path from 'path'
import * as circomlibjs from 'circomlibjs'

// eslint-disable-next-line @typescript-eslint/no-var-requires
const build = (dir: string) => require(`../../circuits/${dir}/build/${dir === 'unshield' ? 'unshield_js' : 'transfer_js'}/witness_calculator.js`)
const wasm = (dir: string, name: string) =>
  readFileSync(path.join(__dirname, `../../circuits/${dir}/build/${dir === 'unshield' ? 'unshield_js' : 'transfer_js'}/${name}.wasm`))

const DOMAIN_PK = 2258241487740017274987n
const D = 32
let P2: (a: bigint, b: bigint) => bigint
let emptyRoots: bigint[]

beforeAll(async () => {
  const poseidon = await circomlibjs.buildPoseidon()
  P2 = (a, b) => poseidon.F.toObject(poseidon([a, b]))
  emptyRoots = [P2(0n, 0n)]
  for (let i = 1; i <= D; i++) emptyRoots.push(P2(emptyRoots[i - 1], emptyRoots[i - 1]))
})

const ownerKey = (nk: bigint) => P2(nk, DOMAIN_PK)

/** Unshield witness for a note owned by `ownerNk`, spent using `spendNk`. */
function unshieldInput(ownerNk: bigint, spendNk: bigint) {
  const value = 250000n, asset = 98765n, rho = 2222n, rcm = 3333n
  const leaf = P2(P2(P2(value, asset), P2(rho, rcm)), ownerKey(ownerNk))
  let node = leaf
  for (let l = 0; l < D; l++) node = P2(node, emptyRoots[l])
  return {
    value: String(value), asset_id: String(asset), rho: String(rho), rcm: String(rcm),
    nk: String(spendNk),
    path: emptyRoots.slice(0, D).map(String), path_index: new Array(D).fill('0'),
    anchor: String(node), nullifier: String(P2(spendNk, rho)),
    pub_value: String(value), pub_asset_id: String(asset), recipient_hash: '42',
  }
}

async function accepts(input: object): Promise<boolean> {
  const wc = await build('unshield')(wasm('unshield', 'unshield'))
  try { await wc.calculateWTNSBin(input, 0); return true } catch (e) { if (process.env.DEBUG_CIRCUIT) console.log(String(e).slice(0, 300)); return false }
}

describe('owner-key binding (audit: nk was not bound to the note)', () => {
  test('positive control: the owner can spend with their own nk', async () => {
    expect(await accepts(unshieldInput(4444n, 4444n))).toBe(true)
  })

  test('a different nk (with a matching fresh nullifier) cannot spend the note', async () => {
    // This is the double-spend / theft attack: same note, arbitrary nk, valid
    // fresh nullifier Poseidon2(nk', rho). It must fail at the Merkle check.
    expect(await accepts(unshieldInput(4444n, 5555n))).toBe(false)
  })

  test('the nullifier is still tied to nk (a wrong nullifier is rejected)', async () => {
    const input = unshieldInput(4444n, 4444n)
    input.nullifier = String(BigInt(input.nullifier) + 1n)
    expect(await accepts(input)).toBe(false)
  })
})
