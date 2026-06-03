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
  auctionAddress,
  buildAuctionDatum,
  exactAuctionOutput,
  inversePrice,
  jsonStringify,
  selectedSpanBounds,
  unitOf,
} from "./src/auction.ts";
import { cardanoNetwork, readFlowEnv } from "./src/env.ts";
import { makeLucid } from "./src/lucid.ts";
import { LimitOrderLimitOrder } from "./src/plutus.ts";
import { submitSignedTx } from "./src/submit.ts";

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
const counterAddress = credentialToAddress(cardanoNetwork(env), {
  hash: deployment.limitOrder.hash,
  type: "Script",
});
const auctionAddr = auctionAddress(env);
const walletAddress = await lucid.wallet().address();

function isPlainWalletUtxo(utxo: UTxO): boolean {
  return !utxo.scriptRef && !utxo.datum && !utxo.datumHash;
}

function assetQuantity(utxo: UTxO, unit: string): bigint {
  return BigInt(utxo.assets[unit] ?? 0);
}

const baseUnit = unitOf(env.auctionBase);
const quoteUnit = unitOf(env.auctionQuote);

const orderInputTxHash = env.orderInputTxHash ?? env.auctionValidatorRefTxHash;
const listedWalletUtxos = await lucid.wallet().getUtxos();
const directOrderInputUtxos = env.orderInputTxHash
  ? (await lucid.utxosByOutRef(
    Array.from({ length: 16 }, (_, outputIndex) => ({
      txHash: orderInputTxHash,
      outputIndex,
    })),
  )).filter((utxo) => utxo.address === walletAddress)
  : [];
const seenUtxos = new Set<string>();
const walletUtxos = [...directOrderInputUtxos, ...listedWalletUtxos].filter(
  (utxo) => {
    const ref = `${utxo.txHash}#${utxo.outputIndex}`;
    if (seenUtxos.has(ref)) return false;
    seenUtxos.add(ref);
    return true;
  },
);
const freshPlainUtxos = walletUtxos.filter((utxo) =>
  utxo.txHash === orderInputTxHash && isPlainWalletUtxo(utxo)
);

const input = freshPlainUtxos.find((utxo) =>
  assetQuantity(utxo, baseUnit) >= env.auctionBaseAmount &&
  assetQuantity(utxo, quoteUnit) >= tradableInput
);
if (!input) {
  throw new Error(
    `Wallet has no order input UTxO from ${orderInputTxHash} with ` +
      `${env.auctionBaseAmount} ${baseUnit} and ${tradableInput} ${quoteUnit}`,
  );
}

const requiredOrderLovelace = env.auctionLovelaceBudget +
  env.counterOrderLovelaceBudget + env.counterOrderFee;
const selectedInputs = [input];
let selectedLovelace = assetQuantity(input, "lovelace");

if (selectedLovelace < requiredOrderLovelace) {
  const supplement = freshPlainUtxos
    .filter((utxo) =>
      !(utxo.txHash === input.txHash && utxo.outputIndex === input.outputIndex)
    )
    .filter((utxo) =>
      Object.keys(utxo.assets).every((unit) => unit === "lovelace")
    )
    .sort((left, right) =>
      Number(assetQuantity(right, "lovelace") - assetQuantity(left, "lovelace"))
    )[0];
  if (!supplement) {
    throw new Error(
      `Fresh setup UTxO has ${selectedLovelace} lovelace but needs ` +
        `${requiredOrderLovelace}, and no non-collateral ADA supplement was found`,
    );
  }
  selectedInputs.push(supplement);
  selectedLovelace += assetQuantity(supplement, "lovelace");
}

if (selectedLovelace < requiredOrderLovelace) {
  throw new Error(
    `Selected fresh setup inputs have ${selectedLovelace} lovelace but need ` +
      `${requiredOrderLovelace}`,
  );
}

const counterCostPerExStep = 600_000n;
const counterMinMarginalOutput = env.auctionBaseAmount;
const counterOutputIndex = 0n;
const auctionOutputIndex = 1n;

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
  counterOutputIndex,
);
const counterDatumValue = counterDatum(beacon);
const counterValue = {
  lovelace: env.counterOrderLovelaceBudget + env.counterOrderFee,
  [unitOf(env.auctionQuote)]: tradableInput,
};

const auctionDatumValue = buildAuctionDatum(env, walletAddress);
const auctionValue = {
  lovelace: env.auctionLovelaceBudget,
  [unitOf(env.auctionBase)]: env.auctionBaseAmount,
};

const tx = await lucid
  .newTx()
  .collectFrom(selectedInputs)
  .pay.ToAddressWithData(
    counterAddress,
    { kind: "inline", value: counterDatumValue },
    counterValue,
  )
  .pay.ToAddressWithData(
    auctionAddr,
    { kind: "inline", value: auctionDatumValue },
    auctionValue,
  )
  .complete();

const output = {
  submit: env.submit,
  activePrice,
  counterPrice,
  counterPriceNum: counterPrice.num,
  counterPriceDenom: counterPrice.denom,
  exactOutput: tradableInput,
  span,
  counter: {
    submit: env.submit,
    address: counterAddress,
    datum: counterDatumValue,
    value: counterValue,
    activePrice,
    counterPrice,
    counterPriceNum: counterPrice.num,
    counterPriceDenom: counterPrice.denom,
    exactOutput: tradableInput,
    span,
    outputIndex: Number(counterOutputIndex),
  },
  auction: {
    submit: env.submit,
    address: auctionAddr,
    datum: auctionDatumValue,
    value: auctionValue,
    outputIndex: Number(auctionOutputIndex),
  },
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
  console.log(jsonStringify({
    ...output,
    submit: true,
    txHash,
    counter: { ...output.counter, submit: true, txHash },
    auction: { ...output.auction, submit: true, txHash },
  }));
}
