pragma circom 2.0.0;

include "../../node_modules/circomlib/circuits/bitify.circom";
include "../../node_modules/circomlib/circuits/comparators.circom";
include "../common/poseidon2.circom";
include "../common/range.circom";

// Basis-points denominator (10000 bps == 100%).
// SwapFairness proves the *executed* amount_out actually respects the
// price protection the user committed to *before* the relayer executed the
// swap. `min_amount_out` is a public input, but it used to be completely
// unconstrained relative to `amount_in`/`max_slippage_bps` — the two values
// that are actually bound into `intent_commitment` below — so a prover
// could supply any `min_amount_out` (as low as 0) at reveal time regardless
// of what slippage tolerance was originally committed to, defeating the
// entire front-running/slippage-protection guarantee this circuit exists
// to provide. Fixed by deriving `min_amount_out` in-circuit as
// `floor(amount_in * (10000 - max_slippage_bps) / 10000)` and constraining
// the public input to equal that derivation, so it can no longer be chosen
// independently of the committed values.
template SwapFairness() {
    signal input intent_nonce;
    signal input amount_in;
    signal input max_slippage_bps;

    signal input intent_commitment;
    signal input asset_in;
    signal input asset_out;
    signal input amount_out;
    signal input min_amount_out;

    component h1 = Poseidon2();
    h1.in[0] <== asset_in;
    h1.in[1] <== asset_out;

    // `packed` bit-packs (amount_in, max_slippage_bps) into one field element
    // for hashing, via `amount_in * 2^32 + max_slippage_bps`. That's only an
    // injective encoding if max_slippage_bps is actually bounded below 2^32
    // — unconstrained, it's an arbitrary field element, and for any target
    // `packed` value a prover can pick max_slippage_bps' freely and solve
    // amount_in' = (packed - max_slippage_bps') / 2^32 mod r (2^32 is
    // invertible mod the field prime), producing a *different* (amount_in,
    // max_slippage_bps) pair that packs to the identical value and therefore
    // the identical intent_commitment. Range64 already bounds amount_in
    // elsewhere in the protocol for exactly this reason; max_slippage_bps
    // needs the same treatment, sized to the packing's own shift amount
    // (32 bits) rather than reusing Range64, since it only needs to be
    // narrower than the shift to prevent overlap into amount_in's bits — a
    // real basis-points value (0-10000) fits in 32 bits with enormous room
    // to spare.
    component amount_in_range = Range64();
    amount_in_range.value <== amount_in;
    component slippage_range = Num2Bits(32);
    slippage_range.in <== max_slippage_bps;

    // max_slippage_bps must additionally be <= 10000 (100%): the derivation
    // below computes `10000 - max_slippage_bps`, which would wrap around
    // the field to a huge value (not a small negative number — field
    // subtraction has no sign) for any max_slippage_bps > 10000, corrupting
    // the min_amount_out derivation rather than merely rejecting an
    // out-of-range basis-points value.
    component slippage_bound = LessThan(32);
    slippage_bound.in[0] <== max_slippage_bps;
    slippage_bound.in[1] <== 10001;
    slippage_bound.out === 1;

    signal packed <== amount_in * (2**32) + max_slippage_bps;

    component h2 = Poseidon2();
    h2.in[0] <== packed;
    h2.in[1] <== intent_nonce;

    component h3 = Poseidon2();
    h3.in[0] <== h1.out;
    h3.in[1] <== h2.out;
    h3.out === intent_commitment;

    // min_amount_out := floor(amount_in * (10000 - max_slippage_bps) / 10000),
    // enforced via the standard circom quotient/remainder pattern: the
    // prover supplies both as witness, and the circuit checks
    // quotient*divisor + remainder == dividend with 0 <= remainder < divisor
    // (a false quotient/remainder pair can't satisfy both constraints
    // simultaneously for a given dividend/divisor).
    signal scaled <== amount_in * (10000 - max_slippage_bps);
    signal remainder <-- scaled % 10000;
    min_amount_out * 10000 + remainder === scaled;
    component remainder_range = LessThan(14); // 10000 < 2^14
    remainder_range.in[0] <== remainder;
    remainder_range.in[1] <== 10000;
    remainder_range.out === 1;

    signal diff <== amount_out - min_amount_out;
    component range = Range64();
    range.value <== diff;
}

component main {
    public [intent_commitment, asset_in, asset_out, amount_out, min_amount_out]
} = SwapFairness();
