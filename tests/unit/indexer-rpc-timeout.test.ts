import { withTimeout } from '../../indexer/src/sync'

describe('indexer RPC timeout', () => {
  test('a hung call is rejected instead of stalling the syncer', async () => {
    const hung = new Promise<never>(() => {})
    await expect(withTimeout(hung, 50, 'getEvents')).rejects.toThrow('getEvents timed out after 50 ms')
  })

  test('a call that settles in time returns its value', async () => {
    await expect(withTimeout(Promise.resolve(7), 1000, 'getEvents')).resolves.toBe(7)
  })
})
