pragma circom 2.0.0;

include "../../node_modules/circomlib/circuits/bitify.circom";
include "../../node_modules/circomlib/circuits/comparators.circom";
include "../common/poseidon2.circom";
include "../common/merkle.circom";

// Proves the prover's address is NOT in a sanctions list, using a sorted
// Merkle tree with adjacent-leaf bracketing.
//
// The list is a depth-D tree whose leaves are 248-bit values in strictly
// ascending order from index 0, starting with a minimum sentinel (0) and
// ending with a maximum sentinel (2^248 - 1) so every address that is not
// itself a sentinel falls between two adjacent leaves. An address is not
// listed iff there are adjacent leaves lower < address < upper. Soundness
// needs all of:
//   - both leaves are real members of the tree (Merkle proofs to the root),
//   - the bounds are STRICT (a sanctioned address equals a leaf; a non-strict
//     bound would let it use that leaf as both neighbours),
//   - the leaves are ADJACENT (upper index = lower index + 1), otherwise a
//     prover could bracket the address with two leaves that have a sanctioned
//     entry between them,
//   - the comparisons happen on values known to fit in 248 bits.
// The address is the low 248 bits of Poseidon2(sk, 1); using the strict
// decomposition avoids the aliasing of a plain 254-bit decomposition.
template NonMembership(D) {
    signal input sk;
    signal input lower_leaf;
    signal input upper_leaf;
    signal input lower_path[D];
    signal input lower_path_index[D];
    signal input upper_path[D];
    signal input upper_path_index[D];

    signal input sanctions_root;
    signal input tk_commitment;

    component sk_commit = Poseidon2();
    sk_commit.in[0] <== sk;
    sk_commit.in[1] <== 0;
    sk_commit.out   === tk_commitment;

    component addr_h = Poseidon2();
    addr_h.in[0] <== sk;
    addr_h.in[1] <== 1;

    component addr_bits = Num2Bits_strict();
    addr_bits.in <== addr_h.out;
    var acc = 0;
    var pw = 1;
    for (var i = 0; i < 248; i++) {
        acc += addr_bits.out[i] * pw;
        pw = pw * 2;
    }
    signal address <== acc;

    component lower_mp = MerkleProof(D);
    lower_mp.leaf <== lower_leaf;
    for (var i = 0; i < D; i++) {
        lower_mp.path[i]  <== lower_path[i];
        lower_mp.index[i] <== lower_path_index[i];
    }
    lower_mp.root === sanctions_root;

    component upper_mp = MerkleProof(D);
    upper_mp.leaf <== upper_leaf;
    for (var i = 0; i < D; i++) {
        upper_mp.path[i]  <== upper_path[i];
        upper_mp.index[i] <== upper_path_index[i];
    }
    upper_mp.root === sanctions_root;

    // Adjacency: the index bits are already constrained boolean by MerkleProof.
    var lower_idx = 0;
    var upper_idx = 0;
    var ipw = 1;
    for (var i = 0; i < D; i++) {
        lower_idx += lower_path_index[i] * ipw;
        upper_idx += upper_path_index[i] * ipw;
        ipw = ipw * 2;
    }
    upper_idx === lower_idx + 1;

    // Leaves must fit in 248 bits for LessThan(248) to be meaningful.
    component lower_range = Num2Bits(248);
    lower_range.in <== lower_leaf;
    component upper_range = Num2Bits(248);
    upper_range.in <== upper_leaf;

    // Strict: lower_leaf < address < upper_leaf.
    component lt_lower = LessThan(248);
    lt_lower.in[0] <== lower_leaf;
    lt_lower.in[1] <== address;
    lt_lower.out === 1;

    component lt_upper = LessThan(248);
    lt_upper.in[0] <== address;
    lt_upper.in[1] <== upper_leaf;
    lt_upper.out === 1;
}

component main {
    public [sanctions_root, tk_commitment]
} = NonMembership(32);
