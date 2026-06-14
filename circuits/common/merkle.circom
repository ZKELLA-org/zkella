pragma circom 2.0.0;

include "./poseidon2.circom";

// Binary incremental Merkle tree path verifier
// D = tree depth (32 for production)
template MerkleProof(D) {
    signal input leaf;
    signal input path[D];
    signal input index[D];
    signal output root;

    component hashers[D];
    signal nodes[D+1];
    nodes[0] <== leaf;

    for (var i = 0; i < D; i++) {
        hashers[i] = Poseidon2();
        hashers[i].in[0] <== (1 - index[i]) * nodes[i] + index[i] * path[i];
        hashers[i].in[1] <== (1 - index[i]) * path[i] + index[i] * nodes[i];
        nodes[i+1] <== hashers[i].out;
    }

    root <== nodes[D];
}
