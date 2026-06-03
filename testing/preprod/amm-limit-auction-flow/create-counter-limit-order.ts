import {
  Data,
  fromHex,
  paymentCredentialOf,
  stakeCredentialOf,
  UTxO,
} from "@lucid-evolution/lucid";
import { credentialToAddress } from "@lucid-evolution/utils";
import { blake2b } from "hash-wasm";
import {
  activeAuctionPrice,
  exactAuctionOutput,
  inversePrice,
  jsonStringify,
  selectedSpanBounds,
  unitOf,
} from "./src/auction.ts";
import { cardanoNetwork, readFlowEnv } from "./src/env.ts";
import { makeLucid } from "./src/lucid.ts";
import { LimitOrderLimitOrder } from "./src/plutus.ts";
import { fetchSubmittedOutputIndex, submitSignedTx } from "./src/submit.ts";

type Deployment = {
  limitOrder: {
    hash: string;
  };
};

function u64be(value: bigint): Uint8Array {
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigUint64(0, value);
  return bytes;
}

async function beaconFromInput(
  utxo: UTxO,
  datumWithErasedBeacon: string,
  orderIndex: bigint,
): Promise<string> {
  const datumHash = fromHex(await blake2b(fromHex(datumWithErasedBeacon), 224));
  return await blake2b(
    Uint8Array.from([
      ...fromHex(utxo.txHash),
      ...u64be(BigInt(utxo.outputIndex)),
      ...u64be(orderIndex),
      ...datumHash,
    ]),
    224,
  );
}

function walletStakeCredential(address: string) {
  try {
    return {
      Inline: [{
        VerificationKeyCredential: [stakeCredentialOf(address).hash],
      }] as [{ VerificationKeyCredential: [string] }],
    };
  } catch (_) {
    return null;
  }
}

const env = await readFlowEnv();
if (!env.createCounterOrder) {
  throw new Error(
    "CREATE_COUNTER_ORDER=0 but create-counter-limit-order.ts was invoked",
  );
}

const now = BigInt(Math.floor(Date.now() / 1000));
const span = selectedSpanBounds(env, now);
if (span.remaining < env.minSpanRemainingSecs) {
  throw new Error(
    `Only ${span.remaining}s remain in auction span ${span.span}; need at least ${env.minSpanRemainingSecs}s`,
  );
}
const activePrice = activeAuctionPrice(env, now);
const counterPrice = inversePrice(activePrice);
const tradableInput = exactAuctionOutput(
  env.auctionBaseAmount,
  activePrice.num,
  activePrice.denom,
);

const lucid = await makeLucid(env);
const seed = await Deno.readTextFile(env.walletSeedFile);
lucid.selectWallet.fromSeed(seed.trim());

const deployment = JSON.parse(
  await Deno.readTextFile(env.deploymentConfig),
) as Deployment;
const orderAddress = credentialToAddress(cardanoNetwork(env), {
  hash: deployment.limitOrder.hash,
  type: "Script",
});
const walletAddress = await lucid.wallet().address();
const walletUtxos = await lucid.wallet().getUtxos();
const input = walletUtxos.find((utxo) =>
  !utxo.scriptRef && !utxo.datum && !utxo.datumHash
);
if (!input) {
  throw new Error(
    "Wallet has no plain UTxOs for counter order beacon derivation",
  );
}
const counterCostPerExStep = 600_000n;
const counterMinMarginalOutput = env.auctionBaseAmount;
const expectedOrderOutputIndex = 0n;

function counterDatum(beacon: string): string {
  return Data.to({
    tag: "01",
    beacon,
    input: { policy: env.auctionQuote.policy, name: env.auctionQuote.nameHex },
    tradableInput,
    costPerExStep: counterCostPerExStep,
    minMarginalOutput: counterMinMarginalOutput,
    output: { policy: env.auctionBase.policy, name: env.auctionBase.nameHex },
    basePrice: counterPrice,
    fee: env.counterOrderFee,
    redeemerAddress: {
      paymentCredential: {
        VerificationKeyCredential: [paymentCredentialOf(walletAddress).hash],
      },
      stakeCredential: walletStakeCredential(walletAddress),
    },
    cancellationPkh: paymentCredentialOf(walletAddress).hash,
    permittedExecutors: [],
  }, LimitOrderLimitOrder.conf);
}

const datumWithErasedBeacon = counterDatum("00".repeat(28));
const beacon = await beaconFromInput(
  input,
  datumWithErasedBeacon,
  expectedOrderOutputIndex,
);
const datum = counterDatum(beacon);

const value = {
  lovelace: env.counterOrderLovelaceBudget + env.counterOrderFee,
  [unitOf(env.auctionQuote)]: tradableInput,
};

const tx = await lucid
  .newTx()
  .collectFrom([input])
  .pay.ToAddressWithData(orderAddress, { kind: "inline", value: datum }, value)
  .complete();

const output = {
  submit: env.submit,
  address: orderAddress,
  datum,
  value,
  activePrice,
  counterPrice,
  counterPriceNum: counterPrice.num,
  counterPriceDenom: counterPrice.denom,
  exactOutput: tradableInput,
  span,
};

if (!env.submit) {
  console.log(jsonStringify({
    ...output,
    submit: false,
    message:
      "Dry-run only. Transaction is intentionally unsigned and no CBOR is printed.",
  }));
} else {
  const signed = await tx.sign.withWallet().complete();
  const txHash = await submitSignedTx(env, signed);
  const outputIndex = await fetchSubmittedOutputIndex(
    env,
    txHash,
    orderAddress,
    datum,
  );
  if (outputIndex !== Number(expectedOrderOutputIndex)) {
    throw new Error(
      `Counter order output index ${outputIndex} did not match beacon index ${expectedOrderOutputIndex}`,
    );
  }
  console.log(jsonStringify({ ...output, submit: true, txHash, outputIndex }));
}
