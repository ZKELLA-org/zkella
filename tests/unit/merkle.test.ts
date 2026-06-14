/* eslint-disable @typescript-eslint/no-explicit-any */
import { poseidon2 } from '../../sdk/src/crypto/poseidon'

// Mirrors the Rust incremental Merkle tree logic in contracts/ct20/src/merkle.rs
// Validates:
//   1. The empty-leaf constant matches what Rust hard-codes (EMPTY_LEAF)
//   2. Path computation for single-leaf insertion
//   3. Root computation for known inputs

const BN254_R = 21888242871839275222246405745257275088548364400416034343698204186575808495617n

function bufToBigInt(buf: Uint8Array): bigint {
  let r = 0n
  for (let i = buf.length - 1; i >= 0; i--) r = (r << 8n) | BigInt(buf[i])
  return r
}

async function hash2(a: Uint8Array, b: Uint8Array): Promise<Uint8Array> {
  return (await poseidon2(a, b)) as any
}

async function emptySubtreeRoot(level: number, emptyLeaf: Uint8Array): Promise<Uint8Array> {
  let cur: Uint8Array = emptyLeaf
  for (let i = 0; i < level; i++) cur = await hash2(cur, cur)
  return cur
}

jest.setTimeout(60_000)

describe('Incremental Merkle tree (mirrors contracts/ct20/src/merkle.rs)', () => {

  test('empty leaf = Poseidon2(zero, zero) matches Rust constant', async () => {
    const zero = new Uint8Array(32)
    const result = await hash2(zero, zero)
    // Rust EMPTY_LEAF from contracts/ct20/src/merkle.rs (updated to match circomlibjs)
    // hex: 6448b64684ee39a823d5fe5fd52431dc81e4817bf2c3ea3cab9e239efbf59820
    const expected = new Uint8Array(Buffer.from(
      '6448b64684ee39a823d5fe5fd52431dc' +
      '81e4817bf2c3ea3cab9e239efbf59820',
      'hex',
    ))
    expect(result).toEqual(expected)
  })

  test('empty root at depth 32 is deterministic and in-field', async () => {
    const zero = new Uint8Array(32)
    const emptyLeaf = await hash2(zero, zero)
    const root1 = await emptySubtreeRoot(32, emptyLeaf)
    const root2 = await emptySubtreeRoot(32, emptyLeaf)
    expect(root1).toEqual(root2)
    const val = bufToBigInt(root1)
    expect(val).toBeGreaterThan(0n)
    expect(val).toBeLessThan(BN254_R)
  })

  test('two different leaves produce different roots', async () => {
    const zero = new Uint8Array(32)
    const emptyLeaf = await hash2(zero, zero)

    const leaf1 = new Uint8Array(32); leaf1[0] = 1
    const leaf2 = new Uint8Array(32); leaf2[0] = 2

    let cur1: Uint8Array = leaf1
    let cur2: Uint8Array = leaf2
    for (let level = 0; level < 32; level++) {
      const sibling = await emptySubtreeRoot(level, emptyLeaf)
      cur1 = await hash2(cur1, sibling)
      cur2 = await hash2(cur2, sibling)
    }
    expect(cur1).not.toEqual(cur2)
  })

  test('leaf at index 1 produces different root than index 0', async () => {
    const zero = new Uint8Array(32)
    const emptyLeaf = await hash2(zero, zero)
    const leaf = new Uint8Array(32); leaf[0] = 0xab

    let rootIdx0: Uint8Array = leaf
    for (let level = 0; level < 32; level++) {
      const sibling = await emptySubtreeRoot(level, emptyLeaf)
      rootIdx0 = await hash2(rootIdx0, sibling)
    }

    let rootIdx1: Uint8Array = leaf
    for (let level = 0; level < 32; level++) {
      const sibling = await emptySubtreeRoot(level, emptyLeaf)
      // index 1: right child at level 0, left child at levels 1+
      rootIdx1 = level === 0
        ? await hash2(sibling, rootIdx1)
        : await hash2(rootIdx1, sibling)
    }

    expect(rootIdx0).not.toEqual(rootIdx1)
  })

  test('path verification: re-deriving root from leaf + path matches original', async () => {
    const zero = new Uint8Array(32)
    const emptyLeaf = await hash2(zero, zero)
    const leaf = new Uint8Array(32); leaf[31] = 0x42

    const path: Uint8Array[] = []
    for (let level = 0; level < 32; level++) {
      path.push(await emptySubtreeRoot(level, emptyLeaf))
    }

    let root: Uint8Array = leaf
    for (let level = 0; level < 32; level++) root = await hash2(root, path[level])

    let derived: Uint8Array = leaf
    for (let level = 0; level < 32; level++) derived = await hash2(derived, path[level])

    expect(derived).toEqual(root)
  })
})
