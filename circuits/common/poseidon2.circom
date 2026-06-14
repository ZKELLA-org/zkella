pragma circom 2.0.0;

include "../../node_modules/circomlib/circuits/poseidon.circom";

// Wrapper around circomlib Poseidon with 2 inputs
template Poseidon2() {
    signal input in[2];
    signal output out;

    component h = Poseidon(2);
    h.inputs[0] <== in[0];
    h.inputs[1] <== in[1];
    out <== h.out;
}
