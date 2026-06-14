pragma circom 2.0.0;

include "../../node_modules/circomlib/circuits/bitify.circom";

// Proves value in [0, 2^64)
template Range64() {
    signal input value;

    component bits = Num2Bits(64);
    bits.in <== value;
}
