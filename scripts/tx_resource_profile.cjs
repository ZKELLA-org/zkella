// Prints the Soroban resource profile (instructions, ledger I/O, footprint size) each listed
// Testnet transaction was submitted with. Usage: node scripts/tx_resource_profile.cjs <label=hash> ...
const { rpc, xdr } = require('@stellar/stellar-sdk')
;(async () => {
  const server = new rpc.Server('https://soroban-testnet.stellar.org')
  console.log('label | instructions | read entries | write entries | read bytes | write bytes | ledger')
  for (const arg of process.argv.slice(2)) {
    const [label, hash] = arg.split('=')
    const tx = await server.getTransaction(hash)
    const sd = tx.envelopeXdr.v1.tx.ext.value
    const r = sd.resources, fp = r.footprint
    const num = v => (typeof v === 'bigint' ? Number(v) : v)
    console.log([label, num(r.instructions), fp.readOnly.length + fp.readWrite.length, fp.readWrite.length,
      num(r.diskReadBytes), num(r.writeBytes), tx.ledger].join(' | '))
  }
})().catch(e => { console.error(e.message); process.exit(1) })
