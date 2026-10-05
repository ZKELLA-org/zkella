const { IndexerClient } = require('../sdk/dist')

;(async () => {
  const url = process.env.INDEXER_URL
  if (!url) throw new Error('set INDEXER_URL to an indexer base URL')
  const client = new IndexerClient(url)
  const { notes, nextLedger } = await client.getNotes(0, 5)
  console.log('notes returned:', notes.length, 'next ledger:', nextLedger)
  for (const n of notes) console.log('leaf', n.leafIndex, 'ledger', n.ledger, 'commitment', n.commitment.slice(0, 16) + '...')
  if (notes.length) {
    const path = await client.getMerklePath(notes[0].leafIndex)
    console.log('merkle path depth:', path.path.length)
  }
})().catch(e => { console.error(e.message); process.exit(1) })
