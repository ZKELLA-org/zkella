// `snarkjs.groth16.verify()` never accepts a `singleThread` option (see
// node_modules/snarkjs/src/groth16_verify.js), so every verify() call in the
// suite unconditionally builds — or reuses — a single, process-wide
// multi-threaded curve_bn128 singleton (node_modules/ffjavascript/src/bn128.js:
// `globalThis.curve_bn128`), backed by a real worker-thread pool. Nothing
// else in the dependency chain ever releases it, which is exactly what was
// leaving Jest workers unable to exit cleanly ("A worker process has failed
// to exit gracefully..."), independent of the `singleThread: true` already
// passed to every real proof-generation call in this suite — that fixes
// proving, not verification.
//
// `globalTeardown` runs in Jest's own orchestrator process, not inside the
// worker process where this singleton actually lives, so it can't see it.
// `setupFilesAfterEnv` registers this `afterAll` once per test file, in the
// worker's own process/module registry — wasteful if a later file in the
// same worker needs to rebuild the curve, but correct: whichever file's
// tests run last in a given worker leaves curve_bn128 cleared before that
// worker process exits.
afterAll(async () => {
  if (globalThis.curve_bn128) {
    await globalThis.curve_bn128.terminate()
  }
})
