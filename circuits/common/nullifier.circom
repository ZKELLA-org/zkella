pragma circom 2.0.0;

include "./poseidon2.circom";

// Nullifier: nf = H(nk, rho)
template Nullifier() {
    signal input nk;
    signal input rho;
    signal output nf;

    component h = Poseidon2();
    h.in[0] <== nk;
    h.in[1] <== rho;
    nf <== h.out;
}
