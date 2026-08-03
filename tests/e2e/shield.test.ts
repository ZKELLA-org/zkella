/**
 * End-to-end shield test on Stellar Testnet.
 *
 * Prerequisites:
 *   - TOKEN_CONTRACT_ID env var set to a deployed ShieldedToken contract address
 *   - STELLAR_SECRET env var set to a funded testnet account secret key
 *   - SOROBAN_RPC_URL env var (default: https://soroban-testnet.stellar.org)
 *   - USDC_TESTNET env var set to a testnet USDC contract address
 *
 * Run: npx jest tests/e2e/shield.test.ts --testTimeout=60000
 */

import { Account, Contract, Keypair, Networks, SorobanRpc, TransactionBuilder, nativeToScVal, scValToNative, xdr } from '@stellar/stellar-sdk'
import { ZKELLAKeys }     from '../../sdk/src/keys/keys'
import { buildNote }       from '../../sdk/src/notes/builder'
import { encryptNote }     from '../../sdk/src/notes/encrypt'

const RPC_URL     = process.env.SOROBAN_RPC_URL ?? 'https://soroban-testnet.stellar.org'
const SECRET      = process.env.STELLAR_SECRET  ?? ''
const TOKEN_ID     = process.env.TOKEN_CONTRACT_ID ?? ''
const USDC_ID     = process.env.USDC_TESTNET    ?? ''

const SKIP = !SECRET || !TOKEN_ID || !USDC_ID

describe('Shield — end-to-end on Stellar Testnet', () => {

  // Skip if env vars are not set (CI without testnet access)
  const maybeTest = SKIP ? test.skip : test

  maybeTest('full shield flow: key gen → note → commitment → submit → verify', async () => {
    const server  = new SorobanRpc.Server(RPC_URL)
    const keypair = Keypair.fromSecret(SECRET)
    const account = await server.getAccount(keypair.publicKey())

    // 1. Generate a ZKELLA key pair
    const keys = await ZKELLAKeys.generate()
    expect(keys.spendingKey.raw).toHaveLength(32)

    // 2. Build a note for 10 USDC (7 decimals = 100_000_000 stroops)
    const AMOUNT = 100_000_000n
    const note   = await buildNote(AMOUNT, USDC_ID)
    expect(note.commitment).toHaveLength(32)

    // 3. Encrypt the note to the recipient (self in this test)
    const encryptedNote = await encryptNote(note, keys.spendingKey.transmissionKey)
    expect(encryptedNote).toHaveLength(176)

    // 4. Read root before shield
    const token     = new Contract(TOKEN_ID)
    const rootBefore = await callView(server, account, keypair, token, 'merkle_root', [])

    // 5. Build the shield transaction
    const shieldPubInputs = {
      commitment:   note.commitment,
      value_commit: new Uint8Array(32), // placeholder — real value_commit in M2
      pub_value:    Number(AMOUNT),
      pub_asset_id: USDC_ID,
    }

    const tx = new TransactionBuilder(account, {
      fee:        '1000000',
      networkPassphrase: Networks.TESTNET,
    })
      .addOperation(
        token.call(
          'shield',
          nativeToScVal(keypair.publicKey(), { type: 'address' }),
          nativeToScVal(USDC_ID,              { type: 'address' }),
          nativeToScVal(Number(AMOUNT),        { type: 'i128' }),
          nativeToScVal(note.rho,              { type: 'bytes' }),
          nativeToScVal(note.rcm,              { type: 'bytes' }),
          nativeToScVal(note.commitment,       { type: 'bytes' }),
          nativeToScVal(encryptedNote,         { type: 'bytes' }),
          nativeToScVal(new Uint8Array(0),     { type: 'bytes' }), // proof placeholder
          nativeToScVal(shieldPubInputs,       { type: 'map' }),
        )
      )
      .setTimeout(30)
      .build()

    // 6. Simulate and submit
    const prepared = await server.prepareTransaction(tx)
    prepared.sign(keypair)
    const response = await server.sendTransaction(prepared)
    expect(response.status).not.toBe('ERROR')

    // 7. Wait for confirmation
    let result: SorobanRpc.Api.GetTransactionResponse | null = null
    for (let i = 0; i < 20; i++) {
      await sleep(3000)
      const r = await server.getTransaction(response.hash)
      if (r.status !== 'NOT_FOUND') { result = r; break }
    }
    expect(result).not.toBeNull()
    expect(result!.status).toBe('SUCCESS')
    if (result!.status !== 'SUCCESS') throw new Error('unreachable: checked above')

    // 8. Extract returned leaf index
    const leafIndex = scValToNative(result!.returnValue!)
    console.log(`✓ Note shielded at leaf index: ${leafIndex}`)
    expect(typeof leafIndex).toBe('number')

    // 9. Verify Merkle root changed
    const rootAfter = await callView(server, account, keypair, token, 'merkle_root', [])
    expect(rootBefore).not.toEqual(rootAfter)
    console.log(`✓ Merkle root updated`)

    // 10. Verify shielded supply increased
    const supply = await callView(server, account, keypair, token, 'shielded_supply', [
      nativeToScVal(USDC_ID, { type: 'address' }),
    ]) as string | number | bigint
    expect(BigInt(supply)).toBeGreaterThanOrEqual(AMOUNT)
    console.log(`✓ Shielded supply: ${supply}`)

    // 11. Verify leaf count incremented
    const leafCount = await callView(server, account, keypair, token, 'leaf_count', [])
    expect(Number(leafCount)).toBeGreaterThan(0)
    console.log(`✓ Leaf count: ${leafCount}`)

  }, 90_000)

})

// ── Helpers ───────────────────────────────────────────────────────────────────

async function callView(
  server:  SorobanRpc.Server,
  account: Account,
  keypair: Keypair,
  contract: Contract,
  method:  string,
  args:    xdr.ScVal[],
): Promise<unknown> {
  const tx = new TransactionBuilder(account, {
    fee:               '100',
    networkPassphrase: Networks.TESTNET,
  })
    .addOperation(contract.call(method, ...args))
    .setTimeout(10)
    .build()

  const sim = await server.simulateTransaction(tx)
  if (SorobanRpc.Api.isSimulationError(sim)) {
    throw new Error(`simulation error: ${sim.error}`)
  }
  return scValToNative((sim as SorobanRpc.Api.SimulateTransactionSuccessResponse).result!.retval)
}

function sleep(ms: number) {
  return new Promise(resolve => setTimeout(resolve, ms))
}
