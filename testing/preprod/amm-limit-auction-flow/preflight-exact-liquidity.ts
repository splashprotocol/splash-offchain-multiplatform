import { Data } from "@lucid-evolution/lucid";
import { LimitOrderLimitOrder } from "./src/plutus.ts";
import {
  activeAuctionPrice,
  exactAuctionOutput,
  inversePrice,
  jsonStringify,
  selectedSpanBounds,
} from "./src/auction.ts";
import { readFlowEnv } from "./src/env.ts";
import { makeLucid } from "./src/lucid.ts";

const env = await readFlowEnv();
const now = BigInt(Math.floor(Date.now() / 1000));
const span = selectedSpanBounds(env, now);
if (span.remaining < env.minSpanRemainingSecs) {
  throw new Error(
    `Only ${span.remaining}s remain in auction span ${span.span}; need at least ${env.minSpanRemainingSecs}s`,
  );
}
const activePrice = activeAuctionPrice(env, now);
const counterPrice = inversePrice(activePrice);
const exact = exactAuctionOutput(
  env.auctionBaseAmount,
  activePrice.num,
  activePrice.denom,
);

if (
  env.targetCounterOrderPriceNum === undefined ||
  env.targetCounterOrderPriceDenom === undefined
) {
  throw new Error(
    "TARGET_COUNTER_ORDER_PRICE_NUM/DENOM are required for TARGET_LIQUIDITY_MODE=counterOrder",
  );
}
if (
  env.targetCounterOrderPriceNum !== counterPrice.num ||
  env.targetCounterOrderPriceDenom !== counterPrice.denom
) {
  throw new Error(
    `Counter-order price ${env.targetCounterOrderPriceNum}/${env.targetCounterOrderPriceDenom} must equal inverse active auction price ${counterPrice.num}/${counterPrice.denom}`,
  );
}

if (
  env.targetCounterOrderTxHash === undefined ||
  env.targetCounterOrderOutputIndex === undefined
) {
  throw new Error(
    "TARGET_COUNTER_ORDER_TX_HASH and TARGET_COUNTER_ORDER_OUTPUT_INDEX are required before preflight",
  );
}

let counterDatum = Deno.env.get("TARGET_COUNTER_ORDER_DATUM");
if (!counterDatum) {
  const lucid = await makeLucid(env);
  const [counter] = await lucid.utxosByOutRef([{
    txHash: env.targetCounterOrderTxHash,
    outputIndex: Number(env.targetCounterOrderOutputIndex),
  }]);
  if (!counter?.datum) {
    throw new Error(
      `Counter-order UTxO ${env.targetCounterOrderTxHash}#${env.targetCounterOrderOutputIndex} is missing or has no inline datum`,
    );
  }
  counterDatum = counter.datum;
}
const decoded = Data.from(counterDatum, LimitOrderLimitOrder.conf) as {
  input: { policy: string; name: string };
  output: { policy: string; name: string };
  tradableInput: bigint;
  basePrice: { num: bigint; denom: bigint };
};
if (
  decoded.input.policy !== env.auctionQuote.policy ||
  decoded.input.name !== env.auctionQuote.nameHex
) {
  throw new Error("Counter order input must equal auction quote asset");
}
if (
  decoded.output.policy !== env.auctionBase.policy ||
  decoded.output.name !== env.auctionBase.nameHex
) {
  throw new Error("Counter order output must equal auction base asset");
}
if (
  decoded.basePrice.num !== counterPrice.num ||
  decoded.basePrice.denom !== counterPrice.denom
) {
  throw new Error(
    "Counter order datum price must equal inverse active auction price",
  );
}
if (decoded.tradableInput < exact) {
  throw new Error(
    `Counter order tradable input ${decoded.tradableInput} is less than auction exact output ${exact}`,
  );
}

console.log(jsonStringify({
  status: "ok",
  activePrice,
  counterPrice,
  span,
  exactOutput: exact.toString(),
  counterOrder:
    `${env.targetCounterOrderTxHash}#${env.targetCounterOrderOutputIndex}`,
}));
