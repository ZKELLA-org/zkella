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
 * `min_amount_out`, and a private nonce alone (see
 * `sdk/src/prover/swapFairness.ts`'s doc comment); it does **not** depend on
 * any quoted `amount_out` at all. The circuit-enforced floor
 * (`amount_out >= min_amount_out`, with `min_amount_out` a direct,
 * asset_out-denominated value the wallet itself chose from a real quoted
 * rate — not derived on-chain from `amount_in` by a flat slippage
 * percentage, which would be unsound for a non-1:1 pair; see
 * `circuits/swap/swap_fairness.circom`'s doc comment) is what actually
 * protects the user, regardless of what a relayer quotes beforehand. A
 * tampered, forged, or simply dishonest quote can mislead a wallet's
 * *expectations*, but cannot make the on-chain protocol accept a worse deal
 * than the user's own committed minimum — the fairness proof at
 * `reveal_and_claim` simply cannot be constructed if the relayer
 * under-delivers below that floor, and the relayer's fronted capital sits
 * unclaimable until `reclaim_expired_swap` if that happens, a real economic
 * cost that gives relayers a reason to honor quotes even without
 * cryptographic enforcement.
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
  assetIn:      string
  assetOut:     string
  /** Decimal string of a `bigint` (stroops or the asset's native unit) — not a `number`, to avoid precision loss over JSON. */
  amountIn:     string
  /**
   * The lowest amount of `assetOut` the wallet will accept, as a decimal
   * `bigint` string — the same, asset_out-denominated value that will be
   * bound into `intent_commitment` at `commitSwap` time (see
   * `ZKELLASwap.commitSwap`'s `minAmountOut` doc comment). Chosen by the
   * caller from a real quoted rate between `assetIn`/`assetOut`; this
   * module has no way to derive it for an arbitrary pair.
   */
  minAmountOut: string
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
 * `quotedAmountOut >= minAmountOut`. `minAmountOut` is the caller's own
 * chosen value (see `SwapQuoteRequest.minAmountOut`'s doc comment), not
 * derived here — there is no formula relating it to `amountIn` any more,
 * since that derivation was asset-blind (see this file's top doc comment).
 */
export function quoteRespectsMinimum(
  minAmountOut:    bigint,
  quotedAmountOut: bigint,
): boolean {
  if (minAmountOut < 0n) {
    throw new QuoteValidationError(`minAmountOut must not be negative: ${minAmountOut}`)
  }
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
    // also validates `req.amountIn`/`req.minAmountOut` up front so a
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
    let minAmountOut: bigint
    try {
      minAmountOut = BigInt(req.minAmountOut)
    } catch {
      throw new QuoteValidationError(`minAmountOut is not a valid integer string: ${req.minAmountOut}`)
    }
    if (minAmountOut < 0n) {
      throw new QuoteValidationError(`minAmountOut must not be negative: ${req.minAmountOut}`)
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
    if (!quoteRespectsMinimum(minAmountOut, amountOut)) {
      throw new QuoteValidationError(
        `relayer quote (amountOut=${quote.amountOut}) is below the requested ` +
        `minAmountOut=${req.minAmountOut}`
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
