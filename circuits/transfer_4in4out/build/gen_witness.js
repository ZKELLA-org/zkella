const circomlibjs = require("circomlibjs");
const fs = require("fs");

(async () => {
  const poseidon = await circomlibjs.buildPoseidon();
  const F = poseidon.F;

  function P2(a, b) {
    return F.toObject(poseidon([BigInt(a), BigInt(b)]));
  }

  const D = 32;
  const EMPTY_LEAF = P2(0n, 0n);
  const emptyRoots = [EMPTY_LEAF];
  for (let i = 1; i <= D; i++) {
    emptyRoots.push(P2(emptyRoots[i - 1], emptyRoots[i - 1]));
  }

  const DOMAIN_PK = 2258241487740017274987n;
  const ownerPk = k => P2(k, DOMAIN_PK);
  function noteCommitment(value, asset_id, rho, rcm, pk) {
    return P2(P2(P2(value, asset_id), P2(rho, rcm)), pk);
  }
  function nullifierOf(nk, rho) {
    return P2(nk, rho);
  }

  const asset_id = 12345n;
  const nk = 555n;

  // Four real input notes at leaf indices 0, 1, 2, 3 of an otherwise-empty
  // depth-32 tree — same construction as transfer_2in2out/build/gen_witness.js,
  // extended to 4 real leaves instead of 2.
  const inNotes = [
    { value: 300000n, rho: 111n, rcm: 222n, rcv: 1212n },
    { value: 200000n, rho: 333n, rcm: 444n, rcv: 1313n },
    { value: 150000n, rho: 555n, rcm: 666n, rcv: 1414n },
    { value: 100000n, rho: 777n, rcm: 888n, rcv: 1515n },
  ];
  const leaves = inNotes.map(n => noteCommitment(n.value, asset_id, n.rho, n.rcm, ownerPk(nk)));

  // Level 0: pairwise combine (0,1) and (2,3).
  const node01 = P2(leaves[0], leaves[1]);
  const node23 = P2(leaves[2], leaves[3]);
  // Level 1: combine (01, 23).
  let node0123 = P2(node01, node23);

  // Levels 2..D-1: combine with the empty-subtree root at each level.
  let anchor = node0123;
  for (let lvl = 2; lvl < D; lvl++) {
    anchor = P2(anchor, emptyRoots[lvl - 1]);
  }

  // Paths (32 entries each): siblings at levels 0 and 1 are real nodes,
  // levels 2..31 are empty-subtree roots.
  const paths = [
    [leaves[1], node23, ...emptyRoots.slice(1, D - 1)], // leaf0: index 00...0
    [leaves[0], node23, ...emptyRoots.slice(1, D - 1)], // leaf1: index 10...0
    [leaves[3], node01, ...emptyRoots.slice(1, D - 1)], // leaf2: index 01 0...0
    [leaves[2], node01, ...emptyRoots.slice(1, D - 1)], // leaf3: index 11 0...0
  ];
  const pathIndices = [
    [0, 0, ...new Array(D - 2).fill(0)],
    [1, 0, ...new Array(D - 2).fill(0)],
    [0, 1, ...new Array(D - 2).fill(0)],
    [1, 1, ...new Array(D - 2).fill(0)],
  ];

  // Independent verification (mirrors the circuit's own MerkleProof logic)
  // before trusting any of this to build a witness.
  function verifyPath(leaf, path, idx, root) {
    let cur = leaf;
    for (let i = 0; i < path.length; i++) cur = idx[i] === 0 ? P2(cur, path[i]) : P2(path[i], cur);
    return cur === root;
  }
  for (let i = 0; i < 4; i++) {
    if (!verifyPath(leaves[i], paths[i], pathIndices[i], anchor)) {
      throw new Error(`path ${i} does not reconstruct anchor`);
    }
  }

  const nullifiers = inNotes.map(n => nullifierOf(nk, n.rho));
  const inValueCommits = inNotes.map(n => P2(n.value, n.rcv));

  const outNotes = [
    { value: 250000n, rho: 1001n, rcm: 1002n, rcv: 1003n },
    { value: 200000n, rho: 2001n, rcm: 2002n, rcv: 2003n },
    { value: 150000n, rho: 3001n, rcm: 3002n, rcv: 3003n },
    { value: 149000n, rho: 4001n, rcm: 4002n, rcv: 4003n },
  ];
  const outPk = outNotes.map((_, i) => ownerPk(i === 0 ? 777n : nk)); // recipient, then change
  const outCommitments = outNotes.map((n, i) => noteCommitment(n.value, asset_id, n.rho, n.rcm, outPk[i]));
  const outValueCommits = outNotes.map(n => P2(n.value, n.rcv));

  const fee = 1000n;
  const sumIn = inNotes.reduce((s, n) => s + n.value, 0n);
  const sumOut = outNotes.reduce((s, n) => s + n.value, 0n);
  if (sumIn !== sumOut + fee) throw new Error(`sum_in (${sumIn}) !== sum_out (${sumOut}) + fee (${fee})`);

  const input = {
    in_value: inNotes.map(n => String(n.value)),
    in_asset_id: inNotes.map(() => String(asset_id)),
    in_rho: inNotes.map(n => String(n.rho)),
    in_rcm: inNotes.map(n => String(n.rcm)),
    in_path: paths.map(p => p.map(String)),
    in_path_index: pathIndices.map(idx => idx.map(String)),
    in_rcv: inNotes.map(n => String(n.rcv)),
    nk: String(nk),

    out_value: outNotes.map(n => String(n.value)),
    out_asset_id: outNotes.map(() => String(asset_id)),
    out_rho: outNotes.map(n => String(n.rho)),
    out_rcm: outNotes.map(n => String(n.rcm)),
    out_rcv: outNotes.map(n => String(n.rcv)),
    out_pk: outPk.map(String),

    anchor: String(anchor),
    nullifiers: nullifiers.map(String),
    out_commitments: outCommitments.map(String),
    in_value_commits: inValueCommits.map(String),
    out_value_commits: outValueCommits.map(String),
    fee: String(fee),
    asset_id: String(asset_id),
  };

  fs.writeFileSync(__dirname + "/input.json", JSON.stringify(input, null, 2));
  console.log("anchor =", anchor.toString());
  console.log("nullifiers =", nullifiers.map(String));
  console.log("out_commitments =", outCommitments.map(String));
  console.log("Wrote input.json");
})();
