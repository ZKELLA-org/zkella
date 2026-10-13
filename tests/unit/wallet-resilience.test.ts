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

function fakeServer(script: Array<'fail-before-send' | 'fail-after-landing' | 'ok' | 'contract-error'>) {
  let landed = false
  const sendTransaction = jest.fn(async () => {
    const step = script.shift()
    if (step === 'fail-before-send') throw new Error('fetch failed')
    if (step === 'fail-after-landing') { landed = true; throw new Error('fetch failed') }
    if (step === 'contract-error') throw new Error('HostError: Error(Contract, #4)')
    landed = true
    return { status: 'PENDING', hash: 'h' }
  })
  return {
    sendTransaction,
    getAccount: jest.fn(async (id: string) => new Account(id, '1')),
    prepareTransaction: jest.fn(async () => ({ sign: () => {}, hash: () => new Uint8Array(32).fill(7) })),
    getTransaction: jest.fn(async () =>
      landed ? { status: 'SUCCESS', returnValue: nativeToScVal(7, { type: 'u32' }) } : { status: 'NOT_FOUND' }),
  }
}

describe('wallet submission resilience', () => {
  test('a transient RPC failure that never reached the network is retried and then succeeds', async () => {
    const server = fakeServer(['fail-before-send', 'ok'])
    const { wallet } = await walletWith(server)
    const value = await wallet.submitContractCall(TESTNET_CONTRACTS.token, 'shield', [] as xdr.ScVal[])
    expect(value).toBeDefined()
    expect(server.sendTransaction).toHaveBeenCalledTimes(2)
  })

  test('an ambiguous failure whose transaction did land is not resubmitted', async () => {
    const server = fakeServer(['fail-after-landing'])
    const { wallet } = await walletWith(server)
    const value = await wallet.submitContractCall(TESTNET_CONTRACTS.token, 'shield', [] as xdr.ScVal[])
    expect(value).toBeDefined()
    expect(server.sendTransaction).toHaveBeenCalledTimes(1)
  })

  test('a contract rejection is not retried', async () => {
    const server = fakeServer(['contract-error'])
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
