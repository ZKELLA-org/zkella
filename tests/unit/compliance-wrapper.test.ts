import * as path from 'path'
import { Keypair } from '@stellar/stellar-sdk'
import { ZKELLAKeys } from '../../sdk/src/keys/keys'
import { ZKELLAWallet } from '../../sdk/src/wallet/wallet'
import { ZKELLACompliance } from '../../sdk/src/compliance/compliance'
import { nonMembershipAddress } from '../../sdk/src/prover/compliance'

const BUILD = path.resolve(__dirname, '../../circuits/compliance/build')
const WASM = path.join(BUILD, 'non_membership_js/non_membership.wasm')
const ZKEY = path.join(BUILD, 'non_membership.zkey')
jest.setTimeout(300000)

async function walletFor(seed: number) {
  const keys = await ZKELLAKeys.fromSeed(new Uint8Array(32).fill(seed))
  return new ZKELLAWallet({
    keys: keys.spendingKey, network: 'testnet', sorobanRpc: 'http://localhost:1',
    indexerUrl: 'http://localhost:1', tokenAddress: 'CAAAA', stellarSecret: Keypair.random().secret(),
  })
}

describe('ZKELLACompliance wrapper', () => {
  test('generates a proof that verifies against the list root it advertises', async () => {
    const wallet = await walletFor(21)
    const compliance = new ZKELLACompliance({
      wallet, complianceAddress: 'CCCC', sanctionedAddresses: [5n, 77n, 9000n],
      wasmPath: WASM, zkeyPath: ZKEY,
    })
    const proof = await compliance.generateNonSanctionedProof()
    expect(proof.proof.length).toBe(256)
    expect(Buffer.from(proof.sanctionsRoot).toString('hex')).toBe(Buffer.from(await compliance.sanctionsRoot()).toString('hex'))
    })

  test('a wallet whose address is on the list cannot produce a proof', async () => {
    const wallet = await walletFor(22)
    const address = await nonMembershipAddress(wallet.complianceSecret())
    const compliance = new ZKELLACompliance({
      wallet, complianceAddress: 'CCCC', sanctionedAddresses: [address],
      wasmPath: WASM, zkeyPath: ZKEY,
    })
    await expect(compliance.generateNonSanctionedProof()).rejects.toThrow('sanctions list')
  })
})
