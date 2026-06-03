import { load } from "@std/dotenv";

export type Asset = {
  policy: string;
  nameHex: string;
};

export type ProviderEnv = {
  network: "preprod";
  submit: boolean;
  provider: "blockfrost" | "maestro" | "koios";
  blockfrostProjectId?: string;
  maestroApiKey?: string;
  walletSeedFile: string;
  deploymentConfig: string;
};

export type FlowEnv = ProviderEnv & {
  auctionValidatorHash: string;
  auctionValidatorRefTxHash: string;
  auctionValidatorRefOutputIndex: bigint;
  orderInputTxHash?: string;
  auctionBase: Asset;
  auctionQuote: Asset;
  auctionBaseAmount: bigint;
  auctionPriceStartNum: bigint;
  auctionPriceStartDenom: bigint;
  auctionStartTimePosix: bigint;
  auctionStepLenSecs: bigint;
  auctionSteps: bigint;
  auctionPriceDecayNum: bigint;
  auctionFeePerQuoteNum: bigint;
  auctionFeePerQuoteDenom: bigint;
  auctionLovelaceBudget: bigint;
  minSpanRemainingSecs: bigint;
  targetLiquidityMode: "counterOrder";
  targetCounterOrderPriceNum?: bigint;
  targetCounterOrderPriceDenom?: bigint;
  targetCounterOrderTxHash?: string;
  targetCounterOrderOutputIndex?: bigint;
  createCounterOrder: boolean;
  counterOrderLovelaceBudget: bigint;
  counterOrderFee: bigint;
};

function required(vars: Record<string, string>, key: string): string {
  const value = vars[key];
  if (!value) throw new Error(`Missing required env: ${key}`);
  return value;
}

function requiredDefined(vars: Record<string, string>, key: string): string {
  const value = vars[key];
  if (value === undefined) throw new Error(`Missing required env: ${key}`);
  return value;
}

function optional(
  vars: Record<string, string>,
  key: string,
): string | undefined {
  return vars[key] || undefined;
}

function bigintEnv(vars: Record<string, string>, key: string): bigint {
  return BigInt(required(vars, key));
}

function optionalBigint(
  vars: Record<string, string>,
  key: string,
): bigint | undefined {
  const value = optional(vars, key);
  return value === undefined ? undefined : BigInt(value);
}

function tokenAsset(
  vars: Record<string, string>,
  policyKey: string,
  nameKey: string,
): Asset {
  const policy = requiredDefined(vars, policyKey);
  const nameHex = requiredDefined(vars, nameKey);
  if (policy === "") {
    throw new Error(
      `${policyKey} must be non-empty; ADA-side auctions are unsupported`,
    );
  }
  if (!/^[0-9a-fA-F]{56}$/.test(policy)) {
    throw new Error(`${policyKey} must be 28-byte hex`);
  }
  if (!/^[0-9a-fA-F]*$/.test(nameHex) || nameHex.length % 2 !== 0) {
    throw new Error(
      `${nameKey} must be even-length hex; empty name is allowed`,
    );
  }
  return { policy, nameHex };
}

export async function readFlowEnv(): Promise<FlowEnv> {
  const envPath = Deno.env.get("FLOW_ENV_FILE") ??
    "testing/preprod/auction-order-flow/.env";
  const dotenv = await load({
    envPath,
    export: false,
  });
  const vars = { ...dotenv, ...Deno.env.toObject() };
  const network = optional(vars, "NETWORK") ?? "preprod";
  if (network !== "preprod") {
    throw new Error("This harness is preprod-only; set NETWORK=preprod");
  }
  const auctionBase = tokenAsset(
    vars,
    "AUCTION_BASE_POLICY",
    "AUCTION_BASE_NAME_HEX",
  );
  const auctionQuote = tokenAsset(
    vars,
    "AUCTION_QUOTE_POLICY",
    "AUCTION_QUOTE_NAME_HEX",
  );
  const targetLiquidityMode = optional(vars, "TARGET_LIQUIDITY_MODE") ??
    "counterOrder";
  if (targetLiquidityMode !== "counterOrder") {
    throw new Error(
      "Only TARGET_LIQUIDITY_MODE=counterOrder is supported by this preprod harness",
    );
  }
  return {
    network: "preprod",
    submit: optional(vars, "SUBMIT") === "1",
    provider: (optional(vars, "PROVIDER") ?? "blockfrost") as
      | "blockfrost"
      | "maestro"
      | "koios",
    blockfrostProjectId: optional(vars, "BLOCKFROST_PROJECT_ID"),
    maestroApiKey: optional(vars, "MAESTRO_API_KEY"),
    walletSeedFile: required(vars, "WALLET_SEED_FILE"),
    deploymentConfig: optional(vars, "DEPLOYMENT_CONFIG") ??
      "bloom-cardano-agent/resources/preprod.deployment.json",
    auctionValidatorHash: required(vars, "AUCTION_VALIDATOR_HASH"),
    auctionValidatorRefTxHash: required(vars, "AUCTION_VALIDATOR_REF_TX_HASH"),
    auctionValidatorRefOutputIndex: bigintEnv(
      vars,
      "AUCTION_VALIDATOR_REF_OUTPUT_INDEX",
    ),
    orderInputTxHash: optional(vars, "ORDER_INPUT_TX_HASH"),
    auctionBase,
    auctionQuote,
    auctionBaseAmount: bigintEnv(vars, "AUCTION_BASE_AMOUNT"),
    auctionPriceStartNum: bigintEnv(vars, "AUCTION_PRICE_START_NUM"),
    auctionPriceStartDenom: bigintEnv(vars, "AUCTION_PRICE_START_DENOM"),
    auctionStartTimePosix: bigintEnv(vars, "AUCTION_START_TIME_POSIX"),
    auctionStepLenSecs: bigintEnv(vars, "AUCTION_STEP_LEN_SECS"),
    auctionSteps: bigintEnv(vars, "AUCTION_STEPS"),
    auctionPriceDecayNum: bigintEnv(vars, "AUCTION_PRICE_DECAY_NUM"),
    auctionFeePerQuoteNum: bigintEnv(vars, "AUCTION_FEE_PER_QUOTE_NUM"),
    auctionFeePerQuoteDenom: bigintEnv(vars, "AUCTION_FEE_PER_QUOTE_DENOM"),
    auctionLovelaceBudget: bigintEnv(vars, "AUCTION_LOVELACE_BUDGET"),
    minSpanRemainingSecs: bigintEnv(vars, "MIN_SPAN_REMAINING_SECS"),
    targetLiquidityMode,
    targetCounterOrderPriceNum: optionalBigint(
      vars,
      "TARGET_COUNTER_ORDER_PRICE_NUM",
    ),
    targetCounterOrderPriceDenom: optionalBigint(
      vars,
      "TARGET_COUNTER_ORDER_PRICE_DENOM",
    ),
    targetCounterOrderTxHash: optional(vars, "TARGET_COUNTER_ORDER_TX_HASH"),
    targetCounterOrderOutputIndex: optionalBigint(
      vars,
      "TARGET_COUNTER_ORDER_OUTPUT_INDEX",
    ),
    createCounterOrder: optional(vars, "CREATE_COUNTER_ORDER") !== "0",
    counterOrderLovelaceBudget: bigintEnv(
      vars,
      "COUNTER_ORDER_LOVELACE_BUDGET",
    ),
    counterOrderFee: bigintEnv(vars, "COUNTER_ORDER_FEE"),
  };
}

export function cardanoNetwork(_env: ProviderEnv): "Preprod" {
  return "Preprod";
}

export function blockfrostBaseUrl(_env: ProviderEnv): string {
  return "https://cardano-preprod.blockfrost.io/api/v0";
}
