/**
 * Structured negative-testing pass against the real compiled shield.circom
 * circuit (Tranche 1, Deliverable 4's "shield circuit implementation" work).
 *
 * Every other shield-related test in this repo either checks the SDK's wire
 * encoding of an already-generated proof (tests/unit/prover-*.test.ts) or
 * checks that the *contract* rejects a bad proof/public-input pair
 * (contracts/verifier, contracts/token). None of them ask the more basic
 * question this file answers: does the circuit's own R1CS constraint set
 * actually reject a witness that violates one of shield.circom's
 * constraints, at witness-generation time, before a proof is ever produced?
 * A circuit with an under-constrained signal can still compile, still
 * produce a "valid" Groth16 proof (of a trivially-true or attacker-steered
 * relation), and pass every contract-level test that only ever feeds it
 * legitimately-generated proofs — the class of bug this file targets is
 * exactly the one already found and fixed once in this codebase, in
 * circuits/common/merkle.circom's unconstrained `index[i]` (see that file's
 * comment). shield.circom has no such history, but this test suite exists
 * so it, and any future changes to it, get the same scrutiny rather than
 * relying on the absence of a known incident as evidence of soundness.
 *
 * Method: use circom's own generated witness calculator
 * (circuits/shield/build/shield_js/{shield.wasm,witness_calculator.js}) —
 * the same artifact `snarkjs groth16 fullprove` uses internally — to attempt
 * to compute a full witness for a deliberately malformed input. circom
 * compiles every `<==`/`===` constraint into an assertion the witness
 * calculator checks while solving; a witness that doesn't satisfy the R1CS
 * throws ("Error: Assert Failed...") instead of returning bytes. Each
 * negative case below tampers with exactly one field relative to a known-
 * good baseline (independently confirmed acceptable first, as a positive
 * control), isolating exactly one of shield.circom's five constraints
 * (`cm_check`, `cv_check`, `value === pub_value`, `asset_id === pub_asset_id`,
 * `Range64`) per case.
 */

import { readFileSync } from 'fs'
import path from 'path'
import * as circomlibjs from 'circomlibjs'

// eslint-disable-next-line @typescript-eslint/no-var-requires
const buildWitnessCalculator = require('../../circuits/shield/build/shield_js/witness_calculator.js')

const WASM_PATH = path.join(__dirname, '../../circuits/shield/build/shield_js/shield.wasm')

type ShieldInput = {
  value: string
  asset_id: string
  rho: string
  rcm: string
  rcv: string
  commitment: string
  value_commit: string
  pub_value: string
  pub_asset_id: string
}

let P2: (a: bigint, b: bigint) => bigint

async function calculateWitness(input: ShieldInput): Promise<void> {
  const buffer = readFileSync(WASM_PATH)
  const calculator = await buildWitnessCalculator(buffer)
  // Throws on any unsatisfied R1CS constraint; we only care whether it
  // throws, not the resulting witness values.
  await calculator.calculateWTNSBin(input, 0)
}

// A real, self-consistent witness — same asset_id as
// circuits/shield/shield_test_vectors.json's v2_shield_500stroops (the real
// testnet USDC-equivalent SAC's derived field value), rho/rcm/rcv chosen
// arbitrarily like gen_witness_testnet.js's fixture.
function buildValidInput(value: bigint): ShieldInput {
  const asset_id = 44239764132731213050593584193610954592770732183378613204087781673509415785175n
  const rho = 823746192837465192837465n
  const rcm = 918273645192837465918273n
  const rcv = 102938475610293847561029n
  const commitment = P2(P2(value, asset_id), P2(rho, rcm))
  const value_commit = P2(value, rcv)
  return {
    value: value.toString(),
    asset_id: asset_id.toString(),
    rho: rho.toString(),
    rcm: rcm.toString(),
    rcv: rcv.toString(),
    commitment: commitment.toString(),
    value_commit: value_commit.toString(),
    pub_value: value.toString(),
    pub_asset_id: asset_id.toString(),
  }
}

describe('shield.circom — negative witness tests (real compiled circuit)', () => {
  beforeAll(async () => {
    const poseidon = await circomlibjs.buildPoseidon()
    const F = poseidon.F
    P2 = (a, b) => F.toObject(poseidon([a, b]))
  })

  test('positive control: a genuinely valid, self-consistent witness is accepted', async () => {
    await expect(calculateWitness(buildValidInput(500n))).resolves.toBeUndefined()
  })

  test('rejects a commitment that does not match Poseidon2(Poseidon2(value,asset_id),Poseidon2(rho,rcm))', async () => {
    const input = buildValidInput(500n)
    input.commitment = (BigInt(input.commitment) + 1n).toString()
    await expect(calculateWitness(input)).rejects.toThrow()
  })

  test('rejects a value_commit that does not match Poseidon2(value,rcv)', async () => {
    const input = buildValidInput(500n)
    input.value_commit = (BigInt(input.value_commit) + 1n).toString()
    await expect(calculateWitness(input)).rejects.toThrow()
  })

  test('rejects pub_value that diverges from the witness value (forged public amount)', async () => {
    const input = buildValidInput(500n)
    input.pub_value = '501'
    await expect(calculateWitness(input)).rejects.toThrow()
  })

  test('rejects pub_asset_id that diverges from the witness asset_id (forged public asset)', async () => {
    const input = buildValidInput(500n)
    input.pub_asset_id = (BigInt(input.pub_asset_id) + 1n).toString()
    await expect(calculateWitness(input)).rejects.toThrow()
  })

  test('rejects a value at exactly 2^64 (one past Range64\'s upper bound)', async () => {
    // commitment/value_commit are recomputed for this same out-of-range
    // value so only the Range64 constraint is exercised, not cm_check/cv_check.
    const input = buildValidInput(2n ** 64n)
    await expect(calculateWitness(input)).rejects.toThrow()
  })

  test('rejects a "negative" value (a large field element near the BN254 modulus)', async () => {
    // circom has no native negative integers; the field-arithmetic
    // equivalent of "-1" is p-1, far outside Range64's [0, 2^64) window.
    const FIELD_MODULUS = 21888242871839275222246405745257275088548364400416034343698204186575808495617n
    const asNegativeOne = FIELD_MODULUS - 1n
    const input = buildValidInput(asNegativeOne)
    await expect(calculateWitness(input)).rejects.toThrow()
  })
})
