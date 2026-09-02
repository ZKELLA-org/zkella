const contracts = [
  ["cXAUt", "0x73cc9aF9d6BEFdb3c3fAf8a5E8c05Cb95FdaEEf1"],
  ["cbbqTGBP", "0xBA4cFF6ED6F7Cb2A58776dECa4E984b498446762"],
  ["ctGBP", "0xa873750ccBafD5ec7Dd13bfD5237d7129832eDD9"],
  ["cZAMA", "0x80CB147Fd86dC6dEe3Eee7e4Cee33d1397d98071"],
  ["cBRON", "0x85dE671c3bec1aDeD752c3Cea943521181C826bc"],
  ["cWETH", "0xda9396b82634Ea99243cE51258B6A5Ae512D4893"],
  ["cUSDT", "0xAe0207C757Aa2B4019Ad96edD0092ddc63EF0c50"],
  ["cUSDC", "0xe978F22157048E5DB8E5d07971376e86671672B2"],
  ["csteakcUSDC", "0x66Bf74E96900D1a19c7070D939D124f2F565C458"],
];

const API = "https://api.routescan.io/v2/network/mainnet/evm/1/etherscan/api";
const WRAP_SELECTOR = "0xbf376c7a";
const MAX_WINDOW = 10000;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function fetchJson(url, tries = 5) {
  let lastError;
  for (let attempt = 0; attempt < tries; attempt += 1) {
    try {
      const response = await fetch(url, { headers: { accept: "application/json" } });
      const text = await response.text();
      if (!response.ok) throw new Error(`HTTP ${response.status}: ${text.slice(0, 200)}`);
      return JSON.parse(text);
    } catch (error) {
      lastError = error;
      await sleep(500 * (attempt + 1));
    }
  }
  throw lastError;
}

async function latestBlockNumber() {
  const params = new URLSearchParams({ module: "proxy", action: "eth_blockNumber" });
  const json = await fetchJson(`${API}?${params}`);
  if (!json.result) throw new Error(`Could not read latest block: ${JSON.stringify(json)}`);
  return Number.parseInt(json.result, 16);
}

async function txlist(address, startBlock, endBlock) {
  const params = new URLSearchParams({
    module: "account",
    action: "txlist",
    address,
    startblock: String(startBlock),
    endblock: String(endBlock),
    page: "1",
    offset: String(MAX_WINDOW),
    sort: "asc",
  });
  const json = await fetchJson(`${API}?${params}`);
  if (json.status === "0") {
    const message = `${json.message || ""} ${json.result || ""}`.toLowerCase();
    if (message.includes("no transactions")) return [];
    throw new Error(`${address} ${startBlock}-${endBlock}: ${json.message} ${json.result}`);
  }
  if (!Array.isArray(json.result)) throw new Error(`${address} ${startBlock}-${endBlock}: unexpected result`);
  return json.result;
}

async function gather(address, startBlock, endBlock) {
  const rows = await txlist(address, startBlock, endBlock);
  if (rows.length < MAX_WINDOW || startBlock >= endBlock) return rows;
  const mid = Math.floor((startBlock + endBlock) / 2);
  return (await gather(address, startBlock, mid)).concat(await gather(address, mid + 1, endBlock));
}

function decodeAddressWord(input, wordIndex) {
  const hex = (input || "").toLowerCase().replace(/^0x/, "");
  const start = 8 + wordIndex * 64;
  const word = hex.slice(start, start + 64);
  if (word.length !== 64) return null;
  return `0x${word.slice(24)}`;
}

const latestBlock = await latestBlockNumber();
const result = [];
const aggregate = {
  totalTxs: 0,
  successfulTxs: 0,
  failedTxs: 0,
  wrapTxs: 0,
  successfulWrapTxs: 0,
  uniqueCallersAll: new Set(),
  uniqueCallersSuccessful: new Set(),
  uniqueWrapCallersAll: new Set(),
  uniqueWrapCallersSuccessful: new Set(),
  uniqueWrapRecipientsAll: new Set(),
  uniqueWrapRecipientsSuccessful: new Set(),
};

console.error(`latestBlock=${latestBlock}`);

for (const [symbol, address] of contracts) {
  console.error(`query ${symbol} ${address}`);
  const txs = await gather(address, 0, latestBlock);
  const uniqueTxs = new Map();
  for (const tx of txs) uniqueTxs.set(`${tx.hash}:${tx.transactionIndex}`, tx);

  const all = Array.from(uniqueTxs.values());
  const successful = all.filter((tx) => tx.isError === "0");
  const wrap = all.filter((tx) => (tx.input || "").toLowerCase().startsWith(WRAP_SELECTOR));
  const successfulWrap = wrap.filter((tx) => tx.isError === "0");

  const users = new Set(all.map((tx) => tx.from.toLowerCase()));
  const successfulUsers = new Set(successful.map((tx) => tx.from.toLowerCase()));
  const wrapCallers = new Set(wrap.map((tx) => tx.from.toLowerCase()));
  const successfulWrapCallers = new Set(successfulWrap.map((tx) => tx.from.toLowerCase()));
  const wrapRecipients = new Set(wrap.map((tx) => decodeAddressWord(tx.input, 0)).filter(Boolean));
  const successfulWrapRecipients = new Set(successfulWrap.map((tx) => decodeAddressWord(tx.input, 0)).filter(Boolean));

  result.push({
    symbol,
    address,
    totalTxs: all.length,
    successfulTxs: successful.length,
    failedTxs: all.length - successful.length,
    wrapTxs: wrap.length,
    successfulWrapTxs: successfulWrap.length,
    uniqueCallersAll: users.size,
    uniqueCallersSuccessful: successfulUsers.size,
    uniqueWrapCallersAll: wrapCallers.size,
    uniqueWrapCallersSuccessful: successfulWrapCallers.size,
    uniqueWrapRecipientsAll: wrapRecipients.size,
    uniqueWrapRecipientsSuccessful: successfulWrapRecipients.size,
  });

  aggregate.totalTxs += all.length;
  aggregate.successfulTxs += successful.length;
  aggregate.failedTxs += all.length - successful.length;
  aggregate.wrapTxs += wrap.length;
  aggregate.successfulWrapTxs += successfulWrap.length;
  for (const value of users) aggregate.uniqueCallersAll.add(value);
  for (const value of successfulUsers) aggregate.uniqueCallersSuccessful.add(value);
  for (const value of wrapCallers) aggregate.uniqueWrapCallersAll.add(value);
  for (const value of successfulWrapCallers) aggregate.uniqueWrapCallersSuccessful.add(value);
  for (const value of wrapRecipients) aggregate.uniqueWrapRecipientsAll.add(value);
  for (const value of successfulWrapRecipients) aggregate.uniqueWrapRecipientsSuccessful.add(value);
}

console.log(JSON.stringify({
  generatedAt: new Date().toISOString(),
  source: API,
  latestBlock,
  wrapperContracts: contracts.length,
  wrapSelector: WRAP_SELECTOR,
  aggregate: {
    totalTxs: aggregate.totalTxs,
    successfulTxs: aggregate.successfulTxs,
    failedTxs: aggregate.failedTxs,
    wrapTxs: aggregate.wrapTxs,
    successfulWrapTxs: aggregate.successfulWrapTxs,
    uniqueCallersAll: aggregate.uniqueCallersAll.size,
    uniqueCallersSuccessful: aggregate.uniqueCallersSuccessful.size,
    uniqueWrapCallersAll: aggregate.uniqueWrapCallersAll.size,
    uniqueWrapCallersSuccessful: aggregate.uniqueWrapCallersSuccessful.size,
    uniqueWrapRecipientsAll: aggregate.uniqueWrapRecipientsAll.size,
    uniqueWrapRecipientsSuccessful: aggregate.uniqueWrapRecipientsSuccessful.size,
  },
  result,
}, null, 2));
