pragma circom 2.0.0;

include "./poseidon2.circom";

// Owner key: pk = Poseidon2(nk, DOMAIN_PK). Committed into every note so that
// spending it requires knowing the matching nullifier key. Without this, `nk`
// was a free private input: any prover could pick a fresh `nk` per spend,
// producing a fresh nullifier for the same note (unbounded double spend), and
// anyone who knew a note's plaintext could spend it.
// DOMAIN_PK = int("zkella_pk") keeps pk distinct from the nullifier hash
// Poseidon2(nk, rho) except for a note whose rho equals that constant.
template OwnerKey() {
    signal input nk;
    signal output pk;

    component h = Poseidon2();
    h.in[0] <== nk;
    h.in[1] <== 2258241487740017274987;
    pk <== h.out;
}
