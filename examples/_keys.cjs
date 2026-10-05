const crypto = require('crypto')
const fs = require('fs')
const { ZKELLAKeys } = require('../sdk/dist')

// A wallet is only useful if its seed is kept: a note is spendable only with the
// spending key that derives from it. Load the seed from the environment, or from a
// file that is created with mode 0600 on first use.
async function loadKeys() {
  if (process.env.SPENDING_SEED) return ZKELLAKeys.fromSeed(Buffer.from(process.env.SPENDING_SEED, 'hex'))
  const file = process.env.SPENDING_SEED_FILE
  if (!file) throw new Error('set SPENDING_SEED (64 hex characters) or SPENDING_SEED_FILE (created if missing)')
  if (fs.existsSync(file)) return ZKELLAKeys.fromSeed(Buffer.from(fs.readFileSync(file, 'utf8').trim(), 'hex'))
  const seed = crypto.randomBytes(32)
  fs.writeFileSync(file, seed.toString('hex'), { mode: 0o600 })
  console.log('new wallet seed saved to', file, '(keep this file; losing it makes the notes unspendable)')
  return ZKELLAKeys.fromSeed(seed)
}

module.exports = { loadKeys }
