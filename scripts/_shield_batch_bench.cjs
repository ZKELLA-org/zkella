// Real, live-Testnet re-measurement of `token.shield_batch`'s instruction
// cost at a given batch size — not a local cost_estimate() figure. Builds
// `batchSize` genuinely independent, individually-valid Shield proofs via
// the SDK's own `generateShieldProof` (not synthetic test data), assembles
// them into a real `shield_batch` call, and simulates it against live
// Testnet RPC. Print the resulting `transactionDataXDR` through
// `stellar xdr decode --type SorobanTransactionData --output json` and read
// `.resources.instructions` for the real instruction count — this script
// only prints the simulation result, it does not decode it itself.
//
// Usage: node scripts/_shield_batch_bench.cjs [batchSize]
//   SHIELD_BENCH_TOKEN_ID   token contract to call (default: production testnet_final)
//   SHIELD_BENCH_IDENTITY   funded `stellar keys` identity to sign with (default: zkella-testnet-deployer)
const path = require('path')
const { execSync } = require('child_process')
const { buildNote } = require('../sdk/dist/notes/builder')
const { encryptNote } = require('../sdk/dist/notes/encrypt')
const { generateShieldProof } = require('../sdk/dist/prover/shield')
const { ZKELLAKeys } = require('../sdk/dist')
const { Keypair, Contract, TransactionBuilder, rpc: SorobanRpc, nativeToScVal, Address, xdr } = require('@stellar/stellar-sdk')

// Soroban structs encode as ScVal maps sorted by field-name symbol. `fields`
// maps each key to a scalar type hint for nativeToScVal, or 'scval' meaning
// obj[key] is already a pre-built xdr.ScVal (for a nested struct).
function buildStruct(obj, fields) {
  const entries = Object.entries(fields).sort(([a], [b]) => a.localeCompare(b)).map(([key, kind]) => {
    const scVal = kind === 'scval' ? obj[key] : nativeToScVal(obj[key], { type: kind })
    return new xdr.ScMapEntry({ key: xdr.ScVal.scvSymbol(key), val: scVal })
  })
  return xdr.ScVal.scvMap(entries)
}

const RPC_URL = 'https://soroban-testnet.stellar.org'
const PASSPHRASE = 'Test SDF Network ; September 2015'
const TOKEN_ID = process.env.SHIELD_BENCH_TOKEN_ID || 'CDOHQBBDCNY6VVFBOOYDIIVQMA2U2XOT7ZP5AZWHUQKKFN6D4Z63WODP'
const IDENTITY = process.env.SHIELD_BENCH_IDENTITY || 'zkella-testnet-deployer'
const build = path.join(__dirname, '..', 'circuits', 'shield', 'build')

async function main() {
  const batchSize = Number(process.argv[2] || 8)
  const secret = execSync(`stellar keys secret ${IDENTITY}`).toString().trim()
  const kp = Keypair.fromSecret(secret)
  const native = execSync('stellar contract id asset --asset native --network testnet').toString().trim()

  const keys = await ZKELLAKeys.fromSeed(require('crypto').randomBytes(32))

  const items = []
  for (let i = 0; i < batchSize; i++) {
    const amount = 1000n + BigInt(i)
    const note = await buildNote(amount, native, keys.spendingKey.ownerKey)
    const transmissionKey = await keys.transmissionKeyForEpoch(0)
    const encryptedBundle = await encryptNote(note, transmissionKey)
    const publicInputs = { commitment: note.commitment, asset: native, amount }
    const { proof, valueCommit } = await generateShieldProof(
      note, publicInputs,
      path.join(build, 'shield_js', 'shield.wasm'), path.join(build, 'shield.zkey'),
    )
    items.push({
      amount,
      commitment: note.commitment,
      encrypted_note: encryptedBundle,
      owner_pk: note.ownerPk,
      rcm: note.rcm,
      rho: note.rho,
      shield_proof: proof,
      shield_pub: { commitment: note.commitment, pub_asset_id: native, pub_value: amount, value_commit: valueCommit },
    })
    process.stderr.write(`built item ${i + 1}/${batchSize}\n`) // proving is slow; show progress
  }

  // Build each item as a properly-sorted Soroban struct ScVal, including the nested ShieldPublicInputs struct.
  const itemScVals = items.map(it => {
    const shieldPubScVal = buildStruct(it.shield_pub, {
      commitment: 'bytes', pub_asset_id: 'address', pub_value: 'i128', value_commit: 'bytes',
    })
    return buildStruct({ ...it, shield_pub: shieldPubScVal }, {
      amount: 'i128', commitment: 'bytes', encrypted_note: 'bytes', owner_pk: 'bytes',
      rcm: 'bytes', rho: 'bytes', shield_proof: 'bytes', shield_pub: 'scval',
    })
  })

  const rpc = new SorobanRpc.Server(RPC_URL)
  const account = await rpc.getAccount(kp.publicKey())
  const contract = new Contract(TOKEN_ID)
  const op = contract.call(
    'shield_batch',
    Address.fromString(kp.publicKey()).toScVal(),
    Address.fromString(native).toScVal(),
    xdr.ScVal.scvVec(itemScVals),
  )
  const tx = new TransactionBuilder(account, { fee: '10000000', networkPassphrase: PASSPHRASE })
    .addOperation(op)
    .setTimeout(60)
    .build()

  const sim = await rpc.simulateTransaction(tx)
  if (SorobanRpc.Api.isSimulationError(sim)) {
    console.error('SIMULATION ERROR:', sim.error)
    process.exit(1)
  }
  console.log('SIM_OK', JSON.stringify({
    minResourceFee: sim.minResourceFee,
    transactionDataXDR: sim.transactionData ? sim.transactionData.build().toXDR('base64') : null,
  }))
  process.exit(0) // the RPC client otherwise leaves an open handle and never returns control
}

main().catch(e => { console.error(e); process.exit(1) })
