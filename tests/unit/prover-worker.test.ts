/**
 * Tests for `sdk/src/prover/worker.ts` (Tranche 1, Deliverable 5's "move
 * witness generation off the main JavaScript thread via a Web Worker" work).
 *
 * What this file does NOT do, and why: `worker.ts`'s intended runtime is a
 * real browser Web Worker (loaded by a bundler via `new Worker(new
 * URL(...))`). This repository has no browser wallet UI and no bundler
 * configured, so there is no page whose responsiveness could be checked,
 * and no way to execute this file as an actual Worker from Jest/Node.
 *
 * Node's own `worker_threads` was tried first and found to be a dead end,
 * not just untested: `snarkjs`'s `ffjavascript` dependency pulls in the
 * `web-worker` package, whose Node implementation branches on
 * `worker_threads.isMainThread` at *module-import time*. Importing
 * anything that reaches `snarkjs` (as `worker.ts` does, to call the real
 * generate*Proof functions) from inside a `worker_threads.Worker` crashes
 * immediately — before any message is even handled — with `Cannot
 * destructure property 'mod' of 'threads.workerData'`. This was confirmed
 * directly with a minimal repro (spawning a worker_threads.Worker that
 * does nothing but `require('web-worker')`) before concluding it wasn't a
 * bug in this SDK's own code. See `worker.ts`'s doc comment for the full
 * explanation and why a real browser (via a bundler's "browser" package
 * export resolution) sidesteps it entirely.
 *
 * What IS tested here, on the main thread: `worker.ts`'s message-dispatch
 * logic is correct (the right generate*Proof function runs for each
 * `ProverWorkerRequest.kind`, and its result matches a direct call), and
 * `self.onmessage`/`postMessage` are wired correctly under a minimal mock
 * of a Worker global scope. That leaves genuine off-thread execution and
 * real-browser compatibility unverified — flagged here rather than
 * silently assumed.
 */

import { buildNote } from '../../sdk/src/notes/builder'
import { generateShieldProof, ShieldPublicInputs, ShieldProofResult } from '../../sdk/src/prover/shield'
import { handle, ProverWorkerRequest, ProverWorkerResponse } from '../../sdk/src/prover/worker'
import * as path from 'path'

const WASM_PATH = path.join(__dirname, '../../circuits/shield/build/shield_js/shield.wasm')
const ZKEY_PATH = path.join(__dirname, '../../circuits/shield/build/shield.zkey')
const ASSET = 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC'
const AMOUNT = 500n

describe('prover worker dispatch (sdk/src/prover/worker.ts)', () => {
  test('handle() routes a "shield" request to generateShieldProof and returns a consistent result', async () => {
    const note = await buildNote(AMOUNT, ASSET)
    const publicInputs: ShieldPublicInputs = { commitment: note.commitment, asset: ASSET, amount: AMOUNT }

    const direct = await generateShieldProof(note, publicInputs, WASM_PATH, ZKEY_PATH)
    const request: ProverWorkerRequest = {
      kind: 'shield',
      args: [note, publicInputs, WASM_PATH, ZKEY_PATH],
    }
    const viaHandle = (await handle(request)) as ShieldProofResult

    // Two independent Groth16 proofs of the same statement aren't
    // byte-identical (generateShieldProof draws a fresh random `rcv` each
    // call) — what must match is the deterministic public data.
    expect(viaHandle.publicInputsLE[0]).toEqual(direct.publicInputsLE[0]) // commitment
    expect(viaHandle.publicInputsLE[2]).toEqual(direct.publicInputsLE[2]) // pub_value
    expect(viaHandle.publicInputsLE[3]).toEqual(direct.publicInputsLE[3]) // pub_asset_id
    expect(viaHandle.proof.length).toBe(direct.proof.length)
  }, 30_000)

  test('generateShieldProof with singleThread:true still produces a proof snarkjs itself accepts', async () => {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const snarkjs = require('snarkjs')
    const vkJson = require('../../circuits/shield/build/verification_key.json')

    const note = await buildNote(AMOUNT, ASSET)
    const publicInputs: ShieldPublicInputs = { commitment: note.commitment, asset: ASSET, amount: AMOUNT }

    const result = await generateShieldProof(note, publicInputs, WASM_PATH, ZKEY_PATH, true)

    // Reconstruct the {proof, publicSignals} shape snarkjs.groth16.verify
    // expects from this SDK's own wire-format output, so this is a real
    // independent check that a singleThread proof is still cryptographically
    // valid, not just "didn't throw".
    const publicSignals = result.publicInputsLE.map(bytesToDecimalString)
    const proofJson = decodeGroth16Proof(result.proof)
    const valid = await snarkjs.groth16.verify(vkJson, publicSignals, proofJson)
    expect(valid).toBe(true)
  }, 30_000)

  test('self.onmessage/postMessage are wired to handle() under a mocked Worker global scope', async () => {
    const note = await buildNote(AMOUNT, ASSET)
    const publicInputs: ShieldPublicInputs = { commitment: note.commitment, asset: ASSET, amount: AMOUNT }

    const posted: ProverWorkerResponse[] = []
    ;(globalThis as unknown as { self: unknown }).self = {
      onmessage: null as ((event: { data: ProverWorkerRequest }) => void) | null,
      postMessage: (response: ProverWorkerResponse) => { posted.push(response) },
    }

    // Re-require after `self` exists so worker.ts's top-level
    // `if (typeof self !== 'undefined')` wiring runs against the mock.
    jest.resetModules()
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const workerModule = require('../../sdk/src/prover/worker')
    const mockSelf = (globalThis as unknown as {
      self: { onmessage: ((event: { data: ProverWorkerRequest }) => void) | null }
    }).self

    expect(mockSelf.onmessage).not.toBeNull()
    await mockSelf.onmessage!({
      data: { kind: 'shield', args: [note, publicInputs, WASM_PATH, ZKEY_PATH] },
    })

    expect(posted).toHaveLength(1)
    expect(posted[0].ok).toBe(true)
    void workerModule
    delete (globalThis as unknown as { self?: unknown }).self
  }, 30_000)
})

function bytesToDecimalString(bytes: Uint8Array): string {
  let n = 0n
  for (let i = bytes.length - 1; i >= 0; i--) n = (n << 8n) | BigInt(bytes[i])
  return n.toString()
}

// Inverse of sdk/src/prover/encoding.ts's encodeProof, just enough to feed
// snarkjs.groth16.verify (which wants decimal-string coordinates, not this
// contract's raw wire bytes).
function decodeGroth16Proof(wire: Uint8Array) {
  const be = (b: Uint8Array) => {
    let n = 0n
    for (const byte of b) n = (n << 8n) | BigInt(byte)
    return n.toString()
  }
  const a = wire.slice(0, 64)
  const b = wire.slice(64, 192)
  const c = wire.slice(192, 256)
  return {
    pi_a: [be(a.slice(0, 32)), be(a.slice(32, 64)), '1'],
    pi_b: [
      [be(b.slice(32, 64)), be(b.slice(0, 32))],
      [be(b.slice(96, 128)), be(b.slice(64, 96))],
      ['1', '0'],
    ],
    pi_c: [be(c.slice(0, 32)), be(c.slice(32, 64)), '1'],
    protocol: 'groth16',
    curve: 'bn128',
  }
}
