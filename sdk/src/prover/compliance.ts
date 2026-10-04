import * as snarkjs from 'snarkjs'
import { poseidon2, bufferToBigInt, bigIntToBuffer } from '../crypto/poseidon'
import { encodeProof } from './encoding'

/**
 * Non-membership proof for `circuits/compliance/non_membership.circom`
 * (`NonMembership(32)`, public `[sanctions_root, tk_commitment]`).
 *
 * The sanctions list is a sorted depth-32 Poseidon2 tree. Its leaves are the
 * sanctioned addresses in ascending order, bracketed by sentinels 0 and
 * 2^248 - 1. A prover is not sanctioned iff two adjacent leaves bracket its
 * address strictly. The root that counts is the one the compliance contract's
 * admin has set; `sanctionsRoot` below is what that admin should publish.
 */

export const SANCTIONS_DEPTH = 32
const ADDRESS_MASK = (1n << 248n) - 1n

async function hash(a: bigint, b: bigint): Promise<bigint> {
  return bufferToBigInt(await poseidon2(bigIntToBuffer(a), bigIntToBuffer(b)))
}

/** `zeros[k]` is the root of an all-empty subtree of height k. */
async function emptySubtreeRoots(): Promise<bigint[]> {
  const zeros = [0n]
  for (let k = 0; k < SANCTIONS_DEPTH; k++) zeros.push(await hash(zeros[k], zeros[k]))
  return zeros
}

export interface SanctionsTree {
  /** Sorted leaves including the sentinels 0 and 2^248 - 1. */
  leaves: bigint[]
  /** `levels[k]` holds the populated nodes at height k; `levels[0]` is the leaves. */
  levels: bigint[][]
  root:   bigint
}

export async function buildSanctionsTree(sanctioned: bigint[]): Promise<SanctionsTree> {
  const members = [...new Set(sanctioned)]
    .filter(a => a > 0n && a < ADDRESS_MASK)
    .sort((x, y) => (x < y ? -1 : x > y ? 1 : 0))
  const leaves = [0n, ...members, ADDRESS_MASK]
  const zeros = await emptySubtreeRoots()

  const levels: bigint[][] = [leaves]
  let level = leaves
  for (let k = 0; k < SANCTIONS_DEPTH; k++) {
    const next: bigint[] = []
    for (let j = 0; j < level.length; j += 2) {
      const right = j + 1 < level.length ? level[j + 1] : zeros[k]
      next.push(await hash(level[j], right))
    }
    levels.push(next)
    level = next
  }
  return { leaves, levels, root: level[0] }
}

/** The 248-bit address a spending key is proven against. */
export async function nonMembershipAddress(sk: bigint): Promise<bigint> {
  return (await hash(sk, 1n)) & ADDRESS_MASK
}

/** Commitment the circuit binds the spending key to: Poseidon2(sk, 0). */
export async function tkCommitmentOf(sk: bigint): Promise<bigint> {
  return hash(sk, 0n)
}

export interface NonMembershipProofResult {
  proof:          Uint8Array
  snarkProof:     object
  publicSignals:  string[]
  sanctionsRoot:  Uint8Array
  tkCommitment:   Uint8Array
}

export async function generateNonMembershipProof(params: {
  sk:         bigint
  sanctioned: bigint[]
  wasmPath:   string
  zkeyPath:   string
}): Promise<NonMembershipProofResult> {
  const tree = await buildSanctionsTree(params.sanctioned)
  const address = await nonMembershipAddress(params.sk)

  let lower = -1
  for (let i = 0; i + 1 < tree.leaves.length; i++) {
    if (tree.leaves[i] < address && address < tree.leaves[i + 1]) { lower = i; break }
  }
  if (lower < 0) throw new Error('address is on the sanctions list (no strict bracketing leaves)')
  const upper = lower + 1

  const zeros = await emptySubtreeRoots()
  const pathFor = (index: number) => {
    const siblings: string[] = []
    const bits: string[] = []
    for (let k = 0; k < SANCTIONS_DEPTH; k++) {
      const node = index >> k
      const sibling = node ^ 1
      const level = tree.levels[k]
      siblings.push((sibling < level.length ? level[sibling] : zeros[k]).toString())
      bits.push(String(node & 1))
    }
    return { siblings, bits }
  }
  const lowerPath = pathFor(lower)
  const upperPath = pathFor(upper)

  const tkCommitment = await tkCommitmentOf(params.sk)
  const input = {
    sk:                 params.sk.toString(),
    lower_leaf:         tree.leaves[lower].toString(),
    upper_leaf:         tree.leaves[upper].toString(),
    lower_path:         lowerPath.siblings,
    lower_path_index:   lowerPath.bits,
    upper_path:         upperPath.siblings,
    upper_path_index:   upperPath.bits,
    sanctions_root:     tree.root.toString(),
    tk_commitment:      tkCommitment.toString(),
  }

  const { proof, publicSignals } = await snarkjs.groth16.fullProve(input, params.wasmPath, params.zkeyPath)
  return {
    proof:         encodeProof(proof),
    snarkProof:    proof,
    publicSignals,
    sanctionsRoot: bigIntToBuffer(tree.root),
    tkCommitment:  bigIntToBuffer(tkCommitment),
  }
}
