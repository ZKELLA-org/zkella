pragma circom 2.0.0;

include "../common/commitment.circom";
include "../common/nullifier.circom";
include "../common/merkle.circom";
include "../common/range.circom";
include "../common/value_commit.circom";

template Transfer4x4(D) {
    var N_IN  = 4;
    var N_OUT = 4;

    signal input in_value[N_IN];
    signal input in_asset_id[N_IN];
    signal input in_rho[N_IN];
    signal input in_rcm[N_IN];
    signal input in_path[N_IN][D];
    signal input in_path_index[N_IN][D];
    signal input in_rcv[N_IN];
    signal input nk;

    signal input out_value[N_OUT];
    signal input out_asset_id[N_OUT];
    signal input out_rho[N_OUT];
    signal input out_rcm[N_OUT];
    signal input out_rcv[N_OUT];

    signal input anchor;
    signal input nullifiers[N_IN];
    signal input out_commitments[N_OUT];
    signal input in_value_commits[N_IN];
    signal input out_value_commits[N_OUT];
    signal input fee;
    signal input asset_id;

    component in_cm[N_IN];
    component in_mp[N_IN];
    component in_nf[N_IN];
    component in_cv[N_IN];
    component in_range[N_IN];

    for (var i = 0; i < N_IN; i++) {
        in_cm[i] = NoteCommitment();
        in_cm[i].value    <== in_value[i];
        in_cm[i].asset_id <== in_asset_id[i];
        in_cm[i].rho      <== in_rho[i];
        in_cm[i].rcm      <== in_rcm[i];

        in_mp[i] = MerkleProof(D);
        in_mp[i].leaf <== in_cm[i].cm;
        for (var j = 0; j < D; j++) {
            in_mp[i].path[j]  <== in_path[i][j];
            in_mp[i].index[j] <== in_path_index[i][j];
        }
        in_mp[i].root === anchor;

        in_nf[i] = Nullifier();
        in_nf[i].nk  <== nk;
        in_nf[i].rho <== in_rho[i];
        in_nf[i].nf  === nullifiers[i];

        in_cv[i] = ValueCommit();
        in_cv[i].value <== in_value[i];
        in_cv[i].rcv   <== in_rcv[i];
        in_cv[i].cv    === in_value_commits[i];

        in_asset_id[i] === asset_id;

        in_range[i] = Range64();
        in_range[i].value <== in_value[i];
    }

    component out_cm[N_OUT];
    component out_cv[N_OUT];
    component out_range[N_OUT];

    for (var i = 0; i < N_OUT; i++) {
        out_cm[i] = NoteCommitment();
        out_cm[i].value    <== out_value[i];
        out_cm[i].asset_id <== out_asset_id[i];
        out_cm[i].rho      <== out_rho[i];
        out_cm[i].rcm      <== out_rcm[i];
        out_cm[i].cm       === out_commitments[i];

        out_cv[i] = ValueCommit();
        out_cv[i].value <== out_value[i];
        out_cv[i].rcv   <== out_rcv[i];
        out_cv[i].cv    === out_value_commits[i];

        out_asset_id[i] === asset_id;

        out_range[i] = Range64();
        out_range[i].value <== out_value[i];
    }

    signal sum_in  <== in_value[0]  + in_value[1]  + in_value[2]  + in_value[3];
    signal sum_out <== out_value[0] + out_value[1] + out_value[2] + out_value[3];
    sum_in === sum_out + fee;
}

component main {
    public [anchor, nullifiers, out_commitments,
            in_value_commits, out_value_commits, fee, asset_id]
} = Transfer4x4(32);
