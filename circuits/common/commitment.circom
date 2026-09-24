pragma circom 2.0.0;

include "./poseidon2.circom";

// Note commitment: cm = H(H(H(value, asset_id), H(rho, rcm)), pk)
// `pk` is the owner key (see owner.circom); it binds the note to its owner.
template NoteCommitment() {
    signal input value;
    signal input asset_id;
    signal input rho;
    signal input rcm;
    signal input pk;
    signal output cm;

    component h1 = Poseidon2();
    h1.in[0] <== value;
    h1.in[1] <== asset_id;

    component h2 = Poseidon2();
    h2.in[0] <== rho;
    h2.in[1] <== rcm;

    component h3 = Poseidon2();
    h3.in[0] <== h1.out;
    h3.in[1] <== h2.out;

    component h4 = Poseidon2();
    h4.in[0] <== h3.out;
    h4.in[1] <== pk;

    cm <== h4.out;
}
