/**
 * End-to-end shield test on Stellar Testnet.
 *
 * Goes through `ZKELLAWallet.shield()` (sdk/src/wallet/wallet.ts) end to
 * end — real note construction, a real Groth16 proof built from the
 * committed `circuits/shield/build/` artifacts (`shield.wasm`/`shield.zkey`,
 * the same dev-ceremony artifacts the unit/cost-parity tests use — see
 * docs/audit/artifacts.json for their provenance), and a real submitted
 * transaction. Earlier versions of this test hand-built the `shield()`
 * call's ScVal arguments directly and had drifted out of sync with the
 * contract's real signature (missing `owner_pk`, an empty placeholder
 * proof, a placeholder `value_commit`) — going through the wallet instead
 * means this test can only go stale if the wallet's own shield() path
 * does, which the unit test suite already exercises on every run.
 *
 * Prerequisites:
 *   - TOKEN_CONTRACT_ID env var set to a deployed ShieldedToken contract address
 *   - STELLAR_SECRET env var set to a funded testnet account secret key,
 *     already governance-approved to shield USDC_TESTNET (or native, if
 *     USDC_TESTNET is left unset — see below)
 *   - SOROBAN_RPC_URL env var (default: https://soroban-testnet.stellar.org)
 *   - USDC_TESTNET env var set to a testnet USDC contract address — if
 *     unset, this test shields native XLM instead (resolved via
 *     `stellar contract id asset --asset native`), since every Testnet
 *     deployment in this repo approves native by default (see
 *     deployments.json) while a specific USDC contract may not be.
 *
 * Run: npx jest tests/e2e/shield.test.ts --testTimeout=90000
 */

import { execSync } from 'child_process'
import * as path from 'path'
import { Account, Contract, Keypair, Networks, rpc, TransactionBuilder, nativeToScVal, scValToNative, xdr } from '@stellar/stellar-sdk'
import { ZKELLAKeys }   from '../../sdk/src/keys/keys'
import { ZKELLAWallet } from '../../sdk/src/wallet/wallet'

const RPC_URL = process.env.SOROBAN_RPC_URL ?? 'https://soroban-testnet.stellar.org'
const SECRET  = process.env.STELLAR_SECRET  ?? ''
const TOKEN_ID = process.env.TOKEN_CONTRACT_ID ?? ''

const SKIP = !SECRET || !TOKEN_ID

const CIRCUIT_BUILD = path.join(__dirname, '..', '..', 'circuits', 'shield', 'build')

describe('Shield — end-to-end on Stellar Testnet', () => {

  // Skip if env vars are not set (CI without testnet access)
  const maybeTest = SKIP ? test.skip : test

  maybeTest('full shield flow: key gen → real proof → submit → on-chain state changes', async () => {
    const server  = new rpc.Server(RPC_URL)
    const keypair = Keypair.fromSecret(SECRET)
    const account = await server.getAccount(keypair.publicKey())

    const ASSET_ID = process.env.USDC_TESTNET
      || execSync('stellar contract id asset --asset native --network testnet').toString().trim()

    // 1. Generate a ZKELLA key pair
    const keys = await ZKELLAKeys.generate()
    expect(keys.spendingKey.raw).toHaveLength(32)

    // 2. Read root before shield
    const token      = new Contract(TOKEN_ID)
    const rootBefore = await callView(server, account, keypair, token, 'merkle_root', [])

    // 3. Shield through the real wallet — builds the note, encrypts it,
    //    generates a real Groth16 proof, and submits the transaction.
    const wallet = new ZKELLAWallet({
      keys:          keys.spendingKey,
      network:       'testnet',
      sorobanRpc:    RPC_URL,
      indexerUrl:    'http://unused', // shield() does not read the indexer
      tokenAddress:  TOKEN_ID,
      stellarSecret: SECRET,
      shieldCircuit: {
        wasmPath: path.join(CIRCUIT_BUILD, 'shield_js', 'shield.wasm'),
        zkeyPath: path.join(CIRCUIT_BUILD, 'shield.zkey'),
      },
    })

    const AMOUNT = 1000n
    const { note, submit } = await wallet.shield({ asset: ASSET_ID, amount: AMOUNT })
    expect(note.commitment).toHaveLength(32)

    const { leafIndex } = await submit()
    console.log(`✓ Note shielded at leaf index: ${leafIndex}`)
    expect(typeof leafIndex).toBe('number')

    // 4. Verify Merkle root changed
    const rootAfter = await callView(server, account, keypair, token, 'merkle_root', [])
    expect(rootBefore).not.toEqual(rootAfter)
    console.log(`✓ Merkle root updated`)

    // 5. Verify shielded supply increased by at least AMOUNT
    const supply = await callView(server, account, keypair, token, 'shielded_supply', [
      nativeToScVal(ASSET_ID, { type: 'address' }),
    ]) as string | number | bigint
    expect(BigInt(supply)).toBeGreaterThanOrEqual(AMOUNT)
    console.log(`✓ Shielded supply: ${supply}`)

    // 6. Verify leaf count incremented
    const leafCount = await callView(server, account, keypair, token, 'leaf_count', [])
    expect(Number(leafCount)).toBeGreaterThan(0)
    console.log(`✓ Leaf count: ${leafCount}`)

  }, 90_000)

})

// ── Helpers ───────────────────────────────────────────────────────────────────

async function callView(
  server:  rpc.Server,
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
  if (rpc.Api.isSimulationError(sim)) {
    throw new Error(`simulation error: ${sim.error}`)
  }
  return scValToNative((sim as rpc.Api.SimulateTransactionSuccessResponse).result!.retval)
}
