import { Account, Keypair, nativeToScVal, xdr } from '@stellar/stellar-sdk'
import { ZKELLAKeys } from '../../sdk/src/keys/keys'
import { ZKELLAWallet } from '../../sdk/src/wallet/wallet'
import { TESTNET_CONTRACTS } from '../../sdk/src/config/testnet'

jest.setTimeout(60000)

async function walletWith(server: unknown) {
  const keys = await ZKELLAKeys.fromSeed(new Uint8Array(32).fill(71))
  const secret = Keypair.random()
  const wallet = new ZKELLAWallet({
    keys: keys.spendingKey, network: 'testnet', sorobanRpc: 'http://localhost:1',
    indexerUrl: 'http://localhost:1', tokenAddress: TESTNET_CONTRACTS.token, stellarSecret: secret.secret(),
  })
  jest.spyOn(wallet as never, 'getServer' as never).mockReturnValue(server as never)
  return { wallet, secret }
}

function fakeServer(sendResults: Array<Error | object>) {
  const sendTransaction = jest.fn(async () => {
    const next = sendResults.shift()
    if (next instanceof Error) throw next
    return next
  })
  return {
    sendTransaction,
    getAccount: jest.fn(async (id: string) => new Account(id, '1')),
    prepareTransaction: jest.fn(async () => ({ sign: () => {} })),
    getTransaction: jest.fn(async () => ({ status: 'SUCCESS', returnValue: nativeToScVal(7, { type: 'u32' }) })),
  }
}

describe('wallet submission resilience', () => {
  test('a transient RPC failure is retried and the call then succeeds', async () => {
    const server = fakeServer([new Error('fetch failed'), { status: 'PENDING', hash: 'h1' }])
    const { wallet } = await walletWith(server)
    const value = await wallet.submitContractCall(TESTNET_CONTRACTS.token, 'shield', [] as xdr.ScVal[])
    expect(value).toBeDefined()
    expect(server.sendTransaction).toHaveBeenCalledTimes(2)
  })

  test('a contract rejection is not retried', async () => {
    const server = fakeServer([new Error('HostError: Error(Contract, #4)')])
    const { wallet } = await walletWith(server)
    await expect(wallet.submitContractCall(TESTNET_CONTRACTS.token, 'shield', [] as xdr.ScVal[])).rejects.toThrow('#4')
    expect(server.sendTransaction).toHaveBeenCalledTimes(1)
  })

  test('an evicted anchor rebuilds the call against a fresh root', async () => {
    const { wallet } = await walletWith(fakeServer([]))
    const rebuild = jest.fn(async () => ({ submit: async () => 'fresh' }))
    const submit = (wallet as never as { withAnchorRetry: (s: () => Promise<string>, r: typeof rebuild) => () => Promise<string> })
      .withAnchorRetry(async () => { throw new Error('HostError: Error(Contract, #5)') }, rebuild)
    await expect(submit()).resolves.toBe('fresh')
    expect(rebuild).toHaveBeenCalledTimes(1)
  })

  test('other failures are rethrown and do not trigger a rebuild', async () => {
    const { wallet } = await walletWith(fakeServer([]))
    const rebuild = jest.fn(async () => ({ submit: async () => 'fresh' }))
    const submit = (wallet as never as { withAnchorRetry: (s: () => Promise<string>, r: typeof rebuild) => () => Promise<string> })
      .withAnchorRetry(async () => { throw new Error('HostError: Error(Contract, #6)') }, rebuild)
    await expect(submit()).rejects.toThrow('#6')
    expect(rebuild).not.toHaveBeenCalled()
  })
})
