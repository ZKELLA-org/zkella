import {
  quoteRespectsSlippage,
  QuoteValidationError,
  SwapQuoteClient,
  SwapQuoteResponse,
} from '../../sdk/src/relayer/quote'

describe('quoteRespectsSlippage', () => {

  test('matches the on-chain floor exactly: 1% slippage on 1_000_000', () => {
    // floor(1_000_000 * (10000 - 100) / 10000) = floor(990_000_000_000 / 10000) = 99_000_000... wait, matches direct formula below
    const amountIn = 1_000_000n
    const bps = 100 // 1%
    const minOut = (amountIn * (10000n - BigInt(bps))) / 10000n
    expect(quoteRespectsSlippage(amountIn, bps, minOut)).toBe(true)
    expect(quoteRespectsSlippage(amountIn, bps, minOut - 1n)).toBe(false)
  })

  test('0 bps slippage requires exact amountIn as the floor', () => {
    expect(quoteRespectsSlippage(1_000_000n, 0, 1_000_000n)).toBe(true)
    expect(quoteRespectsSlippage(1_000_000n, 0, 999_999n)).toBe(false)
  })

  test('10000 bps (100% slippage) accepts a floor of zero', () => {
    expect(quoteRespectsSlippage(1_000_000n, 10000, 0n)).toBe(true)
  })

  test('quote strictly above the floor is accepted', () => {
    expect(quoteRespectsSlippage(1_000_000n, 500, 999_999_999n)).toBe(true)
  })

  test('integer division truncates like the circuit (floor, not round)', () => {
    // amountIn=3, bps=1 (0.01%) -> 3 * 9999 / 10000 = 2.9997 -> floor = 2
    expect(quoteRespectsSlippage(3n, 1, 2n)).toBe(true)
    expect(quoteRespectsSlippage(3n, 1, 1n)).toBe(false)
  })

  test('rejects out-of-range maxSlippageBps', () => {
    expect(() => quoteRespectsSlippage(1000n, -1, 500n)).toThrow(QuoteValidationError)
    expect(() => quoteRespectsSlippage(1000n, 10001, 500n)).toThrow(QuoteValidationError)
    expect(() => quoteRespectsSlippage(1000n, 1.5, 500n)).toThrow(QuoteValidationError)
  })

  test('rejects non-positive amountIn', () => {
    expect(() => quoteRespectsSlippage(0n, 100, 0n)).toThrow(QuoteValidationError)
    expect(() => quoteRespectsSlippage(-1n, 100, 0n)).toThrow(QuoteValidationError)
  })

})

describe('SwapQuoteClient.requestQuote', () => {
  const RELAYER = 'GARELAYER00000000000000000000000000000000000000000000000'

  function mockFetch(response: SwapQuoteResponse, ok = true, status = 200): typeof fetch {
    return jest.fn().mockResolvedValue({
      ok,
      status,
      json: async () => response,
    }) as unknown as typeof fetch
  }

  test('returns a valid quote that respects slippage', async () => {
    const quote: SwapQuoteResponse = {
      quoteId:   'q1',
      amountOut: '990000',
      expiresAt: Date.now() + 60_000,
      relayer:   RELAYER,
    }
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: mockFetch(quote) })
    const result = await client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', maxSlippageBps: 100,
    })
    expect(result).toEqual(quote)
  })

  test('throws on non-ok HTTP response', async () => {
    const client = new SwapQuoteClient({
      relayerUrl: 'https://relayer.example',
      fetchImpl:  mockFetch({} as SwapQuoteResponse, false, 503),
    })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', maxSlippageBps: 100,
    })).rejects.toThrow(/HTTP 503/)
  })

  test('rejects a quote that violates the requested slippage floor', async () => {
    const quote: SwapQuoteResponse = {
      quoteId: 'q2', amountOut: '1', expiresAt: Date.now() + 60_000, relayer: RELAYER,
    }
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: mockFetch(quote) })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', maxSlippageBps: 100,
    })).rejects.toThrow(QuoteValidationError)
  })

  test('rejects a non-positive amountOut even if it would mathematically pass the floor check', async () => {
    const quote: SwapQuoteResponse = {
      quoteId: 'q3', amountOut: '0', expiresAt: Date.now() + 60_000, relayer: RELAYER,
    }
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: mockFetch(quote) })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', maxSlippageBps: 10000,
    })).rejects.toThrow(/non-positive/)
  })

  test('fails fast on a malformed amountIn before hitting the network', async () => {
    const fetchSpy = mockFetch({} as SwapQuoteResponse)
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: fetchSpy })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: 'not-a-number', maxSlippageBps: 100,
    })).rejects.toThrow(QuoteValidationError)
    expect(fetchSpy).not.toHaveBeenCalled()
  })

  test('fails fast on a non-positive amountIn before hitting the network', async () => {
    const fetchSpy = mockFetch({} as SwapQuoteResponse)
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: fetchSpy })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '0', maxSlippageBps: 100,
    })).rejects.toThrow(QuoteValidationError)
    expect(fetchSpy).not.toHaveBeenCalled()
  })

  test('rejects a relayer response with a non-integer amountOut', async () => {
    const quote = { quoteId: 'q5', amountOut: 'garbage', expiresAt: Date.now() + 60_000, relayer: RELAYER }
    const client = new SwapQuoteClient({
      relayerUrl: 'https://relayer.example',
      fetchImpl:  mockFetch(quote as unknown as SwapQuoteResponse),
    })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', maxSlippageBps: 100,
    })).rejects.toThrow(/non-integer amountOut/)
  })

  test('rejects an already-expired quote', async () => {
    const quote: SwapQuoteResponse = {
      quoteId: 'q4', amountOut: '990000', expiresAt: Date.now() - 1000, relayer: RELAYER,
    }
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: mockFetch(quote) })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', maxSlippageBps: 100,
    })).rejects.toThrow(/expired/)
  })

})
