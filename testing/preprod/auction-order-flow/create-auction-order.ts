import { readFlowEnv } from "./src/env.ts";
import { makeLucid } from "./src/lucid.ts";
import { fetchSubmittedOutputIndex, submitSignedTx } from "./src/submit.ts";
import {
  auctionAddress,
  buildAuctionDatum,
  jsonStringify,
  unitOf,
} from "./src/auction.ts";

const env = await readFlowEnv();
const lucid = await makeLucid(env);
const seed = await Deno.readTextFile(env.walletSeedFile);
lucid.selectWallet.fromSeed(seed.trim());

const walletAddress = await lucid.wallet().address();
const address = auctionAddress(env);
const datum = buildAuctionDatum(env, walletAddress);
const value = {
  lovelace: env.auctionLovelaceBudget,
  [unitOf(env.auctionBase)]: env.auctionBaseAmount,
};

const tx = await lucid
  .newTx()
  .pay.ToAddressWithData(address, { kind: "inline", value: datum }, value)
  .complete();

if (!env.submit) {
  console.log(jsonStringify({
    submit: false,
    address,
    datum,
    value,
    message:
      "Dry-run only. Transaction is intentionally unsigned and no CBOR is printed.",
  }));
} else {
  const signed = await tx.sign.withWallet().complete();
  const txHash = await submitSignedTx(env, signed);
  const outputIndex = await fetchSubmittedOutputIndex(env, txHash, address, datum);
  console.log(
    jsonStringify({ submit: true, txHash, outputIndex, address, datum, value }),
  );
}
