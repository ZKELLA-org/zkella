const circomlibjs = require("circomlibjs");
const fs = require("fs");

(async () => {
  const poseidon = await circomlibjs.buildPoseidon();
  const F = poseidon.F;
  function P2(a, b) {
    return F.toObject(poseidon([BigInt(a), BigInt(b)]));
  }

  // Deliberately non-1:1 pair: asset_in and asset_out are different assets
  // (field-encoded addresses 111/222, standing in for e.g. XLM/USDC), and
  // min_amount_out bears no fixed ratio to amount_in — it's the swap
  // creator's own rate-aware choice, not something the circuit derives.
  // amount_in = 1,000,000 units of asset_in; the creator quotes a real rate
  // of ~1:5 and wants at least 4,800,000 units of asset_out (a 4% slippage
  // tolerance on that quote, nothing like a 1:1 derivation would produce).
  const asset_in = 111n;
  const asset_out = 222n;
  const amount_in = 1000000n;
  const min_amount_out = 4800000n; // creator's own rate-aware minimum, asset_out units
  const intent_nonce = 999n;
  const amount_out = 4950000n; // executed price beats the floor

  const packed = amount_in * (2n ** 64n) + min_amount_out;
  const h1 = P2(asset_in, asset_out);
  const h2 = P2(packed, intent_nonce);
  const intent_commitment = P2(h1, h2);

  const input = {
    intent_nonce: String(intent_nonce),
    amount_in: String(amount_in),
    intent_commitment: String(intent_commitment),
    asset_in: String(asset_in),
    asset_out: String(asset_out),
    amount_out: String(amount_out),
    min_amount_out: String(min_amount_out),
  };

  fs.writeFileSync(__dirname + "/input.json", JSON.stringify(input, null, 2));
  console.log("cross-asset min_amount_out (creator-chosen, no fixed ratio to amount_in) =", min_amount_out.toString());
  console.log("amount_out =", amount_out.toString());
  console.log("intent_commitment =", intent_commitment.toString());
  console.log("Wrote input.json");

  // Adversarial witness: same amount_in/asset pair, but min_amount_out
  // forged to 0 without updating intent_commitment to match. This is
  // exactly the old exploit shape (claim an unrelated, unbound
  // min_amount_out at reveal time) — must fail witness generation now
  // that min_amount_out is hashed directly into intent_commitment.
  const forged = { ...input, min_amount_out: "0" };
  fs.writeFileSync(__dirname + "/input_forged.json", JSON.stringify(forged, null, 2));
  console.log("Wrote input_forged.json (min_amount_out forged to 0, commitment not updated — must fail)");
})();
