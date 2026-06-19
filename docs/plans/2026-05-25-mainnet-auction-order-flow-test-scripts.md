# Mainnet Auction Order Flow Test Scripts Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add mainnet-safe scripts that create or observe a Splash auction order, run `bloom-cardano-agent` with auction-order support enabled, and verify the order is executed through the normal agent flow.

**Architecture:** Build a small Deno/Lucid transaction harness under `testing/mainnet/auction-order-flow/` for creating and verifying auction UTxOs, plus Bash orchestration scripts for generating an isolated agent config and running the agent. The scripts default to dry-run/no-submit behavior and require explicit `SUBMIT=1` for any mainnet transaction submission.

**Tech Stack:** Bash, Deno, Lucid Evolution, Blockfrost or Maestro, `jq`, `cargo run --package bloom-cardano-agent`, existing `bloom-cardano-agent/resources/mainnet.*` configuration files.

---

## Constraints And Assumptions

- Mainnet scripts must never submit transactions unless `SUBMIT=1` is present.
- Auction integration currently supports token-token auctions only. The scripts must reject any auction where base or quote is ADA.
- The scripts require an already deployed auction validator reference script UTxO. This is passed through env as `AUCTION_VALIDATOR_HASH`, `AUCTION_VALIDATOR_REF_TX_HASH`, and `AUCTION_VALIDATOR_REF_OUTPUT_INDEX`.
- The live flow needs a real pool or counter-order whose exact output equals the auction span price. The preflight must reject configurations where the selected pool/counter-order would produce better-than-exact output.
- All state, logs, and generated configs must live under `testing/mainnet/auction-order-flow/.run/` and must not reuse production `state/`.
- Mainnet live tests should use tiny token-token amounts and a dedicated wallet.

---

### Task 1: Add Mainnet Auction Flow Environment Template

**Files:**
- Create: `testing/mainnet/auction-order-flow/env.example`
- Create: `testing/mainnet/auction-order-flow/README.md`

**Step 1: Write the environment template**

Create `testing/mainnet/auction-order-flow/env.example`:

```bash
# Copy to .env and fill values. Mainnet submission requires SUBMIT=1.
NETWORK=mainnet
SUBMIT=0

# API provider used by Lucid scripts.
PROVIDER=blockfrost
BLOCKFROST_PROJECT_ID=
MAESTRO_API_KEY=

# Wallet seed file. Use a dedicated low-value mainnet wallet.
WALLET_SEED_FILE=testing/mainnet/auction-order-flow/wallet.seed

# Agent inputs.
NODE_SOCKET=/data/cardano-node/ipc/node.socket
OPERATOR_KEY_CBOR_HEX=
AGENT_BIN=target/debug/bloom-cardano-agent
AGENT_HEALTH_ADDR=127.0.0.1:9024
AGENT_READY_TIMEOUT_SECS=180

# Existing deployment/config files.
BASE_AGENT_CONFIG=bloom-cardano-agent/resources/mainnet.config.json.template
DEPLOYMENT_CONFIG=bloom-cardano-agent/resources/mainnet.deployment.json
VALIDATION_RULES=bloom-cardano-agent/resources/validation-rules.json.template
# Optional override. Leave empty to use the harness run-local stdout log config.
LOG4RS_CONFIG=

# Auction validator reference script.
AUCTION_VALIDATOR_HASH=
AUCTION_VALIDATOR_REF_TX_HASH=
AUCTION_VALIDATOR_REF_OUTPUT_INDEX=
AUCTION_VALIDATOR_COST_MEM=300000
AUCTION_VALIDATOR_COST_STEPS=120000000
AUCTION_VALIDATOR_MARGINAL_COST_MEM=0
AUCTION_VALIDATOR_MARGINAL_COST_STEPS=0
AUCTION_MAX_COST_PER_EX_STEP=500000
AUCTION_MIN_MARGINAL_OUTPUT=1

# Token-token auction config. Empty policy/name means ADA and is rejected by preflight.
AUCTION_BASE_POLICY=
AUCTION_BASE_NAME_HEX=
AUCTION_QUOTE_POLICY=
AUCTION_QUOTE_NAME_HEX=
AUCTION_BASE_AMOUNT=
AUCTION_PRICE_START_NUM=
AUCTION_PRICE_START_DENOM=
AUCTION_START_TIME_POSIX=
AUCTION_STEP_LEN_SECS=60
AUCTION_STEPS=10
AUCTION_PRICE_DECAY_NUM=1000
AUCTION_FEE_PER_QUOTE_NUM=0
AUCTION_FEE_PER_QUOTE_DENOM=1
AUCTION_LOVELACE_BUDGET=3000000

# Optional: run against an already-created auction instead of creating one.
EXISTING_AUCTION_TX_HASH=
EXISTING_AUCTION_OUTPUT_INDEX=

# Expected result polling.
VERIFY_TIMEOUT_SECS=600
VERIFY_POLL_SECS=10

# Exact-liquidity preflight. Mainnet flow creates/uses a paired limit order at
# the active auction price. Pool matching is intentionally not used by this harness.
TARGET_LIQUIDITY_MODE=counterOrder
TARGET_COUNTER_ORDER_PRICE_NUM=
TARGET_COUNTER_ORDER_PRICE_DENOM=
TARGET_COUNTER_ORDER_TX_HASH=
TARGET_COUNTER_ORDER_OUTPUT_INDEX=

# Optional: create a paired limit order during submit flow.
CREATE_COUNTER_ORDER=1
COUNTER_ORDER_LOVELACE_BUDGET=3000000
COUNTER_ORDER_FEE=500000

# Require enough time left in the selected auction span for submit/index/match.
MIN_SPAN_REMAINING_SECS=300
```

**Step 2: Write README**

Create `testing/mainnet/auction-order-flow/README.md` with:

```markdown
# Mainnet Auction Order Flow

This harness tests Splash auction orders through `bloom-cardano-agent`.

Default behavior is dry-run. No mainnet transaction is submitted unless `SUBMIT=1`.

## Modes

- `observe`: do not create an auction; run the agent against `EXISTING_AUCTION_TX_HASH#EXISTING_AUCTION_OUTPUT_INDEX`.
- `submit`: create a token-token auction UTxO, run the agent, and verify it is spent.

## Commands

```bash
cp testing/mainnet/auction-order-flow/env.example testing/mainnet/auction-order-flow/.env
$EDITOR testing/mainnet/auction-order-flow/.env

testing/mainnet/auction-order-flow/run-flow.sh observe
SUBMIT=1 testing/mainnet/auction-order-flow/run-flow.sh submit
```

## Safety

Use a dedicated wallet and tiny token-token amounts. ADA-side auctions are rejected because the current auction integration intentionally does not support ADA-side auction accounting.
```

**Step 3: Verify**

Run:

```bash
test -f testing/mainnet/auction-order-flow/env.example
test -f testing/mainnet/auction-order-flow/README.md
```

Expected: both commands exit `0`.

**Step 4: Commit**

```bash
git add testing/mainnet/auction-order-flow/env.example testing/mainnet/auction-order-flow/README.md
git commit -m "test: document mainnet auction flow harness"
```

---

### Task 2: Add Shared Deno Mainnet Helpers

**Files:**
- Create: `testing/mainnet/auction-order-flow/src/env.ts`
- Create: `testing/mainnet/auction-order-flow/src/lucid.ts`
- Create: `testing/mainnet/auction-order-flow/deno.json`

**Step 1: Write failing checks**

Run:

```bash
deno check testing/mainnet/auction-order-flow/src/env.ts
```

Expected: FAIL because the file does not exist.

**Step 2: Add `deno.json`**

Create `testing/mainnet/auction-order-flow/deno.json`:

```json
{
  "imports": {
    "@lucid-evolution/lucid": "jsr:@lucid-evolution/lucid",
    "@lucid-evolution/utils": "jsr:@lucid-evolution/utils",
    "@std/dotenv": "jsr:@std/dotenv"
  },
  "tasks": {
    "check": "deno check src/*.ts *.ts"
  }
}
```

**Step 3: Add env loader**

Create `testing/mainnet/auction-order-flow/src/env.ts`:

```ts
import { load } from "@std/dotenv";

export type Asset = {
  policy: string;
  nameHex: string;
};

export type FlowEnv = {
  submit: boolean;
  provider: "blockfrost" | "maestro";
  blockfrostProjectId?: string;
  maestroApiKey?: string;
  walletSeedFile: string;
  auctionValidatorHash: string;
  auctionValidatorRefTxHash: string;
  auctionValidatorRefOutputIndex: bigint;
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

function optional(vars: Record<string, string>, key: string): string | undefined {
  return vars[key] || undefined;
}

function bigintEnv(vars: Record<string, string>, key: string): bigint {
  return BigInt(required(vars, key));
}

function tokenAsset(vars: Record<string, string>, policyKey: string, nameKey: string): Asset {
  const policy = requiredDefined(vars, policyKey);
  const nameHex = requiredDefined(vars, nameKey);
  if (policy === "") throw new Error(`${policyKey} must be non-empty; ADA-side auctions are unsupported`);
  if (!/^[0-9a-fA-F]{56}$/.test(policy)) throw new Error(`${policyKey} must be 28-byte hex`);
  if (!/^[0-9a-fA-F]*$/.test(nameHex) || nameHex.length % 2 !== 0) {
    throw new Error(`${nameKey} must be even-length hex; empty name is allowed`);
  }
  return { policy, nameHex };
}

export async function readFlowEnv(): Promise<FlowEnv> {
  const dotenv = await load({ envPath: "testing/mainnet/auction-order-flow/.env", export: false });
  const vars = { ...dotenv, ...Deno.env.toObject() };
  const auctionBase = tokenAsset(vars, "AUCTION_BASE_POLICY", "AUCTION_BASE_NAME_HEX");
  const auctionQuote = tokenAsset(vars, "AUCTION_QUOTE_POLICY", "AUCTION_QUOTE_NAME_HEX");
  return {
    submit: optional(vars, "SUBMIT") === "1",
    provider: (optional(vars, "PROVIDER") ?? "blockfrost") as "blockfrost" | "maestro",
    blockfrostProjectId: optional(vars, "BLOCKFROST_PROJECT_ID"),
    maestroApiKey: optional(vars, "MAESTRO_API_KEY"),
    walletSeedFile: required(vars, "WALLET_SEED_FILE"),
    auctionValidatorHash: required(vars, "AUCTION_VALIDATOR_HASH"),
    auctionValidatorRefTxHash: required(vars, "AUCTION_VALIDATOR_REF_TX_HASH"),
    auctionValidatorRefOutputIndex: bigintEnv(vars, "AUCTION_VALIDATOR_REF_OUTPUT_INDEX"),
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
  };
}
```

**Step 4: Add Lucid helper**

Create `testing/mainnet/auction-order-flow/src/lucid.ts`:

```ts
import { Blockfrost, Lucid, LucidEvolution } from "@lucid-evolution/lucid";
import { FlowEnv } from "./env.ts";

export async function makeLucid(env: FlowEnv): Promise<LucidEvolution> {
  if (env.provider !== "blockfrost") {
    throw new Error("Maestro provider wiring is intentionally left for a follow-up if needed");
  }
  if (!env.blockfrostProjectId) {
    throw new Error("BLOCKFROST_PROJECT_ID is required for PROVIDER=blockfrost");
  }
  return await Lucid(
    new Blockfrost("https://cardano-mainnet.blockfrost.io/api/v0", env.blockfrostProjectId),
    "Mainnet",
  );
}
```

**Step 5: Verify**

Run:

```bash
cd testing/mainnet/auction-order-flow
deno task check
```

Expected: PASS.

**Step 6: Commit**

```bash
git add testing/mainnet/auction-order-flow/deno.json testing/mainnet/auction-order-flow/src/env.ts testing/mainnet/auction-order-flow/src/lucid.ts
git commit -m "test: add auction flow deno helpers"
```

---

### Task 3: Add Auction Datum And Create-Order Script

**Files:**
- Create: `testing/mainnet/auction-order-flow/src/auction.ts`
- Create: `testing/mainnet/auction-order-flow/create-auction-order.ts`

**Step 1: Write failing check**

Run:

```bash
deno check testing/mainnet/auction-order-flow/create-auction-order.ts
```

Expected: FAIL because the file does not exist.

**Step 2: Add auction datum builder**

Create `testing/mainnet/auction-order-flow/src/auction.ts`:

```ts
import { credentialToAddress } from "@lucid-evolution/utils";
import { Data, paymentCredentialOf } from "@lucid-evolution/lucid";
import { Asset, FlowEnv } from "./env.ts";
import { AuctionAuction } from "../../../../splash-testing-cardano/plutus.ts";

export function unitOf(asset: Asset): string {
  return `${asset.policy}${asset.nameHex}`;
}

export function auctionAddress(validatorHash: string): string {
  return credentialToAddress("Mainnet", { type: "Script", hash: validatorHash });
}

export function buildAuctionDatum(env: FlowEnv, redeemerAddress: string): string {
  const pkh = paymentCredentialOf(redeemerAddress).hash;
  return Data.to({
    base: { policy: env.auctionBase.policy, name: env.auctionBase.nameHex },
    quote: { policy: env.auctionQuote.policy, name: env.auctionQuote.nameHex },
    priceStart: { num: env.auctionPriceStartNum, denom: env.auctionPriceStartDenom },
    startTime: env.auctionStartTimePosix,
    stepLen: env.auctionStepLenSecs,
    steps: env.auctionSteps,
    priceDacayNum: env.auctionPriceDecayNum,
    feePerQuote: { num: env.auctionFeePerQuoteNum, denom: env.auctionFeePerQuoteDenom },
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
    decoded.startTime === env.auctionStartTimePosix &&
    decoded.stepLen === env.auctionStepLenSecs &&
    decoded.steps === env.auctionSteps &&
    decoded.priceDacayNum === env.auctionPriceDecayNum &&
    decoded.feePerQuote.num === env.auctionFeePerQuoteNum &&
    decoded.feePerQuote.denom === env.auctionFeePerQuoteDenom;
}

export function auctionDatumRedeemer(datum: string): string {
  return (decodeAuctionDatum(datum) as DecodedAuctionDatum).redeemer;
}

export function exactAuctionOutput(input: bigint, priceNum: bigint, priceDenom: bigint): bigint {
  return (input * priceNum) / priceDenom;
}

export function activeAuctionSpan(env: FlowEnv, nowPosix: bigint): bigint {
  if (nowPosix < env.auctionStartTimePosix) return 0n;
  const span = (nowPosix - env.auctionStartTimePosix) / env.auctionStepLenSecs;
  if (span >= env.auctionSteps) throw new Error("Auction is expired at current time");
  return span;
}

export function selectedSpanBounds(env: FlowEnv, nowPosix: bigint): { span: bigint; low: bigint; high: bigint; remaining: bigint } {
  const span = activeAuctionSpan(env, nowPosix);
  const low = env.auctionStartTimePosix + env.auctionStepLenSecs * span;
  const high = low + env.auctionStepLenSecs;
  return { span, low, high, remaining: high - nowPosix };
}

export function activeAuctionPrice(env: FlowEnv, nowPosix: bigint): { num: bigint; denom: bigint } {
  const span = activeAuctionSpan(env, nowPosix);
  let num = env.auctionPriceStartNum;
  let denom = env.auctionPriceStartDenom;
  for (let i = 0n; i < span; i++) {
    num *= env.auctionPriceDecayNum;
    denom *= 1000n;
  }
  return { num, denom };
}

export function inversePrice(price: { num: bigint; denom: bigint }): { num: bigint; denom: bigint } {
  return { num: price.denom, denom: price.num };
}

export function jsonStringify(value: unknown): string {
  return JSON.stringify(value, (_key, val) => typeof val === "bigint" ? val.toString() : val, 2);
}

```

This intentionally uses the generated `AuctionAuction.conf` schema from `splash-testing-cardano/plutus.ts`, which encodes the Aiken `orders/auction/Config` constructor fields in order:
`base`, `quote`, `priceStart`, `startTime`, `stepLen`, `steps`, `priceDacayNum`, `feePerQuote`, `redeemer`.

**Step 3: Add create-order script**

Create `testing/mainnet/auction-order-flow/create-auction-order.ts`:

```ts
import { readFlowEnv } from "./src/env.ts";
import { makeLucid } from "./src/lucid.ts";
import { auctionAddress, buildAuctionDatum, jsonStringify, unitOf } from "./src/auction.ts";

const env = await readFlowEnv();
const lucid = await makeLucid(env);
const seed = await Deno.readTextFile(env.walletSeedFile);
lucid.selectWallet.fromSeed(seed.trim());

const walletAddress = await lucid.wallet().address();
const address = auctionAddress(env.auctionValidatorHash);
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
    message: "Dry-run only. Transaction is intentionally unsigned and no CBOR is printed.",
  }));
} else {
  const signed = await tx.sign.withWallet().complete();
  const txHash = await signed.submit();
  const headers = { project_id: env.blockfrostProjectId! };
  const url = `https://cardano-mainnet.blockfrost.io/api/v0/txs/${txHash}/utxos`;
  let outputIndex: number | undefined;
  for (let attempt = 0; attempt < 30 && outputIndex === undefined; attempt++) {
    const res = await fetch(url, { headers });
    if (res.ok) {
      const body = await res.json();
      outputIndex = body.outputs.find((out: { output_index: number; address: string; inline_datum?: string }) =>
        out.address === address && out.inline_datum === datum
      )?.output_index;
    }
    if (outputIndex === undefined) await new Promise((resolve) => setTimeout(resolve, 2000));
  }
  if (outputIndex === undefined) {
    throw new Error(`Submitted auction tx ${txHash}, but could not locate auction output index`);
  }
  console.log(jsonStringify({ submit: true, txHash, outputIndex, address, datum, value }));
}
```

**Step 4: Verify dry-run behavior**

Run:

```bash
SUBMIT=0 deno run --allow-read --allow-env --allow-net testing/mainnet/auction-order-flow/create-auction-order.ts
```

Expected: prints JSON with `submit: false`, address, datum, and value; does not sign, does not print CBOR, and does not submit a transaction.

**Step 5: Add schema verification**

Add a small test script or command that decodes the datum with the same generated schema:

```bash
deno eval --allow-read --allow-env --allow-net '
  import { readFlowEnv } from "./testing/mainnet/auction-order-flow/src/env.ts";
  import { buildAuctionDatum } from "./testing/mainnet/auction-order-flow/src/auction.ts";
  import { AuctionAuction } from "./splash-testing-cardano/plutus.ts";
  import { Data } from "@lucid-evolution/lucid";
  const env = await readFlowEnv();
  const datum = buildAuctionDatum(env, "addr_test1vpqgsp7x9...replace-with-real-mainnet-wallet-in-local-run");
  console.log(Data.from(datum, AuctionAuction.conf));
'
```

Expected: decoded object contains `base`, `quote`, `priceStart`, `startTime`, `stepLen`, `steps`, `priceDacayNum`, `feePerQuote`, and `redeemer`. In implementation, replace the placeholder command with a checked script that uses the configured wallet address.

**Step 6: Commit**

```bash
git add testing/mainnet/auction-order-flow/src/auction.ts testing/mainnet/auction-order-flow/create-auction-order.ts
git commit -m "test: add mainnet auction order creator"
```

---

### Task 4: Add Agent Config Generator

**Files:**
- Create: `testing/mainnet/auction-order-flow/generate-agent-config.sh`
- Create: `testing/mainnet/auction-order-flow/.gitignore`

**Step 1: Write failing check**

Run:

```bash
bash -n testing/mainnet/auction-order-flow/generate-agent-config.sh
```

Expected: FAIL because the file does not exist.

**Step 2: Add `.gitignore`**

Create `testing/mainnet/auction-order-flow/.gitignore`:

```gitignore
.env
.run/
wallet.seed
```

**Step 3: Add generator**

Create `testing/mainnet/auction-order-flow/generate-agent-config.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
FLOW_DIR="$ROOT/testing/mainnet/auction-order-flow"
ENV_FILE="$FLOW_DIR/.env"
RUN_DIR="$FLOW_DIR/.run"
RUN_ID="${RUN_ID:-$(date +%Y%m%d-%H%M%S)}"
export RUN_ID
RUN_STATE_DIR="${RUN_STATE_DIR:-$RUN_DIR/state/$RUN_ID}"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "Missing $ENV_FILE. Copy env.example first." >&2
  exit 1
fi

set -a
CLI_SUBMIT="${SUBMIT:-}"
source "$ENV_FILE"
set +a
if [[ -n "$CLI_SUBMIT" ]]; then
  SUBMIT="$CLI_SUBMIT"
fi

mkdir -p "$RUN_STATE_DIR" "$RUN_DIR/logs"

jq \
  --arg node_socket "$NODE_SOCKET" \
  --arg operator_key "$OPERATOR_KEY_CBOR_HEX" \
  --arg db_path "$RUN_STATE_DIR/chain-sync" \
  --arg health "$AGENT_HEALTH_ADDR" \
  --arg ref_tx "$AUCTION_VALIDATOR_REF_TX_HASH" \
  --argjson ref_ix "$AUCTION_VALIDATOR_REF_OUTPUT_INDEX" \
  --arg hash "$AUCTION_VALIDATOR_HASH" \
  --argjson cost_mem "$AUCTION_VALIDATOR_COST_MEM" \
  --argjson cost_steps "$AUCTION_VALIDATOR_COST_STEPS" \
  --argjson marginal_mem "$AUCTION_VALIDATOR_MARGINAL_COST_MEM" \
  --argjson marginal_steps "$AUCTION_VALIDATOR_MARGINAL_COST_STEPS" \
  --argjson max_cost "$AUCTION_MAX_COST_PER_EX_STEP" \
  --argjson min_out "$AUCTION_MIN_MARGINAL_OUTPUT" \
  '
  .node.path = $node_socket
  | .operatorKey = $operator_key
  | .chainSync.dbPath = $db_path
  | .healthListenAddr = $health
  | .auctionOrders = [{
      validator: {
        hash: $hash,
        referenceUtxo: { txHash: $ref_tx, outputIndex: $ref_ix },
        cost: { mem: $cost_mem, steps: $cost_steps },
        marginalCost: { mem: $marginal_mem, steps: $marginal_steps }
      },
      maxCostPerExStep: $max_cost,
      minMarginalOutput: $min_out
    }]
  ' \
  "$ROOT/${BASE_AGENT_CONFIG:-bloom-cardano-agent/resources/mainnet.config.json.template}" \
  > "$RUN_DIR/agent.mainnet.auction.json"

echo "$RUN_DIR/agent.mainnet.auction.json"
```

**Step 4: Verify**

Run:

```bash
bash -n testing/mainnet/auction-order-flow/generate-agent-config.sh
testing/mainnet/auction-order-flow/generate-agent-config.sh
jq '.auctionOrders[0]' testing/mainnet/auction-order-flow/.run/agent.mainnet.auction.json
```

Expected: script syntax passes, generated config exists, and `auctionOrders[0]` contains the reference UTxO and thresholds.

**Step 5: Commit**

```bash
git add testing/mainnet/auction-order-flow/.gitignore testing/mainnet/auction-order-flow/generate-agent-config.sh
git commit -m "test: generate mainnet auction agent config"
```

---

### Task 3.5: Add Exact-Liquidity Preflight

**Files:**
- Create: `testing/mainnet/auction-order-flow/preflight-exact-liquidity.ts`
- Modify: `testing/mainnet/auction-order-flow/src/env.ts`

**Step 1: Extend env type**

Add these fields to `FlowEnv`:

```ts
targetLiquidityMode: "manual" | "counterOrder";
targetCounterOrderPriceNum?: bigint;
targetCounterOrderPriceDenom?: bigint;
targetCounterOrderTxHash?: string;
targetCounterOrderOutputIndex?: bigint;
```

Populate them from:

```ts
targetLiquidityMode: (optional(vars, "TARGET_LIQUIDITY_MODE") ?? "counterOrder") as "counterOrder",
targetCounterOrderPriceNum: optional(vars, "TARGET_COUNTER_ORDER_PRICE_NUM") ? BigInt(required(vars, "TARGET_COUNTER_ORDER_PRICE_NUM")) : undefined,
targetCounterOrderPriceDenom: optional(vars, "TARGET_COUNTER_ORDER_PRICE_DENOM") ? BigInt(required(vars, "TARGET_COUNTER_ORDER_PRICE_DENOM")) : undefined,
targetCounterOrderTxHash: optional(vars, "TARGET_COUNTER_ORDER_TX_HASH"),
targetCounterOrderOutputIndex: optional(vars, "TARGET_COUNTER_ORDER_OUTPUT_INDEX") ? BigInt(required(vars, "TARGET_COUNTER_ORDER_OUTPUT_INDEX")) : undefined,
```

**Step 2: Add preflight script**

Create `testing/mainnet/auction-order-flow/preflight-exact-liquidity.ts`:

```ts
import { readFlowEnv } from "./src/env.ts";
import { activeAuctionPrice, exactAuctionOutput, inversePrice, jsonStringify, selectedSpanBounds } from "./src/auction.ts";
import { makeLucid } from "./src/lucid.ts";
import { Data } from "@lucid-evolution/lucid";
import { LimitOrderLimitOrder } from "../../../splash-testing-cardano/plutus.ts";

const env = await readFlowEnv();
if (env.targetLiquidityMode !== "counterOrder") {
  throw new Error("Only TARGET_LIQUIDITY_MODE=counterOrder is supported by this mainnet harness");
}
const now = BigInt(Math.floor(Date.now() / 1000));
const span = selectedSpanBounds(env, now);
if (span.remaining < env.minSpanRemainingSecs) {
  throw new Error(`Only ${span.remaining}s remain in auction span ${span.span}; need at least ${env.minSpanRemainingSecs}s`);
}
const activePrice = activeAuctionPrice(env, now);
const counterPrice = inversePrice(activePrice);
const exact = exactAuctionOutput(
  env.auctionBaseAmount,
  activePrice.num,
  activePrice.denom,
);

if (env.targetCounterOrderPriceNum === undefined || env.targetCounterOrderPriceDenom === undefined) {
  throw new Error("TARGET_COUNTER_ORDER_PRICE_NUM/DENOM are required for TARGET_LIQUIDITY_MODE=counterOrder");
}
if (env.targetCounterOrderPriceNum !== counterPrice.num ||
    env.targetCounterOrderPriceDenom !== counterPrice.denom) {
  throw new Error(`Counter-order price ${env.targetCounterOrderPriceNum}/${env.targetCounterOrderPriceDenom} must equal inverse active auction price ${counterPrice.num}/${counterPrice.denom}`);
}

if (env.targetCounterOrderTxHash === undefined || env.targetCounterOrderOutputIndex === undefined) {
  throw new Error("TARGET_COUNTER_ORDER_TX_HASH and TARGET_COUNTER_ORDER_OUTPUT_INDEX are required before preflight");
}

const lucid = await makeLucid(env);
const [counter] = await lucid.utxosByOutRef([{
  txHash: env.targetCounterOrderTxHash,
  outputIndex: Number(env.targetCounterOrderOutputIndex),
}]);
if (!counter?.datum) {
  throw new Error(`Counter-order UTxO ${env.targetCounterOrderTxHash}#${env.targetCounterOrderOutputIndex} is missing or has no inline datum`);
}
const decoded = Data.from(counter.datum, LimitOrderLimitOrder.conf) as {
  input: { policy: string; name: string };
  output: { policy: string; name: string };
  tradableInput: bigint;
  basePrice: { num: bigint; denom: bigint };
};
if (decoded.input.policy !== env.auctionQuote.policy || decoded.input.name !== env.auctionQuote.nameHex) {
  throw new Error("Counter order input must equal auction quote asset");
}
if (decoded.output.policy !== env.auctionBase.policy || decoded.output.name !== env.auctionBase.nameHex) {
  throw new Error("Counter order output must equal auction base asset");
}
if (decoded.basePrice.num !== counterPrice.num || decoded.basePrice.denom !== counterPrice.denom) {
  throw new Error("Counter order datum price must equal inverse active auction price");
}
if (decoded.tradableInput < exact) {
  throw new Error(`Counter order tradable input ${decoded.tradableInput} is less than auction exact output ${exact}`);
}

console.log(jsonStringify({
  status: "ok",
  activePrice,
  counterPrice,
  span,
  exactOutput: exact.toString(),
  counterOrder: `${env.targetCounterOrderTxHash}#${env.targetCounterOrderOutputIndex}`,
}));
```

This task is deliberately conservative. It does not use pool liquidity, because the live agent could otherwise choose a better-than-exact pool output. Mainnet live runs must use a paired counter-order whose limit-order datum price is the inverse of the active auction price. In observe mode the implementation must fetch and decode the referenced counter-order UTxO before returning `ok`; in submit mode the orchestrator creates that counter-order first and passes its real out-ref into this preflight.

**Step 3: Verify**

Run:

```bash
deno check testing/mainnet/auction-order-flow/preflight-exact-liquidity.ts
deno run --allow-read --allow-env --allow-net testing/mainnet/auction-order-flow/preflight-exact-liquidity.ts
```

Expected: PASS when target exact output/price matches; FAIL before any submission when it does not.

**Step 4: Commit**

```bash
git add testing/mainnet/auction-order-flow/src/env.ts testing/mainnet/auction-order-flow/preflight-exact-liquidity.ts
git commit -m "test: add auction exact-liquidity preflight"
```

---

### Task 3.6: Add Paired Counter Limit Order Creator

**Files:**
- Create: `testing/mainnet/auction-order-flow/create-counter-limit-order.ts`
- Modify: `testing/mainnet/auction-order-flow/src/env.ts`

**Step 1: Add counter-order env fields**

Add `createCounterOrder`, `counterOrderLovelaceBudget`, and `counterOrderFee` to `FlowEnv`, populated from `CREATE_COUNTER_ORDER`, `COUNTER_ORDER_LOVELACE_BUDGET`, and `COUNTER_ORDER_FEE`.

**Step 2: Add creator script**

Create `testing/mainnet/auction-order-flow/create-counter-limit-order.ts` by adapting `splash-testing-cardano/src/limitOrder.ts`, but with these constraints:

```ts
// Counter order must be opposite side of auction:
// input = auction quote, output = auction base.
// basePrice must equal inversePrice(activeAuctionPrice(env, now)).
// tradable input must be exactAuctionOutput(auctionBaseAmount, activePrice.num, activePrice.denom).
// selectedSpanBounds(env, now).remaining must be >= env.minSpanRemainingSecs.
// Dry-run must not sign and must not print signed CBOR.
// Submit mode must locate and print the real outputIndex by matching address + inline datum.
// Both dry-run and submit JSON output must use jsonStringify and include
// counterPriceNum/counterPriceDenom so run-flow.sh can export them into preflight.
```

Use the generated `LimitOrderLimitOrder.conf` schema from `splash-testing-cardano/plutus.ts`, the mainnet limit-order validator from `bloom-cardano-agent/resources/mainnet.deployment.json`, and the same beacon derivation approach already used by `splash-testing-cardano/src/limitOrder.ts`.

**Step 3: Verify real counter-order preflight**

After submission, fetch the created counter-order UTxO by its printed out-ref and decode its datum. Assert:

- input asset equals auction quote
- output asset equals auction base
- limit-order datum basePrice equals the inverse active auction price
- tradable input is enough to pay the auction exact output
- order is token-token and not ADA-side

**Step 4: Verify**

Run:

```bash
deno check testing/mainnet/auction-order-flow/create-counter-limit-order.ts
SUBMIT=0 deno run --allow-read --allow-env --allow-net testing/mainnet/auction-order-flow/create-counter-limit-order.ts
```

Expected: PASS; dry-run prints unsigned summary only.

**Step 5: Commit**

```bash
git add testing/mainnet/auction-order-flow/src/env.ts testing/mainnet/auction-order-flow/create-counter-limit-order.ts
git commit -m "test: create paired auction counter order"
```

---

### Task 5: Add Agent Runner Script

**Files:**
- Create: `testing/mainnet/auction-order-flow/run-agent.sh`

**Step 1: Write failing check**

Run:

```bash
bash -n testing/mainnet/auction-order-flow/run-agent.sh
```

Expected: FAIL because the file does not exist.

**Step 2: Add runner**

Create `testing/mainnet/auction-order-flow/run-agent.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
FLOW_DIR="$ROOT/testing/mainnet/auction-order-flow"
ENV_FILE="$FLOW_DIR/.env"
RUN_DIR="$FLOW_DIR/.run"

set -a
CLI_SUBMIT="${SUBMIT:-}"
source "$ENV_FILE"
set +a
if [[ -n "$CLI_SUBMIT" ]]; then
  SUBMIT="$CLI_SUBMIT"
fi

CONFIG_PATH="$("$FLOW_DIR/generate-agent-config.sh")"
mkdir -p "$RUN_DIR/logs"
export AGENT_LOG_FILE="${AGENT_LOG_FILE:-$RUN_DIR/logs/agent.$(date +%Y%m%d-%H%M%S).log}"
export RUN_ID="${RUN_ID:-$(date +%Y%m%d-%H%M%S)}"
LOG4RS_RUNTIME_CONFIG="$RUN_DIR/log4rs.$RUN_ID.yaml"

cat > "$LOG4RS_RUNTIME_CONFIG" <<YAML
refresh_rate: 30 seconds
appenders:
  stdout:
    kind: console
    encoder:
      pattern: "{d(%Y-%m-%d %H:%M:%S)} {l:5.5} {t} {m}{n}"
root:
  level: trace
  appenders:
    - stdout
loggers:
  chain_sync:
    level: trace
  agent_main:
    level: trace
YAML

if [[ "${BUILD_AGENT:-1}" == "1" ]]; then
  cargo build --package bloom-cardano-agent
fi

"${AGENT_BIN:-target/debug/bloom-cardano-agent}" \
  --config-path "$CONFIG_PATH" \
  --deployment-path "$ROOT/${DEPLOYMENT_CONFIG:-bloom-cardano-agent/resources/mainnet.deployment.json}" \
  --validation-rules-path "$ROOT/${VALIDATION_RULES:-bloom-cardano-agent/resources/validation-rules.json.template}" \
  --log4rs-path "${LOG4RS_CONFIG:-$LOG4RS_RUNTIME_CONFIG}" \
  2>&1 | tee "$AGENT_LOG_FILE"
```

**Step 3: Verify syntax**

Run:

```bash
bash -n testing/mainnet/auction-order-flow/run-agent.sh
```

Expected: PASS.

**Step 4: Commit**

```bash
git add testing/mainnet/auction-order-flow/run-agent.sh
git commit -m "test: add mainnet auction agent runner"
```

---

### Task 6: Add Auction Flow Verifier

**Files:**
- Create: `testing/mainnet/auction-order-flow/verify-auction-flow.ts`

**Step 1: Write failing check**

Run:

```bash
deno check testing/mainnet/auction-order-flow/verify-auction-flow.ts
```

Expected: FAIL because the file does not exist.

**Step 2: Add verifier**

Create `testing/mainnet/auction-order-flow/verify-auction-flow.ts`:

```ts
import { readFlowEnv } from "./src/env.ts";
import { makeLucid } from "./src/lucid.ts";
import { paymentCredentialOf } from "@lucid-evolution/lucid";
import { activeAuctionPrice, auctionAddress, auctionDatumRedeemer, selectedSpanBounds } from "./src/auction.ts";

const env = await readFlowEnv();
const lucid = await makeLucid(env);

const txHash = Deno.env.get("AUCTION_TX_HASH") ?? Deno.env.get("EXISTING_AUCTION_TX_HASH");
const outputIndex = Deno.env.get("AUCTION_OUTPUT_INDEX") ?? Deno.env.get("EXISTING_AUCTION_OUTPUT_INDEX");
if (!txHash || !outputIndex) {
  throw new Error("AUCTION_TX_HASH/AUCTION_OUTPUT_INDEX or EXISTING_AUCTION_* is required");
}

const agentLog = Deno.env.get("AGENT_LOG_FILE");
if (!agentLog) throw new Error("AGENT_LOG_FILE is required to correlate spend with bloom-cardano-agent");

const timeoutSecs = Number(Deno.env.get("VERIFY_TIMEOUT_SECS") ?? "600");
const pollSecs = Number(Deno.env.get("VERIFY_POLL_SECS") ?? "10");
const deadline = Date.now() + timeoutSecs * 1000;
const startedAt = Date.now();

const initial = await lucid.utxosByOutRef([{ txHash, outputIndex: Number(outputIndex) }]);
if (initial.length !== 1) {
  throw new Error(`Auction UTxO ${txHash}#${outputIndex} is not present before agent verification starts`);
}
const initialDatum = initial[0].datum;
if (!initialDatum) {
  throw new Error(`Auction UTxO ${txHash}#${outputIndex} has no inline datum`);
}
const initialRedeemer = auctionDatumRedeemer(initialDatum);
const auctionAddr = auctionAddress(env.auctionValidatorHash);
const baseUnit = `${env.auctionBase.policy}${env.auctionBase.nameHex}`;
const quoteUnit = `${env.auctionQuote.policy}${env.auctionQuote.nameHex}`;

function amountOf(amounts: Array<{ unit: string; quantity: string }>, unit: string): bigint {
  return BigInt(amounts.find((a) => a.unit === unit)?.quantity ?? "0");
}

function agentSawAuctionExecution(): boolean {
  try {
    const log = Deno.readTextFileSync(agentLog);
    return log.includes("AuctionOrder::exec(");
  } catch (_) {
    return false;
  }
}

function paymentCredentialHash(address: string): string | undefined {
  try {
    return paymentCredentialOf(address).hash;
  } catch (_) {
    return undefined;
  }
}

async function findSpendingTx(): Promise<string | undefined> {
  const headers = { project_id: env.blockfrostProjectId! };
  const txsUrl = `https://cardano-mainnet.blockfrost.io/api/v0/addresses/${auctionAddr}/transactions?order=desc&count=100`;
  const txs = await fetch(txsUrl, { headers }).then((r) => r.ok ? r.json() : []);
  for (const item of txs as Array<{ tx_hash: string }>) {
    const utxosUrl = `https://cardano-mainnet.blockfrost.io/api/v0/txs/${item.tx_hash}/utxos`;
    const body = await fetch(utxosUrl, { headers }).then((r) => r.ok ? r.json() : undefined);
    if (body?.inputs?.some((input: { tx_hash: string; output_index: number }) =>
      input.tx_hash === txHash && input.output_index === Number(outputIndex)
    )) {
      return item.tx_hash;
    }
  }
  return undefined;
}

async function verifyAuctionSpend(spendingTx: string): Promise<void> {
  const headers = { project_id: env.blockfrostProjectId! };
  const txInfo = await fetch(`https://cardano-mainnet.blockfrost.io/api/v0/txs/${spendingTx}`, { headers })
    .then((r) => r.ok ? r.json() : undefined);
  if (!txInfo?.invalid_before || !txInfo?.invalid_hereafter) {
    throw new Error(`Spending tx ${spendingTx} has no validity interval`);
  }
  if (!txInfo?.block_time) {
    throw new Error(`Spending tx ${spendingTx} has no block_time`);
  }
  const txTime = BigInt(txInfo.block_time);
  const span = selectedSpanBounds(env, txTime);
  const activePrice = activeAuctionPrice(env, txTime);
  if (txTime < span.low || txTime >= span.high) {
    throw new Error(`Spending tx ${spendingTx} block_time ${txTime} is outside selected auction span ${span.low}-${span.high}`);
  }
  const body = await fetch(`https://cardano-mainnet.blockfrost.io/api/v0/txs/${spendingTx}/utxos`, { headers })
    .then((r) => r.ok ? r.json() : undefined);
  if (!body) throw new Error(`Cannot fetch spending tx ${spendingTx}`);
  const inputs = body.inputs as Array<{ tx_hash: string; output_index: number; amount: Array<{ unit: string; quantity: string }> }>;
  const outputs = body.outputs as Array<{ address: string; amount: Array<{ unit: string; quantity: string }>; inline_datum?: string }>;
  const selfInput = inputs.find((input) => input.tx_hash === txHash && input.output_index === Number(outputIndex));
  if (!selfInput) throw new Error(`Spending tx ${spendingTx} does not consume auction input ${txHash}#${outputIndex}`);
  const base0 = amountOf(selfInput.amount, baseUnit);
  const quote0 = amountOf(selfInput.amount, quoteUnit);

  const candidates = outputs.map((out) => {
    const isScriptSuccessor = out.address === auctionAddr && out.inline_datum === initialDatum;
    const isTerminal = out.address !== auctionAddr && paymentCredentialHash(out.address) === initialRedeemer;
    return { out, isScriptSuccessor, isTerminal };
  }).filter(({ out, isScriptSuccessor, isTerminal }) => {
    if (isScriptSuccessor) return true;
    return isTerminal && amountOf(out.amount, baseUnit) === 0n && amountOf(out.amount, quoteUnit) > quote0;
  });

  for (const candidate of candidates) {
    const base1 = amountOf(candidate.out.amount, baseUnit);
    const quote1 = amountOf(candidate.out.amount, quoteUnit);
    const baseSubtracted = base0 - base1;
    const quoteAdded = quote1 - quote0;
    if (baseSubtracted <= 0n || quoteAdded <= 0n) continue;
    if (quoteAdded * activePrice.denom !== baseSubtracted * activePrice.num) continue;
    if (candidate.isScriptSuccessor && base1 === 0n) continue;
    if (candidate.isTerminal && base1 !== 0n) continue;
    return;
  }

  throw new Error(`Spending tx ${spendingTx} has no successor/terminal output matching this auction and exact active price`);
}

while (Date.now() < deadline) {
  const utxos = await lucid.utxosByOutRef([{ txHash, outputIndex: Number(outputIndex) }]);
  if (utxos.length === 0) {
    if (!agentSawAuctionExecution()) {
      throw new Error(`Auction UTxO ${txHash}#${outputIndex} was spent, but agent log has no AuctionOrder::exec after verifier start ${startedAt}`);
    }
    const spendingTx = await findSpendingTx();
    if (!spendingTx) throw new Error(`Could not locate spending transaction for ${txHash}#${outputIndex}`);
    await verifyAuctionSpend(spendingTx);
    console.log(JSON.stringify({ status: "spent_by_agent_flow", txHash, outputIndex, spendingTx, agentLog }, null, 2));
    Deno.exit(0);
  }
  console.log(JSON.stringify({ status: "still_unspent", txHash, outputIndex }, null, 2));
  await new Promise((resolve) => setTimeout(resolve, pollSecs * 1000));
}

throw new Error(`Auction UTxO ${txHash}#${outputIndex} was not spent by agent flow before timeout`);
```

The implementation must complete `verifyAuctionSpend` before acceptance. Minimum acceptance criterion: the UTxO was present at verifier start, disappears during the agent run, the spending tx actually consumes that out-ref, the same run log contains `AuctionOrder::exec(`, and the spending tx outputs satisfy auction successor/terminal exact-price semantics.

**Step 3: Verify**

Run:

```bash
deno check testing/mainnet/auction-order-flow/verify-auction-flow.ts
```

Expected: PASS.

**Step 4: Commit**

```bash
git add testing/mainnet/auction-order-flow/verify-auction-flow.ts
git commit -m "test: verify mainnet auction flow"
```

---

### Task 7: Add End-To-End Orchestrator

**Files:**
- Create: `testing/mainnet/auction-order-flow/run-flow.sh`

**Step 1: Write failing check**

Run:

```bash
bash -n testing/mainnet/auction-order-flow/run-flow.sh
```

Expected: FAIL because the file does not exist.

**Step 2: Add orchestrator**

Create `testing/mainnet/auction-order-flow/run-flow.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-observe}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
FLOW_DIR="$ROOT/testing/mainnet/auction-order-flow"
ENV_FILE="$FLOW_DIR/.env"
RUN_DIR="$FLOW_DIR/.run"

if [[ "$MODE" != "observe" && "$MODE" != "submit" ]]; then
  echo "Usage: $0 observe|submit" >&2
  exit 1
fi

if [[ ! -f "$ENV_FILE" ]]; then
  echo "Missing $ENV_FILE. Copy env.example first." >&2
  exit 1
fi

set -a
CLI_SUBMIT="${SUBMIT:-}"
source "$ENV_FILE"
set +a
if [[ -n "$CLI_SUBMIT" ]]; then
  SUBMIT="$CLI_SUBMIT"
fi
RUN_ID="${RUN_ID:-$(date +%Y%m%d-%H%M%S)}"
export RUN_ID

mkdir -p "$RUN_DIR/logs"
export RUN_STATE_DIR="$RUN_DIR/state/$RUN_ID"
export AGENT_LOG_FILE="$RUN_DIR/logs/agent.$RUN_ID.log"

wait_for_agent_ready() {
  local deadline=$((SECONDS + ${AGENT_READY_TIMEOUT_SECS:-180}))
  local health_url="http://${AGENT_HEALTH_ADDR:-127.0.0.1:9024}/health"
  while (( SECONDS < deadline )); do
    if ! kill -0 "$AGENT_PID" 2>/dev/null; then
      echo "agent exited before readiness" >&2
      tail -200 "$AGENT_LOG_FILE" >&2 || true
      exit 1
    fi
    if curl -sf "$health_url" >/dev/null 2>&1 &&
       grep -q "Health API listening" "$AGENT_LOG_FILE" 2>/dev/null &&
       grep -q "Tip reached, waiting for new blocks" "$AGENT_LOG_FILE" 2>/dev/null; then
      return 0
    fi
    sleep 2
  done
  echo "agent did not become ready before submitting mainnet orders" >&2
  tail -200 "$AGENT_LOG_FILE" >&2 || true
  exit 1
}

"$FLOW_DIR/run-agent.sh" &
AGENT_PID="$!"
trap 'kill "$AGENT_PID" 2>/dev/null || true' EXIT
wait_for_agent_ready

if [[ "$MODE" == "submit" ]]; then
  if [[ "${SUBMIT:-0}" != "1" ]]; then
    echo "submit mode requires SUBMIT=1" >&2
    exit 1
  fi
  deno run --allow-read --allow-env --allow-net "$FLOW_DIR/create-counter-limit-order.ts" \
    | tee "$RUN_DIR/logs/create-counter-limit-order.json"
  export TARGET_COUNTER_ORDER_TX_HASH="$(jq -r 'select(.txHash) | .txHash' "$RUN_DIR/logs/create-counter-limit-order.json" | tail -1)"
  export TARGET_COUNTER_ORDER_OUTPUT_INDEX="$(jq -r 'select(.outputIndex != null) | .outputIndex' "$RUN_DIR/logs/create-counter-limit-order.json" | tail -1)"
  export TARGET_COUNTER_ORDER_PRICE_NUM="$(jq -r 'select(.counterPriceNum != null) | .counterPriceNum' "$RUN_DIR/logs/create-counter-limit-order.json" | tail -1)"
  export TARGET_COUNTER_ORDER_PRICE_DENOM="$(jq -r 'select(.counterPriceDenom != null) | .counterPriceDenom' "$RUN_DIR/logs/create-counter-limit-order.json" | tail -1)"
  deno run --allow-read --allow-env --allow-net "$FLOW_DIR/preflight-exact-liquidity.ts"
  deno run --allow-read --allow-env --allow-net "$FLOW_DIR/create-auction-order.ts" \
    | tee "$RUN_DIR/logs/create-auction-order.json"
  export AUCTION_TX_HASH="$(jq -r 'select(.txHash) | .txHash' "$RUN_DIR/logs/create-auction-order.json" | tail -1)"
  export AUCTION_OUTPUT_INDEX="$(jq -r 'select(.outputIndex != null) | .outputIndex' "$RUN_DIR/logs/create-auction-order.json" | tail -1)"
else
  export AUCTION_TX_HASH="${EXISTING_AUCTION_TX_HASH:?EXISTING_AUCTION_TX_HASH is required}"
  export AUCTION_OUTPUT_INDEX="${EXISTING_AUCTION_OUTPUT_INDEX:?EXISTING_AUCTION_OUTPUT_INDEX is required}"
  deno run --allow-read --allow-env --allow-net "$FLOW_DIR/preflight-exact-liquidity.ts"
fi

deno run --allow-read --allow-env --allow-net "$FLOW_DIR/verify-auction-flow.ts"
```

Starting the agent before creating the paired orders is intentional: the isolated `.run/state` begins at the current mainnet tip and observes only newly submitted counter-order/auction transactions, preventing stale pre-existing pools from being selected in the local book. The readiness gate is required before any submit-mode transaction: it must confirm the health endpoint responds and the chain-sync log has reached the tip marker. If the repository log configuration does not emit the `chain_sync` trace marker, the implementation must add a harness-local log4rs config that enables that target for this script.

**Step 3: Verify syntax**

Run:

```bash
bash -n testing/mainnet/auction-order-flow/run-flow.sh
```

Expected: PASS.

**Step 4: Commit**

```bash
git add testing/mainnet/auction-order-flow/run-flow.sh
git commit -m "test: orchestrate mainnet auction flow"
```

---

### Task 8: Add Dry-Run And CI Verification Commands

**Files:**
- Modify: `testing/mainnet/auction-order-flow/README.md`

**Step 1: Add verification checklist**

Append to README:

```markdown
## Local Verification

Run these before any mainnet submission:

```bash
bash -n testing/mainnet/auction-order-flow/*.sh
cd testing/mainnet/auction-order-flow && deno task check
cargo check --package bloom-cardano-agent
cargo test --package bloom-offchain-cardano orders::auction::tests
git diff --check
```

For live mainnet:

```bash
SUBMIT=1 testing/mainnet/auction-order-flow/run-flow.sh submit
```

Success criteria:

- creator prints submitted auction tx hash
- agent starts from isolated `.run/state`
- verifier reports `status: "spent_by_agent_flow"`
- agent log contains an auction order execution transition
```

**Step 2: Run verification**

Run:

```bash
bash -n testing/mainnet/auction-order-flow/*.sh
cd testing/mainnet/auction-order-flow && deno task check
cargo check --package bloom-cardano-agent
cargo test --package bloom-offchain-cardano orders::auction::tests
git diff --check
```

Expected: all commands pass, except `cargo fmt --check` is intentionally not part of this checklist because the repository currently has unrelated formatting drift.

**Step 3: Commit**

```bash
git add testing/mainnet/auction-order-flow/README.md
git commit -m "test: document auction flow verification"
```

---

## Final Review

After all tasks are implemented:

1. Run:

```bash
git diff --check
cargo check --package bloom-cardano-agent
cargo test --package bloom-offchain-cardano orders::auction::tests
bash -n testing/mainnet/auction-order-flow/*.sh
cd testing/mainnet/auction-order-flow && deno task check
```

2. Request reviewer sub-agent review with this prompt:

```text
Review mainnet auction order flow test scripts under testing/mainnet/auction-order-flow.
Focus on mainnet safety, no accidental submission without SUBMIT=1, correct auction datum/schema, correct token-token constraints, isolated agent state, config generation, verification reliability, and whether the scripts actually exercise bloom-cardano-agent auction execution.
Return OK only if no blockers.
```

3. Fix all reviewer blockers and repeat review until it returns OK.
