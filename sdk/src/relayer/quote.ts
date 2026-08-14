/**
 * Off-chain request-for-quote (RFQ) protocol between a wallet and a
 * `contracts/swap` relayer, closing the gap `docs/TECHNICAL_SPEC.md` §12.4
 * (Known Limitation 1) documents: "the shielded swap relayer learns swap
 * parameters off-chain" — a real limitation, not a hypothetical one, since
 * without *some* channel a relayer has no way to know what `amount_out` to
 * offer before `execute_swap`, and a wallet has no way to know whether a
 * swap is even worth committing to before locking a note into escrow.
 *
 * Security model — read this before wiring this into anything:
 *
 * This protocol carries **no trust requirement of its own**. Nothing about
 * `contracts/swap`'s on-chain soundness depends on a quote being honest,
 * authentic, or even delivered — `intent_commitment` (what actually gets
 * committed on-chain via `commit_swap`) is derived from `amount_in`,
 * `max_slippage_bps`, and a private nonce alone (see
 * `sdk/src/prover/swapFairness.ts`'s doc comment); it does **not** depend on
 * any quoted `amount_out` at all. The circuit-enforced floor
 * (`min_amount_out = floor(amount_in * (10000 - max_slippage_bps) / 10000)`)
 * is what actually protects the user, regardless of what a relayer quotes
 * beforehand. A tampered, forged, or simply dishonest quote can mislead a
 * wallet's *expectations*, but cannot make the on-chain protocol accept a
 * worse deal than the user's own committed slippage tolerance — the
 * fairness proof at `reveal_and_claim` simply cannot be constructed if the
 * relayer under-delivers below that floor, and the relayer's fronted
 * capital sits unclaimable until `reclaim_expired_swap` if that happens, a
 * real economic cost that gives relayers a reason to honor quotes even
 * without cryptographic enforcement.
 *
 * What this protocol does NOT provide (know this before relying on it):
 *
 * - **No execution reservation.** `commit_swap` takes no relayer parameter —
 *   any approved relayer can call `execute_swap` for a given `swap_id` on a
 *   first-come basis. A quote from relayer A does not stop relayer B from
 *   executing first with a different (still floor-respecting, but possibly
 *   worse) `amount_out`. Reserving execution rights for a specific quoting
 *   relayer would require a `contracts/swap` change (e.g. an optional
 *   preferred-relayer field on `SwapState`) — out of scope for this
 *   off-chain-only module; tracked as a further exploration item in
 *   `docs/DESIGN_EXPLORATION.md` §2.2.
 * - **No delivery guarantee.** A relayer can simply not respond, or promise
 *   a quote and never call `execute_swap` at all. The wallet's only
 *   recourse is `cancel_swap` once `expiry_ledger` passes.
 */

export interface SwapQuoteRequest {
  assetIn:        string
  assetOut:       string
  /** Decimal string of a `bigint` (stroops or the asset's native unit) — not a `number`, to avoid precision loss over JSON. */
  amountIn:       string
  maxSlippageBps: number
}

export interface SwapQuoteResponse {
  /** Relayer-assigned identifier for this quote, for the relayer's own bookkeeping — carries no on-chain meaning. */
  quoteId:   string
  /** Decimal string of a `bigint`. */
  amountOut: string
  /** Unix milliseconds. Purely advisory — nothing on-chain enforces this. */
  expiresAt: number
  /** Stellar address of the relayer offering this quote. Must be an address separately approved via `swap.set_relayer` to have any chance of actually executing — this module doesn't check approval status, only the contract does. */
  relayer:   string
}

export class QuoteValidationError extends Error {}

/**
 * True iff `quotedAmountOut` satisfies the same floor
 * `circuits/swap/swap_fairness.circom` enforces on-chain:
 * `quotedAmountOut >= floor(amountIn * (10000 - maxSlippageBps) / 10000)`.
 * Integer (`bigint`) division here matches the circuit's field-arithmetic
 * quotient/remainder construction bit-for-bit for non-negative operands —
 * this is not an approximation of the on-chain check, it's the same formula.
 */
export function quoteRespectsSlippage(
  amountIn:        bigint,
  maxSlippageBps:  number,
  quotedAmountOut: bigint,
): boolean {
  if (!Number.isInteger(maxSlippageBps) || maxSlippageBps < 0 || maxSlippageBps > 10000) {
    throw new QuoteValidationError(`maxSlippageBps out of range [0, 10000]: ${maxSlippageBps}`)
  }
  if (amountIn <= 0n) {
    throw new QuoteValidationError(`amountIn must be positive: ${amountIn}`)
  }
  const minAmountOut = (amountIn * (10000n - BigInt(maxSlippageBps))) / 10000n
  return quotedAmountOut >= minAmountOut
}

export interface SwapQuoteClientConfig {
  relayerUrl: string
  /** Injectable for tests; defaults to the global `fetch`. */
  fetchImpl?: typeof fetch
}

export class SwapQuoteClient {
  constructor(private readonly config: SwapQuoteClientConfig) {}

  /**
   * Requests a price quote from one relayer, before committing anything
   * on-chain. Validates the response against the same slippage floor the
   * chain will enforce — a caller that only ever acts on quotes returned by
   * this method cannot be talked into proceeding with a swap that would
   * fail its own `reveal_and_claim` fairness proof later.
   *
   * A wallet wanting the best price should call this against multiple
   * configured relayer URLs and pick the highest valid `amountOut` — this
   * method deliberately only talks to one relayer per call, since relayer
   * selection/ranking policy belongs to the caller, not this module.
   */
  async requestQuote(req: SwapQuoteRequest): Promise<SwapQuoteResponse> {
    // Fail fast on a malformed request before ever hitting the network —
    // also validates `req.amountIn`/`req.maxSlippageBps` up front so a
    // parse failure below is attributable to the relayer's response, not
    // ambiguous between the caller's own input and the relayer's.
    let amountIn: bigint
    try {
      amountIn = BigInt(req.amountIn)
    } catch {
      throw new QuoteValidationError(`amountIn is not a valid integer string: ${req.amountIn}`)
    }
    if (amountIn <= 0n) {
      throw new QuoteValidationError(`amountIn must be positive: ${req.amountIn}`)
    }

    const fetchFn = this.config.fetchImpl ?? fetch
    const res = await fetchFn(`${this.config.relayerUrl}/quote`, {
      method:  'POST',
      headers: { 'Content-Type': 'application/json' },
      body:    JSON.stringify(req),
    })
    if (!res.ok) {
      throw new Error(`relayer quote request failed: HTTP ${res.status}`)
    }
    const quote = (await res.json()) as SwapQuoteResponse

    let amountOut: bigint
    try {
      amountOut = BigInt(quote.amountOut)
    } catch {
      throw new QuoteValidationError(`relayer returned a non-integer amountOut: ${quote.amountOut}`)
    }
    if (amountOut <= 0n) {
      throw new QuoteValidationError(`relayer returned a non-positive amountOut: ${quote.amountOut}`)
    }
    if (!quoteRespectsSlippage(amountIn, req.maxSlippageBps, amountOut)) {
      throw new QuoteValidationError(
        `relayer quote (amountOut=${quote.amountOut}) does not respect the requested ` +
        `${req.maxSlippageBps} bps slippage tolerance on amountIn=${req.amountIn}`
      )
    }
    if (quote.expiresAt <= Date.now()) {
      throw new QuoteValidationError(`relayer returned an already-expired quote (expiresAt=${quote.expiresAt})`)
    }
    return quote
  }
}

/**
 * Relayer-side handler signature. Not a server — a relayer wires its own
 * pricing/inventory logic into this shape and serves it however it likes
 * (unlike `indexer/`, which ships a full reference service, there is no one
 * "reference" relayer implementation to provide here: inventory, spread,
 * and risk limits are inherently operator-specific business logic). Return
 * `null` to decline quoting (e.g. unsupported pair, insufficient inventory)
 * rather than a fabricated quote.
 */
export type RelayerQuoteHandler = (
  req: SwapQuoteRequest
) => Promise<SwapQuoteResponse | null>
