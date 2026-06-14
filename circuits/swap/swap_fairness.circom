pragma circom 2.0.0;

include "../common/poseidon2.circom";
include "../common/range.circom";

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

    signal packed <== amount_in * (2**32) + max_slippage_bps;

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
