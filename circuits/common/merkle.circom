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
        // index[i] MUST be boolean. Without this, index[i] is an
        // unconstrained field element, and the "conditional swap" below
        // becomes a full linear interpolation over the entire field: for
        // essentially any target pair (P, Q) a prover wants hashers[i].in[0]
        // and in[1] to equal, the 2x2 system in (index[i], path[i]) — both
        // otherwise free witness values — is solvable, letting a prover force
        // the final `root` to equal any value for any starting `leaf`. That
        // is a complete Merkle-membership forgery: a fabricated note (with
        // attacker-chosen value, up to the Range64 bound) could be "proven"
        // to already exist in the real, current on-chain tree, bypassing the
        // core guarantee that only real, previously-shielded notes are
        // spendable.
        index[i] * (index[i] - 1) === 0;

        hashers[i] = Poseidon2();
        // Written as a single product term (index[i] * (a - b)) plus a linear
        // term, not "(1-index[i])*a + index[i]*b" (two separate quadratic
        // products summed) — circom's `<==` requires the whole RHS to reduce
        // to one linear-combination-times-linear-combination; it does not do
        // the symbolic factoring itself, so the unfactored form fails to
        // compile ("Non quadratic constraints are not allowed") even though
        // it's mathematically equivalent and both are quadratic overall.
        hashers[i].in[0] <== nodes[i] + index[i] * (path[i] - nodes[i]);
        hashers[i].in[1] <== path[i] + index[i] * (nodes[i] - path[i]);
        nodes[i+1] <== hashers[i].out;
    }

    root <== nodes[D];
}
