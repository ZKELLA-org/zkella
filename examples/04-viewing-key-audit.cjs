const fs = require('fs')
const { ZKELLAAuditor } = require('../sdk/dist')

;(async () => {
  const url = process.env.INDEXER_URL
  const file = process.env.VIEWING_KEY_EXPORT
  if (!url || !file) throw new Error('set INDEXER_URL and VIEWING_KEY_EXPORT (path to the exported JSON)')
  const auditor = new ZKELLAAuditor({ viewingKeyExport: JSON.parse(fs.readFileSync(file, 'utf8')), indexerUrl: url })
  await auditor.sync()
  const native = require('child_process').execSync('stellar contract id asset --asset native --network testnet').toString().trim()
  for (const r of auditor.transactionHistory(native)) console.log('receipt', r.amount.toString(), 'at ledger', r.ledger)
  console.log('receipts found:', auditor.transactionHistory(native).length, '(spends are not visible to a viewing key)')
})().then(() => process.exit(0), e => { console.error(e.message); process.exit(1) })
