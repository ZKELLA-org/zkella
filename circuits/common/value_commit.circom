pragma circom 2.0.0;

include "./poseidon2.circom";

// Value commitment binding (in-circuit binding component)
// Full Pedersen G1 arithmetic handled on-chain via BN254 host functions
template ValueCommit() {
    signal input value;
    signal input rcv;
    signal output cv;

    component h = Poseidon2();
    h.in[0] <== value;
    h.in[1] <== rcv;
    cv <== h.out;
}
