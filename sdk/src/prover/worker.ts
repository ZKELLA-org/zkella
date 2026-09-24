/**
 * Web Worker entry point for proof generation.
 *
 * `generateShieldProof`/`generateTransferProof`/`generateTransfer4Proof`/
 * `generateUnshieldProof`/`generateSwapFairnessProof` all run a real Groth16
 * witness calculation + proving pass via snarkjs — CPU-bound work that, run
 * on a UI's own thread, blocks that thread for the duration (hundreds of
 * milliseconds to low seconds depending on the circuit — transfer4's 4x4
 * circuit is the largest). This file is meant to be loaded as a real
 * browser Web Worker script — e.g. via a bundler's `new Worker(new
 * URL('./worker', import.meta.url))` (Vite) or `new Worker(new
 * URL('./worker.js', import.meta.url))` (webpack 5) — so that work runs on
 * a separate thread and the page stays responsive while a proof is
 * generated. It uses the standard `self.onmessage`/`postMessage` Worker
 * protocol, not Node's `worker_threads`, deliberately:
 *
 * `snarkjs`'s field-arithmetic backend (`ffjavascript`) depends on the
 * `web-worker` package to build its own internal multi-exponentiation
 * thread pool. `web-worker`'s Node implementation decides its behavior via
 * `worker_threads.isMainThread` *at module-import time* — so merely
 * importing anything that pulls in `snarkjs` (as this file's
 * generate*Proof imports below do) inside a `worker_threads.Worker`
 * crashes immediately, before any message is even handled:
 * `TypeError: Cannot destructure property 'mod' of 'threads.workerData' as
 * it is undefined` (confirmed directly against `web-worker@1.2.0` — see
 * `node_modules/web-worker/cjs/node.js`'s unconditional `threads.isMainThread
 * ? mainThread() : workerThread()` at the top of the file). There is no
 * workaround for this short of patching that dependency: it fires on
 * import, before this file's own code runs, and `singleThread` proving
 * options (see `generateShieldProof`'s doc comment) don't help because
 * they only change what happens *after* the crashing import already ran.
 *
 * A real browser has no such problem: `web-worker`'s package.json maps the
 * `"browser"` condition to a different implementation
 * (`cjs/browser.js`) that just uses the DOM's native `Worker` global
 * directly, with no `isMainThread` logic — so a bundler configured for a
 * browser target resolves straight past the code path that crashes under
 * Node. It is exercised in a real browser by `scripts/browser_worker_check.mjs`
 * (esbuild bundle + headless Chromium): with a real shield proof, the page's
 * main thread stalled for ~205ms when proving inline but only ~19ms when the
 * same proof ran in this worker, so the page stays responsive.
 */
import { generateShieldProof } from './shield'
import { generateTransferProof } from './transfer'
import { generateTransfer4Proof } from './transfer4'
import { generateUnshieldProof } from './unshield'
import { generateSwapFairnessProof } from './swapFairness'

export type ProverWorkerRequest =
  | { kind: 'shield';       args: Parameters<typeof generateShieldProof> }
  | { kind: 'transfer';     args: Parameters<typeof generateTransferProof> }
  | { kind: 'transfer4';    args: Parameters<typeof generateTransfer4Proof> }
  | { kind: 'unshield';     args: Parameters<typeof generateUnshieldProof> }
  | { kind: 'swapFairness'; args: Parameters<typeof generateSwapFairnessProof> }

export type ProverWorkerResponse =
  | { ok: true; result: unknown }
  | { ok: false; error: string }

export async function handle(request: ProverWorkerRequest): Promise<unknown> {
  switch (request.kind) {
    case 'shield':       return generateShieldProof(...request.args)
    case 'transfer':     return generateTransferProof(...request.args)
    case 'transfer4':    return generateTransfer4Proof(...request.args)
    case 'unshield':     return generateUnshieldProof(...request.args)
    case 'swapFairness': return generateSwapFairnessProof(...request.args)
  }
}

// Minimal ambient shape instead of TypeScript's `WebWorker` lib (which
// can't be combined with the `DOM` lib this package already compiles
// against — the two declare conflicting globals). `self` itself is a real
// global wherever this actually runs (a Worker's global scope); reached via
// `globalThis` (not the bare identifier `self`, which isn't declared under
// every `lib` configuration this file gets compiled under — e.g. the test
// suite's own tsconfig omits `DOM`) and cast only to narrow its assumed
// shape for this file's own use.
type WorkerSelf = {
  onmessage:   ((event: { data: ProverWorkerRequest }) => void) | null
  postMessage: (response: ProverWorkerResponse) => void
}

const globalSelf = (globalThis as unknown as { self?: WorkerSelf }).self

if (typeof globalSelf !== 'undefined') {
  const workerSelf = globalSelf
  workerSelf.onmessage = async (event) => {
    try {
      const result = await handle(event.data)
      workerSelf.postMessage({ ok: true, result })
    } catch (err) {
      workerSelf.postMessage({
        ok:    false,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }
}
