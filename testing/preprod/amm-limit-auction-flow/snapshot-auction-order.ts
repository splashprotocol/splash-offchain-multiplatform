import {
  auctionAddress,
  auctionDatumRedeemer,
  jsonStringify,
} from "./src/auction.ts";
import { readFlowEnv } from "./src/env.ts";
import { makeLucid } from "./src/lucid.ts";

const env = await readFlowEnv();
const txHash = Deno.env.get("AUCTION_TX_HASH") ??
  Deno.env.get("EXISTING_AUCTION_TX_HASH");
const outputIndex = Deno.env.get("AUCTION_OUTPUT_INDEX") ??
  Deno.env.get("EXISTING_AUCTION_OUTPUT_INDEX");
if (!txHash || !outputIndex) {
  throw new Error(
    "AUCTION_TX_HASH/AUCTION_OUTPUT_INDEX or EXISTING_AUCTION_* is required",
  );
}

const lucid = await makeLucid(env);
const [utxo] = await lucid.utxosByOutRef([{
  txHash,
  outputIndex: Number(outputIndex),
}]);
if (!utxo) {
  throw new Error(`Auction UTxO ${txHash}#${outputIndex} is not present`);
}
if (utxo.address !== auctionAddress(env)) {
  throw new Error(
    `Auction UTxO ${txHash}#${outputIndex} address does not match configured validator`,
  );
}
if (!utxo.datum) {
  throw new Error(`Auction UTxO ${txHash}#${outputIndex} has no inline datum`);
}

console.log(jsonStringify({
  txHash,
  outputIndex,
  address: utxo.address,
  datum: utxo.datum,
  redeemer: auctionDatumRedeemer(utxo.datum),
}));
