import {
  Blockfrost,
  CML,
  Data,
  fromHex,
  Koios,
  Lucid,
  Maestro,
  paymentCredentialOf,
} from "@lucid-evolution/lucid";
import {
  mintingPolicyToId,
  scriptFromNative,
  validatorToAddress,
} from "@lucid-evolution/utils";
import { load } from "@std/dotenv";
import { mnemonicToEntropy } from "npm:bip39@3.1.0";

type PlutusBlueprint = {
  validators: Array<{
    title: string;
    hash: string;
    compiledCode: string;
  }>;
};

const flowDir = "testing/preprod/auction-order-flow";
const runDir = `${flowDir}/.run`;
const envPath = Deno.env.get("FLOW_ENV_FILE") ?? `${flowDir}/.env`;
const seedPath = Deno.env.get("WALLET_SEED_FILE") ??
  `${flowDir}/wallet.seed`;
const blueprintPath = Deno.env.get("AUCTION_BLUEPRINT_PATH");
const nodeSocket = Deno.env.get("NODE_SOCKET");
if (!nodeSocket) {
  throw new Error("NODE_SOCKET is required");
}

function textToHex(text: string): string {
  return Array.from(new TextEncoder().encode(text))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

const tokenSuffix = Deno.env.get("SETUP_TOKEN_SUFFIX");
const baseNameHex = Deno.env.get("SETUP_AUCTION_BASE_NAME_HEX") ??
  (tokenSuffix ? textToHex(`auctionBase-${tokenSuffix}`) : undefined) ??
  Deno.env.get("AUCTION_BASE_NAME_HEX") ??
  "61756374696f6e42617365";
const quoteNameHex = Deno.env.get("SETUP_AUCTION_QUOTE_NAME_HEX") ??
  (tokenSuffix ? textToHex(`auctionQuote-${tokenSuffix}`) : undefined) ??
  Deno.env.get("AUCTION_QUOTE_NAME_HEX") ??
  "61756374696f6e51756f7465";

const jsonStringify = (value: unknown) =>
  JSON.stringify(
    value,
    (_key, val) => typeof val === "bigint" ? val.toString() : val,
    2,
  );

function required(vars: Record<string, string>, key: string): string {
  const value = vars[key];
  if (!value) throw new Error(`Missing required env: ${key}`);
  return value;
}

function rootKeyFromSeed(seed: string): CML.Bip32PrivateKey {
  const entropy = mnemonicToEntropy(seed.trim());
  return CML.Bip32PrivateKey.from_bip39_entropy(
    fromHex(entropy),
    new Uint8Array(),
  );
}

function bytesFromHex(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) throw new Error("Expected even-length hex");
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

function validatorAddressFromHash(hash: string): string {
  return CML.EnterpriseAddress
    .new(0, CML.Credential.new_script(CML.ScriptHash.from_hex(hash)))
    .to_address()
    .to_bech32(undefined);
}

async function submitViaKoios(txCbor: string): Promise<string> {
  const res = await fetch("https://preprod.koios.rest/api/v1/submittx", {
    method: "POST",
    headers: {
      "content-type": "application/cbor",
      accept: "application/json",
    },
    body: bytesFromHex(txCbor),
  });
  if (!res.ok) {
    throw new Error(
      `Koios submit failed: ${res.status} ${await res.text()}`,
    );
  }
  return await res.json();
}

function blockfrostBaseUrl(vars: Record<string, string>): string {
  return vars["BLOCKFROST_BASE_URL"] ??
    "https://cardano-preprod.blockfrost.io/api/v0";
}

async function submitSignedTx(
  vars: Record<string, string>,
  signed: { submit(): Promise<string>; toCBOR(): string },
): Promise<string> {
  if ((vars["PROVIDER"] ?? "koios") === "koios") {
    return await submitViaKoios(signed.toCBOR());
  }
  return await signed.submit();
}

function agentAddresses(rootKey: CML.Bip32PrivateKey): {
  operatorKey: string;
  collateral: string;
  funding: string[];
} {
  const operatorKey = rootKey.to_bech32();
  const operatorPk = rootKey.to_public();
  const mainPkh = operatorPk.to_raw_key().hash();
  const mainCred = CML.Credential.new_pub_key(mainPkh);
  const collateral = CML.EnterpriseAddress.new(0, mainCred)
    .to_address()
    .to_bech32(undefined);
  const funding = [1, 2, 3, 4].map((ix) => {
    const stakePkh = operatorPk.derive(ix).to_raw_key().hash();
    return CML.BaseAddress.new(
      0,
      mainCred,
      CML.Credential.new_pub_key(stakePkh),
    ).to_address().to_bech32(undefined);
  });
  return { operatorKey, collateral, funding };
}

async function submittedOutputIndex(
  vars: Record<string, string>,
  txHash: string,
  predicate: (output: {
    tx_index: number;
    output_index?: number;
    payment_addr?: { bech32: string };
    address?: string;
    reference_script?: { hash?: string } | null;
    reference_script_hash?: string | null;
  }) => boolean,
): Promise<number> {
  if ((vars["PROVIDER"] ?? "koios") === "blockfrost") {
    const headers = { project_id: required(vars, "BLOCKFROST_PROJECT_ID") };
    const url = `${blockfrostBaseUrl(vars)}/txs/${txHash}/utxos`;
    for (let attempt = 0; attempt < 60; attempt++) {
      const res = await fetch(url, { headers });
      if (res.ok) {
        const tx = await res.json();
        const output = tx.outputs?.find((
          out: {
            output_index: number;
            address?: string;
            reference_script_hash?: string | null;
          },
        ) => predicate({
          tx_index: out.output_index,
          output_index: out.output_index,
          address: out.address,
          reference_script_hash: out.reference_script_hash,
        }));
        if (output) return output.output_index;
      }
      await new Promise((resolve) => setTimeout(resolve, 2000));
    }
    throw new Error(`Submitted tx ${txHash}, but expected output was not found`);
  }
  const url = "https://preprod.koios.rest/api/v1/tx_info";
  const body = JSON.stringify({ _tx_hashes: [txHash], _scripts: true });
  const headers = { "content-type": "application/json" };
  for (let attempt = 0; attempt < 60; attempt++) {
    const res = await fetch(url, { method: "POST", headers, body });
    if (res.ok) {
      const txs = await res.json();
      const output = txs[0]?.outputs?.find(predicate);
      if (output) return output.tx_index;
    }
    await new Promise((resolve) => setTimeout(resolve, 2000));
  }
  throw new Error(`Submitted tx ${txHash}, but expected output was not found`);
}

await Deno.mkdir(runDir, { recursive: true });
const dotenv = await load({ envPath, export: false }).catch(() => ({}));
const vars = { ...dotenv, ...Deno.env.toObject() };

const seed = await Deno.readTextFile(seedPath);
const provider = vars["PROVIDER"] ?? "koios";
const lucid = await Lucid(
  provider === "blockfrost"
    ? new Blockfrost(blockfrostBaseUrl(vars), required(vars, "BLOCKFROST_PROJECT_ID"))
    : provider === "maestro"
    ? new Maestro({
      network: "Preprod",
      apiKey: required(vars, "MAESTRO_API_KEY"),
      turboSubmit: false,
    })
    : new Koios("https://preprod.koios.rest/api/v1"),
  "Preprod",
);
lucid.selectWallet.fromSeed(seed.trim());

const walletAddress = await lucid.wallet().address();
const paymentKeyHash = paymentCredentialOf(walletAddress).hash;
const nativePolicy = scriptFromNative({ type: "sig", keyHash: paymentKeyHash });
const policy = mintingPolicyToId(nativePolicy);
const baseUnit = `${policy}${baseNameHex}`;
const quoteUnit = `${policy}${quoteNameHex}`;

const rootKey = rootKeyFromSeed(seed);
const addresses = agentAddresses(rootKey);
const runId = Deno.env.get("RUN_ID") ?? `${Date.now()}`;
const baseDeploymentConfig = vars["DEPLOYMENT_CONFIG"] ??
  "bloom-cardano-agent/resources/preprod.deployment.json";
const runtimeDeploymentConfig = `${runDir}/preprod.deployment.${runId}.json`;
const deployment = JSON.parse(await Deno.readTextFile(baseDeploymentConfig));
const blueprint = blueprintPath
  ? JSON.parse(await Deno.readTextFile(blueprintPath)) as PlutusBlueprint
  : undefined;
const auctionValidator = blueprint?.validators.find((validator) =>
  validator.title === "orders/auction.auction"
) ?? {
  title: "orders/auction.auction",
  hash: required(vars, "AUCTION_VALIDATOR_HASH"),
  compiledCode: "",
};
if (!auctionValidator) {
  throw new Error(`Auction validator not found in ${blueprintPath}`);
}
const limitOrderValidator = blueprint?.validators.find((validator) =>
  validator.title === "orders/limit_order.limit_order"
) ?? {
  title: "orders/limit_order.limit_order",
  hash: deployment.limitOrder.hash,
  compiledCode: "",
};
if (!limitOrderValidator) {
  throw new Error(`Limit order validator not found in ${blueprintPath}`);
}
const limitOrderWitnessValidator = blueprint?.validators.find((validator) =>
  validator.title === "orders/limit_order.batch_witness"
) ?? {
  title: "orders/limit_order.batch_witness",
  hash: deployment.limitOrderWitness.hash,
  compiledCode: "",
};
if (!limitOrderWitnessValidator) {
  throw new Error(
    `Limit order witness validator not found in ${blueprintPath}`,
  );
}
const auctionScript = blueprint
  ? {
    type: "PlutusV2" as const,
    script: auctionValidator.compiledCode,
  }
  : undefined;
const limitOrderScript = blueprint
  ? {
    type: "PlutusV2" as const,
    script: limitOrderValidator.compiledCode,
  }
  : undefined;
const limitOrderWitnessScript = blueprint
  ? {
    type: "PlutusV2" as const,
    script: limitOrderWitnessValidator.compiledCode,
  }
  : undefined;
const auctionAddress = auctionScript
  ? validatorToAddress("Preprod", auctionScript)
  : validatorAddressFromHash(auctionValidator.hash);

const existingSetupTxHash = vars["SETUP_TX_HASH"] ??
  (!blueprint ? required(vars, "AUCTION_VALIDATOR_REF_TX_HASH") : undefined);
const useFreshLimitOrderRefs = vars["USE_FRESH_LIMIT_ORDER_REFS"] === "1";
const fundingLovelace = BigInt(vars["AGENT_FUNDING_LOVELACE"] ?? "50000000");
const orderInputLovelace =
  BigInt(vars["AUCTION_LOVELACE_BUDGET"] ?? "3000000") +
  BigInt(vars["COUNTER_ORDER_LOVELACE_BUDGET"] ?? "3000000") +
  BigInt(vars["COUNTER_ORDER_FEE"] ?? "500000");
const txHash = existingSetupTxHash ?? await (async () => {
  if (!auctionScript || !limitOrderScript || !limitOrderWitnessScript) {
    throw new Error("AUCTION_BLUEPRINT_PATH is required to deploy validator references");
  }
  const tx = await lucid
    .newTx()
    .pay.ToAddressWithData(
      auctionAddress,
      { kind: "inline", value: Data.to(0n) },
      { lovelace: 5_000_000n },
      auctionScript,
    )
    .pay.ToAddressWithData(
      auctionAddress,
      { kind: "inline", value: Data.to(1n) },
      { lovelace: 5_000_000n },
      limitOrderScript,
    )
    .pay.ToAddressWithData(
      auctionAddress,
      { kind: "inline", value: Data.to(2n) },
      { lovelace: 5_000_000n },
      limitOrderWitnessScript,
    )
    .attach.MintingPolicy(nativePolicy)
    .mintAssets({
      [baseUnit]: 1_000_000n,
      [quoteUnit]: 1_000_000n,
    })
    .pay.ToAddress(walletAddress, {
      lovelace: orderInputLovelace,
      [baseUnit]: 1_000_000n,
      [quoteUnit]: 1_000_000n,
    })
    .pay.ToAddress(addresses.collateral, { lovelace: 10_000_000n })
    .pay.ToAddress(addresses.funding[0], { lovelace: fundingLovelace })
    .pay.ToAddress(addresses.funding[1], { lovelace: fundingLovelace })
    .pay.ToAddress(addresses.funding[2], { lovelace: fundingLovelace })
    .pay.ToAddress(addresses.funding[3], { lovelace: fundingLovelace })
    .complete();
  const signed = await tx.sign.withWallet().complete();
  return await submitSignedTx(vars, signed);
})();
const refOutputIndex = blueprint
  ? await submittedOutputIndex(
    vars,
    txHash,
    (out) =>
      (out.payment_addr?.bech32 ?? out.address) === auctionAddress &&
      (out.reference_script?.hash ?? out.reference_script_hash) ===
        auctionValidator.hash,
  )
  : Number(required(vars, "AUCTION_VALIDATOR_REF_OUTPUT_INDEX"));
const limitOrderRefOutputIndex = useFreshLimitOrderRefs
  ? await submittedOutputIndex(
    vars,
    txHash,
    (out) =>
      (out.payment_addr?.bech32 ?? out.address) === auctionAddress &&
      (out.reference_script?.hash ?? out.reference_script_hash) ===
        limitOrderValidator.hash,
  )
  : undefined;
const limitOrderWitnessRefOutputIndex = useFreshLimitOrderRefs
  ? await submittedOutputIndex(
    vars,
    txHash,
    (out) =>
      (out.payment_addr?.bech32 ?? out.address) === auctionAddress &&
      (out.reference_script?.hash ?? out.reference_script_hash) ===
        limitOrderWitnessValidator.hash,
  )
  : undefined;

let mintTxHash: string | undefined;
if (vars["MINT_TX_HASH"]) {
  mintTxHash = vars["MINT_TX_HASH"];
} else if (existingSetupTxHash) {
  const mintTx = await lucid
    .newTx()
    .attach.MintingPolicy(nativePolicy)
    .mintAssets({
      [baseUnit]: 1_000_000n,
      [quoteUnit]: 1_000_000n,
    })
    .pay.ToAddress(walletAddress, {
      lovelace: orderInputLovelace,
      [baseUnit]: 1_000_000n,
      [quoteUnit]: 1_000_000n,
    })
    .pay.ToAddress(addresses.collateral, { lovelace: 10_000_000n })
    .pay.ToAddress(addresses.funding[0], { lovelace: fundingLovelace })
    .pay.ToAddress(addresses.funding[1], { lovelace: fundingLovelace })
    .pay.ToAddress(addresses.funding[2], { lovelace: fundingLovelace })
    .pay.ToAddress(addresses.funding[3], { lovelace: fundingLovelace })
    .complete();
  const signedMint = await mintTx.sign.withWallet().complete();
  mintTxHash = await submitSignedTx(vars, signedMint);
}

const now = Math.floor(Date.now() / 1000);
const startTime = now - 60;
if (useFreshLimitOrderRefs) {
  deployment.limitOrder.hash = limitOrderValidator.hash;
  deployment.limitOrder.referenceUtxo = {
    txHash,
    outputIndex: limitOrderRefOutputIndex,
  };
  deployment.limitOrderWitness.hash = limitOrderWitnessValidator.hash;
  deployment.limitOrderWitness.referenceUtxo = {
    txHash,
    outputIndex: limitOrderWitnessRefOutputIndex,
  };
}
await Deno.writeTextFile(runtimeDeploymentConfig, jsonStringify(deployment));

const env = `NETWORK=preprod
SUBMIT=1
PROVIDER=blockfrost
BLOCKFROST_PROJECT_ID=
MAESTRO_API_KEY=
WALLET_SEED_FILE=${seedPath}
NODE_SOCKET=${nodeSocket}
OPERATOR_KEY_CBOR_HEX=${addresses.operatorKey}
AGENT_BIN=target/debug/bloom-cardano-agent
AGENT_HEALTH_ADDR=127.0.0.1:9024
AGENT_READY_TIMEOUT_SECS=300
BUILD_AGENT=1
BASE_AGENT_CONFIG=bloom-cardano-agent/resources/preprod.config.json
DEPLOYMENT_CONFIG=${runtimeDeploymentConfig}
VALIDATION_RULES=bloom-cardano-agent/resources/validation-rules.json.template
LOG4RS_CONFIG=
AUCTION_VALIDATOR_HASH=${auctionValidator.hash}
AUCTION_VALIDATOR_REF_TX_HASH=${txHash}
AUCTION_VALIDATOR_REF_OUTPUT_INDEX=${refOutputIndex}
ORDER_INPUT_TX_HASH=${mintTxHash ?? txHash}
AUCTION_VALIDATOR_COST_MEM=500000
AUCTION_VALIDATOR_COST_STEPS=160000000
AUCTION_VALIDATOR_MARGINAL_COST_MEM=0
AUCTION_VALIDATOR_MARGINAL_COST_STEPS=0
AUCTION_MAX_COST_PER_EX_STEP=500000
AUCTION_MIN_MARGINAL_OUTPUT=1
AUCTION_BASE_POLICY=${policy}
AUCTION_BASE_NAME_HEX=${baseNameHex}
AUCTION_QUOTE_POLICY=${policy}
AUCTION_QUOTE_NAME_HEX=${quoteNameHex}
AUCTION_BASE_AMOUNT=1000
AUCTION_PRICE_START_NUM=2
AUCTION_PRICE_START_DENOM=1
AUCTION_START_TIME_POSIX=${startTime}
AUCTION_STEP_LEN_SECS=600
AUCTION_STEPS=12
AUCTION_PRICE_DECAY_NUM=1000
AUCTION_FEE_PER_QUOTE_NUM=0
AUCTION_FEE_PER_QUOTE_DENOM=1
AUCTION_LOVELACE_BUDGET=3000000
EXISTING_AUCTION_TX_HASH=
EXISTING_AUCTION_OUTPUT_INDEX=
VERIFY_TIMEOUT_SECS=900
VERIFY_POLL_SECS=10
TARGET_LIQUIDITY_MODE=counterOrder
TARGET_COUNTER_ORDER_PRICE_NUM=
TARGET_COUNTER_ORDER_PRICE_DENOM=
TARGET_COUNTER_ORDER_TX_HASH=
TARGET_COUNTER_ORDER_OUTPUT_INDEX=
CREATE_COUNTER_ORDER=1
COUNTER_ORDER_LOVELACE_BUDGET=3000000
COUNTER_ORDER_FEE=500000
MIN_SPAN_REMAINING_SECS=120
`;
await Deno.writeTextFile(envPath, env);

const result = {
  setupTxHash: txHash,
  mintTxHash,
  auctionValidatorHash: auctionValidator.hash,
  auctionValidatorRef: `${txHash}#${refOutputIndex}`,
  useFreshLimitOrderRefs,
  limitOrderValidatorHash: deployment.limitOrder.hash,
  limitOrderValidatorRef:
    `${deployment.limitOrder.referenceUtxo.txHash}#${deployment.limitOrder.referenceUtxo.outputIndex}`,
  limitOrderWitnessValidatorHash: deployment.limitOrderWitness.hash,
  limitOrderWitnessValidatorRef:
    `${deployment.limitOrderWitness.referenceUtxo.txHash}#${deployment.limitOrderWitness.referenceUtxo.outputIndex}`,
  auctionAddress,
  baseUnit,
  quoteUnit,
  collateralAddress: addresses.collateral,
  fundingAddresses: addresses.funding,
  deploymentConfig: runtimeDeploymentConfig,
  envPath,
};
await Deno.writeTextFile(
  `${runDir}/setup-preprod-flow.json`,
  jsonStringify(result),
);
console.log(jsonStringify(result));
