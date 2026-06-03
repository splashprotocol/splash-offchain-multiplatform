import { Data, paymentCredentialOf } from "@lucid-evolution/lucid";
import { credentialToAddress } from "@lucid-evolution/utils";
import { Asset, cardanoNetwork, FlowEnv } from "./env.ts";
import { AuctionAuction } from "./plutus.ts";

export function unitOf(asset: Asset): string {
  return `${asset.policy}${asset.nameHex}`;
}

export function auctionAddress(env: FlowEnv): string {
  return credentialToAddress(cardanoNetwork(env), {
    type: "Script",
    hash: env.auctionValidatorHash,
  });
}

export function buildAuctionDatum(
  env: FlowEnv,
  redeemerAddress: string,
): string {
  const pkh = paymentCredentialOf(redeemerAddress).hash;
  return Data.to({
    base: { policy: env.auctionBase.policy, name: env.auctionBase.nameHex },
    quote: { policy: env.auctionQuote.policy, name: env.auctionQuote.nameHex },
    priceStart: {
      num: env.auctionPriceStartNum,
      denom: env.auctionPriceStartDenom,
    },
    startTime: env.auctionStartTimePosix * 1000n,
    stepLen: env.auctionStepLenSecs * 1000n,
    steps: env.auctionSteps,
    priceDacayNum: env.auctionPriceDecayNum,
    feePerQuote: {
      num: env.auctionFeePerQuoteNum,
      denom: env.auctionFeePerQuoteDenom,
    },
    redeemer: pkh,
  }, AuctionAuction.conf);
}

export function decodeAuctionDatum(datum: string): unknown {
  return Data.from(datum, AuctionAuction.conf);
}

export type DecodedAuctionDatum = {
  base: { policy: string; name: string };
  quote: { policy: string; name: string };
  priceStart: { num: bigint; denom: bigint };
  startTime: bigint;
  stepLen: bigint;
  steps: bigint;
  priceDacayNum: bigint;
  feePerQuote: { num: bigint; denom: bigint };
  redeemer: string;
};

export function auctionDatumMatchesEnv(datum: string, env: FlowEnv): boolean {
  const decoded = decodeAuctionDatum(datum) as DecodedAuctionDatum;
  return decoded.base.policy === env.auctionBase.policy &&
    decoded.base.name === env.auctionBase.nameHex &&
    decoded.quote.policy === env.auctionQuote.policy &&
    decoded.quote.name === env.auctionQuote.nameHex &&
    decoded.priceStart.num === env.auctionPriceStartNum &&
    decoded.priceStart.denom === env.auctionPriceStartDenom &&
    decoded.startTime === env.auctionStartTimePosix * 1000n &&
    decoded.stepLen === env.auctionStepLenSecs * 1000n &&
    decoded.steps === env.auctionSteps &&
    decoded.priceDacayNum === env.auctionPriceDecayNum &&
    decoded.feePerQuote.num === env.auctionFeePerQuoteNum &&
    decoded.feePerQuote.denom === env.auctionFeePerQuoteDenom;
}

export function auctionDatumRedeemer(datum: string): string {
  return (decodeAuctionDatum(datum) as DecodedAuctionDatum).redeemer;
}

export function exactAuctionOutput(
  input: bigint,
  priceNum: bigint,
  priceDenom: bigint,
): bigint {
  return (input * priceNum) / priceDenom;
}

export function activeAuctionSpan(env: FlowEnv, nowPosix: bigint): bigint {
  if (nowPosix < env.auctionStartTimePosix) return 0n;
  const span = (nowPosix - env.auctionStartTimePosix) / env.auctionStepLenSecs;
  if (span >= env.auctionSteps) {
    throw new Error("Auction is expired at current time");
  }
  return span;
}

export function selectedSpanBounds(
  env: FlowEnv,
  nowPosix: bigint,
): { span: bigint; low: bigint; high: bigint; remaining: bigint } {
  const span = activeAuctionSpan(env, nowPosix);
  const low = env.auctionStartTimePosix + env.auctionStepLenSecs * span;
  const high = low + env.auctionStepLenSecs;
  return { span, low, high, remaining: high - nowPosix };
}

export function activeAuctionPrice(
  env: FlowEnv,
  nowPosix: bigint,
): { num: bigint; denom: bigint } {
  const span = activeAuctionSpan(env, nowPosix);
  let num = env.auctionPriceStartNum;
  let denom = env.auctionPriceStartDenom;
  for (let i = 0n; i < span; i++) {
    num *= env.auctionPriceDecayNum;
    denom *= 1000n;
  }
  return { num, denom };
}

export function inversePrice(
  price: { num: bigint; denom: bigint },
): { num: bigint; denom: bigint } {
  return { num: price.denom, denom: price.num };
}

export function jsonStringify(value: unknown): string {
  return JSON.stringify(
    value,
    (_key, val) => typeof val === "bigint" ? val.toString() : val,
    2,
  );
}
