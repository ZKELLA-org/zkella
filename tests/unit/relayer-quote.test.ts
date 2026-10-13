import {
  quoteRespectsMinimum,
  QuoteValidationError,
  SwapQuoteClient,
  SwapQuoteResponse,
} from '../../sdk/src/relayer/quote'

describe('quoteRespectsMinimum', () => {

  test('accepts a quote at or above minAmountOut, rejects below', () => {
    expect(quoteRespectsMinimum(990_000n, 990_000n)).toBe(true)
    expect(quoteRespectsMinimum(990_000n, 990_001n)).toBe(true)
    expect(quoteRespectsMinimum(990_000n, 989_999n)).toBe(false)
  })

  test('minAmountOut of zero accepts any non-negative quote', () => {
    expect(quoteRespectsMinimum(0n, 0n)).toBe(true)
    expect(quoteRespectsMinimum(0n, 1_000_000n)).toBe(true)
  })

  test('rejects a negative minAmountOut', () => {
    expect(() => quoteRespectsMinimum(-1n, 500n)).toThrow(QuoteValidationError)
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

  test('returns a valid quote that meets minAmountOut', async () => {
    const quote: SwapQuoteResponse = {
      quoteId:   'q1',
      amountOut: '990000',
      expiresAt: Date.now() + 60_000,
      relayer:   RELAYER,
    }
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: mockFetch(quote) })
    const result = await client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', minAmountOut: '990000',
    })
    expect(result).toEqual(quote)
  })

  test('throws on non-ok HTTP response', async () => {
    const client = new SwapQuoteClient({
      relayerUrl: 'https://relayer.example',
      fetchImpl:  mockFetch({} as SwapQuoteResponse, false, 503),
    })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', minAmountOut: '990000',
    })).rejects.toThrow(/HTTP 503/)
  })

  test('rejects a quote that violates the requested minimum', async () => {
    const quote: SwapQuoteResponse = {
      quoteId: 'q2', amountOut: '1', expiresAt: Date.now() + 60_000, relayer: RELAYER,
    }
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: mockFetch(quote) })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', minAmountOut: '990000',
    })).rejects.toThrow(QuoteValidationError)
  })

  test('rejects a non-positive amountOut even if it would mathematically pass the floor check', async () => {
    const quote: SwapQuoteResponse = {
      quoteId: 'q3', amountOut: '0', expiresAt: Date.now() + 60_000, relayer: RELAYER,
    }
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: mockFetch(quote) })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', minAmountOut: '0',
    })).rejects.toThrow(/non-positive/)
  })

  test('fails fast on a malformed amountIn before hitting the network', async () => {
    const fetchSpy = mockFetch({} as SwapQuoteResponse)
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: fetchSpy })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: 'not-a-number', minAmountOut: '990000',
    })).rejects.toThrow(QuoteValidationError)
    expect(fetchSpy).not.toHaveBeenCalled()
  })

  test('fails fast on a non-positive amountIn before hitting the network', async () => {
    const fetchSpy = mockFetch({} as SwapQuoteResponse)
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: fetchSpy })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '0', minAmountOut: '990000',
    })).rejects.toThrow(QuoteValidationError)
    expect(fetchSpy).not.toHaveBeenCalled()
  })

  test('fails fast on a malformed minAmountOut before hitting the network', async () => {
    const fetchSpy = mockFetch({} as SwapQuoteResponse)
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: fetchSpy })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', minAmountOut: 'not-a-number',
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
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', minAmountOut: '990000',
    })).rejects.toThrow(/non-integer amountOut/)
  })

  test('rejects an already-expired quote', async () => {
    const quote: SwapQuoteResponse = {
      quoteId: 'q4', amountOut: '990000', expiresAt: Date.now() - 1000, relayer: RELAYER,
    }
    const client = new SwapQuoteClient({ relayerUrl: 'https://relayer.example', fetchImpl: mockFetch(quote) })
    await expect(client.requestQuote({
      assetIn: 'CAAA', assetOut: 'CBBB', amountIn: '1000000', minAmountOut: '990000',
    })).rejects.toThrow(/expired/)
  })

})
