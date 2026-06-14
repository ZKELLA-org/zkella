pragma circom 2.0.0;

include "../common/poseidon2.circom";
include "../common/merkle.circom";
include "../common/range.circom";

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
    signal address <== addr_h.out;

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

    signal diff_lower <== address - lower_leaf;
    signal diff_upper <== upper_leaf - address;

    component rl = Range64();
    rl.value <== diff_lower;

    component ru = Range64();
    ru.value <== diff_upper;
}

component main {
    public [sanctions_root, tk_commitment]
} = NonMembership(32);
