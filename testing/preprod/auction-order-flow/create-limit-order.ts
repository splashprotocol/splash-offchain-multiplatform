import {
  Data,
  fromHex,
  paymentCredentialOf,
  stakeCredentialOf,
  UTxO,
} from "@lucid-evolution/lucid";
import {
  credentialToAddress,
  mintingPolicyToId,
  scriptFromNative,
} from "@lucid-evolution/utils";
import { blake2b } from "hash-wasm";
import { jsonStringify, unitOf } from "./src/auction.ts";
import { assetVar, bigintVar, boolVar, readDemoEnv } from "./src/demo-env.ts";
import { Asset } from "./src/env.ts";
import { cardanoNetwork } from "./src/env.ts";
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

function addAssetValue(
  value: Record<string, bigint>,
  unit: string,
  amount: bigint,
) {
  if (amount === 0n) return;
  if (unit === "") {
    value.lovelace = (value.lovelace ?? 0n) + amount;
  } else {
    value[unit] = (value[unit] ?? 0n) + amount;
  }
}

function optional(
  vars: Record<string, string>,
  key: string,
): string | undefined {
  return vars[key] || undefined;
}

function textToHex(text: string): string {
  return Array.from(new TextEncoder().encode(text))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

function hasAssetEnv(
  vars: Record<string, string>,
  prefix: string,
  fallbackPrefix?: string,
): boolean {
  return optional(vars, `${prefix}_POLICY`) !== undefined ||
    optional(vars, `${prefix}_NAME_HEX`) !== undefined ||
    (fallbackPrefix !== undefined &&
      (optional(vars, `${fallbackPrefix}_POLICY`) !== undefined ||
        optional(vars, `${fallbackPrefix}_NAME_HEX`) !== undefined));
}

const { env, vars } = await readDemoEnv();
const tradableInput = bigintVar(vars, "LIMIT_TRADABLE_INPUT", "2000");
const costPerExStep = bigintVar(vars, "LIMIT_COST_PER_EX_STEP", "600000");
const minMarginalOutput = bigintVar(vars, "LIMIT_MIN_MARGINAL_OUTPUT", "1");
const basePriceNum = bigintVar(vars, "LIMIT_BASE_PRICE_NUM", "1");
const basePriceDenom = bigintVar(vars, "LIMIT_BASE_PRICE_DENOM", "1");
const fee = bigintVar(vars, "LIMIT_FEE", "500000");
const lovelaceBudget = bigintVar(vars, "LIMIT_LOVELACE_BUDGET", "1500000");
const expectedOrderOutputIndex = 0n;

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
const paymentKeyHash = paymentCredentialOf(walletAddress).hash;
const nativePolicy = scriptFromNative({ type: "sig", keyHash: paymentKeyHash });
const nativePolicyId = mintingPolicyToId(nativePolicy);
const runSuffix = optional(vars, "LIMIT_TOKEN_SUFFIX") ??
  new Date().toISOString().replaceAll(/[-:.TZ]/g, "").slice(0, 14);
const mintDemoInput = boolVar(vars, "LIMIT_MINT_DEMO_INPUT") ||
  !hasAssetEnv(vars, "LIMIT_INPUT", "AUCTION_QUOTE");
const inputAsset: Asset = mintDemoInput
  ? {
    policy: nativePolicyId,
    nameHex: optional(vars, "LIMIT_DEMO_INPUT_NAME_HEX") ??
      textToHex(`limitIn-${runSuffix}`),
  }
  : assetVar(vars, "LIMIT_INPUT", "AUCTION_QUOTE", true);
const outputAsset: Asset = hasAssetEnv(vars, "LIMIT_OUTPUT", "AUCTION_BASE")
  ? assetVar(vars, "LIMIT_OUTPUT", "AUCTION_BASE", true)
  : { policy: "", nameHex: "" };
const inputUnit = unitOf(inputAsset);

function assetQuantity(utxo: UTxO, unit: string): bigint {
  return BigInt(utxo.assets[unit] ?? 0);
}

function isPlainWalletUtxo(utxo: UTxO): boolean {
  return !utxo.scriptRef && !utxo.datum && !utxo.datumHash;
}

const walletUtxos = await lucid.wallet().getUtxos();
const plainUtxos = walletUtxos.filter(isPlainWalletUtxo);
const input = mintDemoInput
  ? plainUtxos.find((utxo) => assetQuantity(utxo, "lovelace") > 0n)
  : plainUtxos.find((utxo) => assetQuantity(utxo, inputUnit) >= tradableInput);
if (!input) {
  throw new Error(
    mintDemoInput
      ? "Wallet has no plain UTxOs for limit order beacon derivation"
      : `Wallet has no plain UTxO with ${tradableInput} ${inputUnit}`,
  );
}

const selectedInputs = [input];
let selectedLovelace = assetQuantity(input, "lovelace");
const requiredLovelace = lovelaceBudget + fee;
if (selectedLovelace < requiredLovelace) {
  const supplements = plainUtxos
    .filter((utxo) =>
      !(utxo.txHash === input.txHash && utxo.outputIndex === input.outputIndex)
    )
    .filter((utxo) => assetQuantity(utxo, inputUnit) === 0n)
    .sort((left, right) =>
      Number(assetQuantity(right, "lovelace") - assetQuantity(left, "lovelace"))
    );
  for (const supplement of supplements) {
    selectedInputs.push(supplement);
    selectedLovelace += assetQuantity(supplement, "lovelace");
    if (selectedLovelace >= requiredLovelace) break;
  }
  if (selectedLovelace < requiredLovelace) {
    throw new Error(
      `Selected input has ${selectedLovelace} lovelace but needs ${requiredLovelace}`,
    );
  }
}

function limitOrderDatum(beacon: string): string {
  return Data.to({
    tag: "01",
    beacon,
    input: { policy: inputAsset.policy, name: inputAsset.nameHex },
    tradableInput,
    costPerExStep,
    minMarginalOutput,
    output: { policy: outputAsset.policy, name: outputAsset.nameHex },
    basePrice: { num: basePriceNum, denom: basePriceDenom },
    fee,
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

const datumWithErasedBeacon = limitOrderDatum("00".repeat(28));
const beacon = await beaconFromInput(
  input,
  datumWithErasedBeacon,
  expectedOrderOutputIndex,
);
const datum = limitOrderDatum(beacon);

const value: Record<string, bigint> = {
  lovelace: lovelaceBudget + fee,
};
addAssetValue(value, unitOf(inputAsset), tradableInput);

let txBuilder = lucid
  .newTx()
  .collectFrom(selectedInputs);
if (mintDemoInput) {
  txBuilder = txBuilder
    .attach.MintingPolicy(nativePolicy)
    .mintAssets({ [unitOf(inputAsset)]: tradableInput });
}
const tx = await txBuilder
  .pay.ToAddressWithData(orderAddress, { kind: "inline", value: datum }, value)
  .complete();

const output = {
  submit: env.submit,
  address: orderAddress,
  datum,
  value,
  inputAsset,
  outputAsset,
  mintDemoInput,
  tradableInput,
  basePrice: { num: basePriceNum, denom: basePriceDenom },
  fee,
  lovelaceBudget,
};

if (!env.submit) {
  console.log(jsonStringify({
    ...output,
    message: "Dry-run only. Set SUBMIT=1 to sign and submit the limit order.",
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
      `Limit order output index ${outputIndex} did not match beacon index ${expectedOrderOutputIndex}`,
    );
  }
  console.log(jsonStringify({ ...output, txHash, outputIndex }));
}
