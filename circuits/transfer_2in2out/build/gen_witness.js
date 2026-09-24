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

  // empty_subtree_root[0] = EMPTY_LEAF, empty_subtree_root[i] = P2(prev, prev)
  const emptyRoots = [EMPTY_LEAF];
  for (let i = 1; i <= D; i++) {
    emptyRoots.push(P2(emptyRoots[i - 1], emptyRoots[i - 1]));
  }

  const DOMAIN_PK = 2258241487740017274987n;
  const ownerPk = k => P2(k, DOMAIN_PK);
  function noteCommitment(value, asset_id, rho, rcm, pk) {
    return P2(P2(P2(value, asset_id), P2(rho, rcm)), pk);
  }
  function nullifier(nk, rho) {
    return P2(nk, rho);
  }
  function valueCommit(value, rcv) {
    return P2(value, rcv);
  }

  // ---- Witness values ----
  const asset_id = 12345n;
  const nk = 555n;

  const in_value = [600000n, 400000n];
  const in_rho = [111n, 333n];
  const in_rcm = [222n, 444n];
  const in_rcv = [1212n, 1313n];

  const in_cm = [
    noteCommitment(in_value[0], asset_id, in_rho[0], in_rcm[0], ownerPk(nk)),
    noteCommitment(in_value[1], asset_id, in_rho[1], in_rcm[1], ownerPk(nk)),
  ];
  const nullifiers = [nullifier(nk, in_rho[0]), nullifier(nk, in_rho[1])];
  const in_value_commits = [valueCommit(in_value[0], in_rcv[0]), valueCommit(in_value[1], in_rcv[1])];

  const fee = 0n;
  const out_value = [700000n, 300000n];
  const out_pk = [ownerPk(777n), ownerPk(nk)]; // recipient, change
  const out_rho = [666n, 999n];
  const out_rcm = [777n, 1010n];
  const out_rcv = [888n, 1111n];

  // sanity: sum_in === sum_out + fee
  const sumIn = in_value[0] + in_value[1];
  const sumOut = out_value[0] + out_value[1];
  if (sumIn !== sumOut + fee) throw new Error(`value conservation broken: ${sumIn} != ${sumOut}+${fee}`);

  const out_commitments = [
    noteCommitment(out_value[0], asset_id, out_rho[0], out_rcm[0], out_pk[0]),
    noteCommitment(out_value[1], asset_id, out_rho[1], out_rcm[1], out_pk[1]),
  ];
  const out_value_commits = [valueCommit(out_value[0], out_rcv[0]), valueCommit(out_value[1], out_rcv[1])];

  // ---- Build a tree with in_cm[0] at leaf 0, in_cm[1] at leaf 1 (fresh/empty tree otherwise) ----
  // level-0 nodes we actually know: index0=in_cm[0], index1=in_cm[1]. Everything else is the empty subtree root.
  // Compute node values level by level for the path of leaf0 and leaf1 (they're siblings at level 0,
  // then both on the "left" spine for all subsequent levels since nothing else was inserted).
  const level1Parent = P2(in_cm[0], in_cm[1]); // level 1, index 0
  let node = level1Parent;
  for (let lvl = 1; lvl < D; lvl++) {
    node = P2(node, emptyRoots[lvl]); // current node is always left (index 0) at every level >= 1
  }
  const anchor = node;

  // Path for leaf 0 (index 0): level0 sibling = in_cm[1] (index bit 1, i.e. sibling is on the right -> index[0]=0 meaning leaf0 is left)
  // Path/index convention matches merkle.rs: index bit = 0 means "current is left child, sibling stored is the right one" etc.
  // In the circom circuit: in[0] = nodes[i] + index[i]*(path[i]-nodes[i]); in[1] = path[i] + index[i]*(nodes[i]-path[i])
  //   index=0 -> in[0]=nodes[i] (current first), in[1]=path[i] (sibling second)  => current is LEFT
  //   index=1 -> in[0]=path[i] (sibling first),  in[1]=nodes[i] (current second) => current is RIGHT
  function buildPath(leafIndex) {
    const path = [];
    const index = [];
    let idx = leafIndex;
    // level 0: sibling is the other leaf
    if (idx % 2 === 0) {
      path.push(in_cm[1]);
      index.push(0);
    } else {
      path.push(in_cm[0]);
      index.push(1);
    }
    idx = Math.floor(idx / 2);
    // levels 1..D-1: current node is always the left child (index 0), sibling is the empty subtree root
    for (let lvl = 1; lvl < D; lvl++) {
      path.push(emptyRoots[lvl]);
      index.push(0);
    }
    return { path, index };
  }

  const path0 = buildPath(0);
  const path1 = buildPath(1);

  // Verify both paths reconstruct to `anchor` (independent check, mirroring the circuit's own logic)
  function verifyPath(leaf, path, index, root) {
    let cur = leaf;
    for (let i = 0; i < path.length; i++) {
      cur = index[i] === 0 ? P2(cur, path[i]) : P2(path[i], cur);
    }
    return cur === root;
  }
  if (!verifyPath(in_cm[0], path0.path, path0.index, anchor)) throw new Error("path0 does not reconstruct anchor");
  if (!verifyPath(in_cm[1], path1.path, path1.index, anchor)) throw new Error("path1 does not reconstruct anchor");

  const input = {
    in_value: in_value.map(String),
    in_asset_id: [String(asset_id), String(asset_id)],
    in_rho: in_rho.map(String),
    in_rcm: in_rcm.map(String),
    in_path: [path0.path.map(String), path1.path.map(String)],
    in_path_index: [path0.index.map(String), path1.index.map(String)],
    in_rcv: in_rcv.map(String),
    nk: String(nk),

    out_value: out_value.map(String),
    out_asset_id: [String(asset_id), String(asset_id)],
    out_rho: out_rho.map(String),
    out_rcm: out_rcm.map(String),
    out_rcv: out_rcv.map(String),
    out_pk: out_pk.map(String),

    anchor: String(anchor),
    nullifiers: nullifiers.map(String),
    out_commitments: out_commitments.map(String),
    in_value_commits: in_value_commits.map(String),
    out_value_commits: out_value_commits.map(String),
    fee: String(fee),
    asset_id: String(asset_id),
  };

  fs.writeFileSync(__dirname + "/input.json", JSON.stringify(input, null, 2));

  // Also dump the public inputs in the exact order the contract/verifier expect,
  // for cross-checking against public.json after witness generation.
  console.log("anchor            =", anchor.toString());
  console.log("nullifiers        =", nullifiers.map(String));
  console.log("out_commitments   =", out_commitments.map(String));
  console.log("in_value_commits  =", in_value_commits.map(String));
  console.log("out_value_commits =", out_value_commits.map(String));
  console.log("fee               =", fee.toString());
  console.log("asset_id          =", asset_id.toString());
  console.log("Wrote input.json");
})();
