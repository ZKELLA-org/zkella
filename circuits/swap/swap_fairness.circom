pragma circom 2.0.0;

include "../common/poseidon2.circom";
include "../common/range.circom";

// SwapFairness proves the *executed* amount_out actually respects the
// price protection the user committed to *before* the relayer executed the
// swap. `min_amount_out` used to be derived in-circuit from `amount_in` and
// a flat slippage percentage (`floor(amount_in * (10000 - bps) / 10000)`) —
// but that derivation is asset-blind: `amount_in` is denominated in
// asset_in units and `min_amount_out` is compared against `amount_out`,
// denominated in asset_out units, with no exchange rate anywhere in the
// computation. For a 1:1 pair that coincidence made the bound look sound;
// for any real pair (e.g. XLM/USDC) it either rejects every honest
// execution or lets a relayer keep nearly all the value. Fixed by making
// `min_amount_out` itself a direct witness the swap creator supplies (from
// a real quoted rate between asset_in/asset_out), bound into
// `intent_commitment` in place of the old amount_in/slippage pair, with no
// in-circuit derivation at all — the creator alone chooses it, exactly as
// they already alone chose max_slippage_bps before, so the same "a wrong
// value only ever harms its own author" property from
// `contracts/swap::commit_swap`'s doc comment still holds.
template SwapFairness() {
    signal input intent_nonce;
    signal input amount_in;

    signal input intent_commitment;
    signal input asset_in;
    signal input asset_out;
    signal input amount_out;
    signal input min_amount_out;

    component h1 = Poseidon2();
    h1.in[0] <== asset_in;
    h1.in[1] <== asset_out;

    // `packed` bit-packs (amount_in, min_amount_out) into one field element
    // for hashing, via `amount_in * 2^64 + min_amount_out`. That's only an
    // injective encoding if both halves are actually bounded below 2^64 —
    // unconstrained, a prover could pick a different (amount_in,
    // min_amount_out) pair that packs to the identical value and therefore
    // the identical intent_commitment. Range64 bounds both to 64 bits,
    // comfortably narrower than the field (~254 bits), so the two halves
    // can never overlap.
    component amount_in_range = Range64();
    amount_in_range.value <== amount_in;
    component min_out_range = Range64();
    min_out_range.value <== min_amount_out;

    signal packed <== amount_in * (2**64) + min_amount_out;

    component h2 = Poseidon2();
    h2.in[0] <== packed;
    h2.in[1] <== intent_nonce;

    component h3 = Poseidon2();
    h3.in[0] <== h1.out;
    h3.in[1] <== h2.out;
    h3.out === intent_commitment;

    signal diff <== amount_out - min_amount_out;
    component range = Range64();
    range.value <== diff;
}

component main {
    public [intent_commitment, asset_in, asset_out, amount_out, min_amount_out]
} = SwapFairness();
