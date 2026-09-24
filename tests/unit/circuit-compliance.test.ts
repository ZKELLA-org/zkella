/**
 * Soundness tests for circuits/compliance/non_membership.circom against the
 * real compiled circuit. The original circuit accepted non-strict bounds (a
 * listed address could use its own leaf as both neighbours), did not require
 * the two bracketing leaves to be adjacent (so a listed entry could sit
 * between them), and compared 254-bit values with a 64-bit range check (so an
 * honest address essentially never verified). These tests pin each of those.
 */
import { readFileSync } from 'fs'
import path from 'path'
import * as circomlibjs from 'circomlibjs'

// eslint-disable-next-line @typescript-eslint/no-var-requires
const buildWc = require('../../circuits/compliance/build/non_membership_js/witness_calculator.js')
const WASM = readFileSync(path.join(__dirname, '../../circuits/compliance/build/non_membership_js/non_membership.wasm'))

const D = 32
const MASK248 = (1n << 248n) - 1n
let P2: (a: bigint, b: bigint) => bigint
let emptyRoots: bigint[]

beforeAll(async () => {
  const poseidon = await circomlibjs.buildPoseidon()
  P2 = (a, b) => poseidon.F.toObject(poseidon([a, b]))
  emptyRoots = [0n]
  for (let i = 1; i <= D; i++) emptyRoots.push(P2(emptyRoots[i - 1], emptyRoots[i - 1]))
})

/** Sparse tree over `leaves` at indices 0..n-1; everything else empty. */
function tree(leaves: bigint[]) {
  const levels: Map<number, bigint>[] = [new Map(leaves.map((v, i) => [i, v]))]
  for (let l = 0; l < D; l++) {
    const next = new Map<number, bigint>()
    for (const idx of new Set([...levels[l].keys()].map(i => i >> 1))) {
      const left = levels[l].get(2 * idx) ?? emptyRoots[l]
      const right = levels[l].get(2 * idx + 1) ?? emptyRoots[l]
      next.set(idx, P2(left, right))
    }
    levels.push(next)
  }
  const root = levels[D].get(0)!
  const proof = (index: number) => {
    const path: string[] = [], bits: string[] = []
    let i = index
    for (let l = 0; l < D; l++) {
      path.push(String(levels[l].get(i ^ 1) ?? emptyRoots[l]))
      bits.push(String(i & 1))
      i >>= 1
    }
    return { path, bits }
  }
  return { root, proof }
}

function witness(sk: bigint, leaves: bigint[], lowerIdx: number, upperIdx: number) {
  const t = tree(leaves)
  const lo = t.proof(lowerIdx), up = t.proof(upperIdx)
  return {
    sk: String(sk),
    lower_leaf: String(leaves[lowerIdx]), upper_leaf: String(leaves[upperIdx]),
    lower_path: lo.path, lower_path_index: lo.bits,
    upper_path: up.path, upper_path_index: up.bits,
    sanctions_root: String(t.root), tk_commitment: String(P2(sk, 0n)),
  }
}

async function accepts(input: object): Promise<boolean> {
  const wc = await buildWc(WASM)
  try { await wc.calculateWTNSBin(input, 0); return true } catch { return false }
}

const SK = 12345n
const addr = () => P2(SK, 1n) & MASK248
const MAX = MASK248

describe('non_membership.circom (sorted-tree bracketing)', () => {
  test('an unlisted address bracketed by adjacent leaves is accepted', async () => {
    const a = addr()
    expect(await accepts(witness(SK, [0n, a - 1000n, a + 1000n, MAX], 1, 2))).toBe(true)
  })

  test('accepts the sentinel bracket when the list is empty', async () => {
    expect(await accepts(witness(SK, [0n, MAX], 0, 1))).toBe(true)
  })

  test('a listed address cannot use its own leaf as both neighbours', async () => {
    const a = addr()
    const leaves = [0n, a, a + 5n, MAX] // address is sanctioned (leaf 1)
    expect(await accepts(witness(SK, leaves, 1, 1))).toBe(false)
  })

  test('a listed address is rejected with its own leaf as the lower bound (strict)', async () => {
    const a = addr()
    expect(await accepts(witness(SK, [0n, a, a + 5n, MAX], 1, 2))).toBe(false)
  })

  test('a listed address is rejected with its own leaf as the upper bound (strict)', async () => {
    const a = addr()
    expect(await accepts(witness(SK, [0n, a - 5n, a, MAX], 1, 2))).toBe(false)
  })

  test('non-adjacent bracketing leaves are rejected (a listed entry sits between them)', async () => {
    const a = addr()
    // Sanctioned entry a+1 lies between leaf 1 (a-1000) and leaf 3 (a+1000): skipping it must fail.
    const leaves = [0n, a - 1000n, a + 1n, a + 1000n, MAX]
    expect(await accepts(witness(SK, leaves, 1, 3))).toBe(false)
  })

  test('swapped bounds (lower above address) are rejected', async () => {
    const a = addr()
    expect(await accepts(witness(SK, [0n, a - 1000n, a + 1000n, MAX], 2, 3))).toBe(false)
  })

  test('a tk_commitment that does not match sk is rejected', async () => {
    const a = addr()
    const w = witness(SK, [0n, a - 1000n, a + 1000n, MAX], 1, 2)
    w.tk_commitment = String(BigInt(w.tk_commitment) + 1n)
    expect(await accepts(w)).toBe(false)
  })
})
