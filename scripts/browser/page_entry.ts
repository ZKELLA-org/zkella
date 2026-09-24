// Bundled for the browser by ../browser_worker_check.mjs. Measures how long the page's main
// thread is blocked while a real shield proof is generated (a) on the main thread and (b) in the
// Web Worker built from sdk/src/prover/worker.ts.
import { buildNote, computeOwnerKey } from '../../sdk/src/notes/builder'
import { generateShieldProof } from '../../sdk/src/prover/shield'
import { bigIntToBuffer } from '../../sdk/src/crypto/poseidon'

const WASM = '/circuits/shield/build/shield_js/shield.wasm'
const ZKEY = '/circuits/shield/build/shield.zkey'
const ASSET = 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC'

// Longest gap between 10ms timer ticks while `work` runs = worst main-thread stall.
async function measure(work: () => Promise<unknown>) {
  let last = performance.now(), worst = 0, ticks = 0
  const timer = setInterval(() => {
    const now = performance.now()
    worst = Math.max(worst, now - last)
    last = now
    ticks++
  }, 10)
  const t0 = performance.now()
  await work()
  const elapsed = performance.now() - t0
  clearInterval(timer)
  return { elapsedMs: Math.round(elapsed), worstStallMs: Math.round(worst), ticks }
}

async function main() {
  const note = await buildNote(500n, ASSET, await computeOwnerKey(bigIntToBuffer(4444n)))
  const pub = { commitment: note.commitment, asset: ASSET, amount: 500n }
  const abs = (p: string) => new URL(p, location.href).href

  const onMain = await measure(() => generateShieldProof(note, pub, abs(WASM), abs(ZKEY)))

  const worker = new Worker('/worker.js')
  const inWorker = await measure(() => new Promise<void>((resolve, reject) => {
    worker.onmessage = (e: MessageEvent) => (e.data.ok ? resolve() : reject(new Error(e.data.error)))
    worker.onerror = e => reject(new Error(String(e.message)))
    worker.postMessage({ kind: 'shield', args: [note, pub, abs(WASM), abs(ZKEY)] })
  }))
  ;(window as any).__result = { onMain, inWorker }
}
main().catch(e => { (window as any).__result = { error: String(e && e.stack || e) } })
