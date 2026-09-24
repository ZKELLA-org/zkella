// Real-browser check for the proving Web Worker: bundles sdk/src/prover/worker.ts and a test page,
// serves them with the compiled circuit artifacts, drives headless Chromium and reports how long
// the main thread is blocked while a real shield proof is generated on the main thread vs in the worker.
// Requires: npm i --no-save playwright-core esbuild esbuild-plugin-polyfill-node && npx playwright-core install chromium
import { build } from 'esbuild'
import { polyfillNode } from 'esbuild-plugin-polyfill-node'
import { chromium } from 'playwright-core'
import http from 'node:http'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const out = fs.mkdtempSync('/tmp/zkella-browser-')
const common = { bundle: true, platform: 'browser', format: 'iife', target: 'es2022', define: { 'process.browser': 'true', global: 'globalThis' }, logLevel: 'error',
  plugins: [polyfillNode({ polyfills: { fs: 'empty', readline: 'empty', os: 'empty', worker_threads: 'empty', child_process: 'empty' } })] }
await build({ ...common, entryPoints: [path.join(root, 'sdk/src/prover/worker.ts')], outfile: path.join(out, 'worker.js') })
await build({ ...common, entryPoints: [path.join(root, 'scripts/browser/page_entry.ts')], outfile: path.join(out, 'page.js') })
fs.writeFileSync(path.join(out, 'index.html'), '<!doctype html><script src="/page.js"></script>')

const types = { '.js': 'text/javascript', '.html': 'text/html', '.wasm': 'application/wasm' }
const server = http.createServer((req, res) => {
  const url = decodeURIComponent(req.url.split('?')[0])
  const file = url.startsWith('/circuits/') ? path.join(root, url) : path.join(out, url === '/' ? 'index.html' : url)
  if (!file.startsWith(root) && !file.startsWith(out) || !fs.existsSync(file)) { res.writeHead(404); return res.end() }
  res.writeHead(200, { 'content-type': types[path.extname(file)] || 'application/octet-stream' })
  fs.createReadStream(file).pipe(res)
}).listen(0)
const port = server.address().port

const browser = await chromium.launch()
const page = await browser.newPage()
page.on('pageerror', e => console.error('pageerror:', e.message))
await page.goto(`http://localhost:${port}/`)
const result = await page.waitForFunction(() => window.__result, null, { timeout: 240000 }).then(h => h.jsonValue())
await browser.close(); server.close()
console.log(JSON.stringify(result, null, 2))
if (result.error) process.exit(1)
const { onMain, inWorker } = result
console.log(`main-thread proof: worst stall ${onMain.worstStallMs}ms; worker proof: worst stall ${inWorker.worstStallMs}ms`)
process.exit(inWorker.worstStallMs < 250 && inWorker.worstStallMs < onMain.worstStallMs ? 0 : 2)
