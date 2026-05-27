import { paymentCredentialOf } from "@lucid-evolution/lucid";
import {
  activeAuctionPrice,
  auctionAddress,
  auctionDatumRedeemer,
  selectedSpanBounds,
} from "./src/auction.ts";
import { blockfrostBaseUrl, readFlowEnv } from "./src/env.ts";
import { makeLucid } from "./src/lucid.ts";

const env = await readFlowEnv();
const lucid = await makeLucid(env);

const txHash = Deno.env.get("AUCTION_TX_HASH") ??
  Deno.env.get("EXISTING_AUCTION_TX_HASH");
const outputIndex = Deno.env.get("AUCTION_OUTPUT_INDEX") ??
  Deno.env.get("EXISTING_AUCTION_OUTPUT_INDEX");
if (!txHash || !outputIndex) {
  throw new Error(
    "AUCTION_TX_HASH/AUCTION_OUTPUT_INDEX or EXISTING_AUCTION_* is required",
  );
}
const auctionTxHash = txHash;
const auctionOutputIndex = Number(outputIndex);

const agentLogEnv = Deno.env.get("AGENT_LOG_FILE");
if (!agentLogEnv) {
  throw new Error(
    "AGENT_LOG_FILE is required to correlate spend with bloom-cardano-agent",
  );
}
const agentLog = agentLogEnv;

const timeoutSecs = Number(Deno.env.get("VERIFY_TIMEOUT_SECS") ?? "600");
const pollSecs = Number(Deno.env.get("VERIFY_POLL_SECS") ?? "10");
const deadline = Date.now() + timeoutSecs * 1000;
const startedAt = Date.now();

const anchoredDatum = Deno.env.get("AUCTION_INITIAL_DATUM");
const initial = env.provider === "koios" ? [] : await lucid.utxosByOutRef([{
  txHash: auctionTxHash,
  outputIndex: auctionOutputIndex,
}]);
if (initial.length !== 1 && !anchoredDatum) {
  throw new Error(
    `Auction UTxO ${txHash}#${outputIndex} is not present before agent verification starts`,
  );
}
const initialDatum = anchoredDatum ?? initial[0]?.datum;
if (!initialDatum) {
  throw new Error(`Auction UTxO ${txHash}#${outputIndex} has no inline datum`);
}
const initialRedeemer = auctionDatumRedeemer(initialDatum);
const auctionAddr = auctionAddress(env);
const baseUnit = `${env.auctionBase.policy}${env.auctionBase.nameHex}`;
const quoteUnit = `${env.auctionQuote.policy}${env.auctionQuote.nameHex}`;

function amountOf(
  amounts: Array<{ unit: string; quantity: string }>,
  unit: string,
): bigint {
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

async function blockfrostJson<T>(path: string): Promise<T | undefined> {
  if (env.provider !== "blockfrost") return undefined;
  const headers = { project_id: env.blockfrostProjectId! };
  const res = await fetch(
    `${blockfrostBaseUrl(env)}${path}`,
    { headers },
  );
  return res.ok ? await res.json() as T : undefined;
}

async function koiosJson<T>(path: string, body: unknown): Promise<T | undefined> {
  if (env.provider !== "koios") return undefined;
  const res = await fetch(`https://preprod.koios.rest/api/v1${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  return res.ok ? await res.json() as T : undefined;
}

type KoiosAsset = {
  policy_id: string;
  asset_name: string;
  quantity: string;
};

type KoiosTxOutput = {
  value: string;
  tx_hash?: string;
  tx_index?: number;
  asset_list?: KoiosAsset[] | string;
  inline_datum?: { bytes?: string | null } | null;
  payment_addr?: { bech32?: string } | null;
};

type KoiosTxInfo = {
  tx_hash: string;
  block_height: number;
  tx_timestamp: number;
  invalid_before?: string | null;
  invalid_after?: string | null;
  inputs: KoiosTxOutput[];
  outputs: KoiosTxOutput[];
};

async function koiosTxInfo(hash: string): Promise<KoiosTxInfo | undefined> {
  const rows = await koiosJson<KoiosTxInfo[]>("/tx_info", {
    _tx_hashes: [hash],
  });
  return rows?.[0];
}

async function koiosUtxoIsPresent(): Promise<boolean> {
  const rows = await koiosJson<Array<{ tx_hash: string; tx_index: number }>>(
    "/utxo_info",
    {
      _utxo_refs: [`${auctionTxHash}#${auctionOutputIndex}`],
    },
  );
  return (rows?.length ?? 0) > 0;
}

function koiosAmountOf(output: KoiosTxOutput, unit: string): bigint {
  if (unit === "lovelace") return BigInt(output.value);
  const assets = Array.isArray(output.asset_list) ? output.asset_list : [];
  const policy = unit.slice(0, 56);
  const name = unit.slice(56);
  return BigInt(
    assets.find((asset) =>
      asset.policy_id === policy && asset.asset_name === name
    )?.quantity ?? "0",
  );
}

async function findSpendingTx(): Promise<string | undefined> {
  if (env.provider === "koios") {
    const creation = await koiosTxInfo(auctionTxHash);
    const txs = await koiosJson<Array<{ tx_hash: string }>>("/address_txs", {
      _addresses: [auctionAddr],
      _after_block_height: Math.max(0, (creation?.block_height ?? 0) - 1),
    }) ?? [];
    for (const item of txs.reverse()) {
      const body = await koiosTxInfo(item.tx_hash);
      if (
        body?.inputs?.some((input) =>
          input.tx_hash === auctionTxHash &&
          input.tx_index === auctionOutputIndex
        )
      ) {
        return item.tx_hash;
      }
    }
    return undefined;
  }

  const txs = await blockfrostJson<Array<{ tx_hash: string }>>(
    `/addresses/${auctionAddr}/transactions?order=desc&count=100`,
  ) ?? [];
  for (const item of txs) {
    const body = await blockfrostJson<{
      inputs?: Array<{ tx_hash: string; output_index: number }>;
    }>(`/txs/${item.tx_hash}/utxos`);
    if (
      body?.inputs?.some((input) =>
        input.tx_hash === auctionTxHash &&
        input.output_index === auctionOutputIndex
      )
    ) {
      return item.tx_hash;
    }
  }
  return undefined;
}

async function verifyAuctionSpend(spendingTx: string): Promise<void> {
  if (env.provider === "koios") {
    const txInfo = await koiosTxInfo(spendingTx);
    if (!txInfo?.invalid_before || !txInfo?.invalid_after) {
      throw new Error(`Spending tx ${spendingTx} has no validity interval`);
    }
    if (!txInfo.tx_timestamp) {
      throw new Error(`Spending tx ${spendingTx} has no tx_timestamp`);
    }
    const txTime = BigInt(txInfo.tx_timestamp);
    const span = selectedSpanBounds(env, txTime);
    const activePrice = activeAuctionPrice(env, txTime);
    if (txTime < span.low || txTime >= span.high) {
      throw new Error(
        `Spending tx ${spendingTx} block_time ${txTime} is outside selected auction span ${span.low}-${span.high}`,
      );
    }

    const selfInput = txInfo.inputs.find((input) =>
      input.tx_hash === auctionTxHash && input.tx_index === auctionOutputIndex
    );
    if (!selfInput) {
      throw new Error(
        `Spending tx ${spendingTx} does not consume auction input ${txHash}#${outputIndex}`,
      );
    }
    const base0 = koiosAmountOf(selfInput, baseUnit);
    const quote0 = koiosAmountOf(selfInput, quoteUnit);

    const candidates = txInfo.outputs.map((out) => {
      const address = out.payment_addr?.bech32 ?? "";
      const isScriptSuccessor = address === auctionAddr &&
        out.inline_datum?.bytes === initialDatum;
      const isTerminal = address !== auctionAddr &&
        paymentCredentialHash(address) === initialRedeemer;
      return { out, isScriptSuccessor, isTerminal };
    }).filter(({ out, isScriptSuccessor, isTerminal }) => {
      if (isScriptSuccessor) return true;
      return isTerminal && koiosAmountOf(out, baseUnit) === 0n &&
        koiosAmountOf(out, quoteUnit) > quote0;
    });

    for (const candidate of candidates) {
      const base1 = koiosAmountOf(candidate.out, baseUnit);
      const quote1 = koiosAmountOf(candidate.out, quoteUnit);
      const baseSubtracted = base0 - base1;
      const quoteAdded = quote1 - quote0;
      if (baseSubtracted <= 0n || quoteAdded <= 0n) continue;
      if (quoteAdded * activePrice.denom !== baseSubtracted * activePrice.num) {
        continue;
      }
      if (candidate.isScriptSuccessor && base1 === 0n) continue;
      if (candidate.isTerminal && base1 !== 0n) continue;
      return;
    }

    throw new Error(
      `Spending tx ${spendingTx} has no successor/terminal output matching this auction and exact active price`,
    );
  }

  const txInfo = await blockfrostJson<{
    invalid_before?: string;
    invalid_hereafter?: string;
    block_time?: number;
  }>(`/txs/${spendingTx}`);
  if (!txInfo?.invalid_before || !txInfo?.invalid_hereafter) {
    throw new Error(`Spending tx ${spendingTx} has no validity interval`);
  }
  if (!txInfo.block_time) {
    throw new Error(`Spending tx ${spendingTx} has no block_time`);
  }
  const txTime = BigInt(txInfo.block_time);
  const span = selectedSpanBounds(env, txTime);
  const activePrice = activeAuctionPrice(env, txTime);
  if (txTime < span.low || txTime >= span.high) {
    throw new Error(
      `Spending tx ${spendingTx} block_time ${txTime} is outside selected auction span ${span.low}-${span.high}`,
    );
  }

  const body = await blockfrostJson<{
    inputs: Array<
      {
        tx_hash: string;
        output_index: number;
        amount: Array<{ unit: string; quantity: string }>;
      }
    >;
    outputs: Array<
      {
        address: string;
        amount: Array<{ unit: string; quantity: string }>;
        inline_datum?: string;
      }
    >;
  }>(`/txs/${spendingTx}/utxos`);
  if (!body) throw new Error(`Cannot fetch spending tx ${spendingTx}`);
  const selfInput = body.inputs.find((input) =>
    input.tx_hash === auctionTxHash && input.output_index === auctionOutputIndex
  );
  if (!selfInput) {
    throw new Error(
      `Spending tx ${spendingTx} does not consume auction input ${txHash}#${outputIndex}`,
    );
  }
  const base0 = amountOf(selfInput.amount, baseUnit);
  const quote0 = amountOf(selfInput.amount, quoteUnit);

  const candidates = body.outputs.map((out) => {
    const isScriptSuccessor = out.address === auctionAddr &&
      out.inline_datum === initialDatum;
    const isTerminal = out.address !== auctionAddr &&
      paymentCredentialHash(out.address) === initialRedeemer;
    return { out, isScriptSuccessor, isTerminal };
  }).filter(({ out, isScriptSuccessor, isTerminal }) => {
    if (isScriptSuccessor) return true;
    return isTerminal && amountOf(out.amount, baseUnit) === 0n &&
      amountOf(out.amount, quoteUnit) > quote0;
  });

  for (const candidate of candidates) {
    const base1 = amountOf(candidate.out.amount, baseUnit);
    const quote1 = amountOf(candidate.out.amount, quoteUnit);
    const baseSubtracted = base0 - base1;
    const quoteAdded = quote1 - quote0;
    if (baseSubtracted <= 0n || quoteAdded <= 0n) continue;
    if (quoteAdded * activePrice.denom !== baseSubtracted * activePrice.num) {
      continue;
    }
    if (candidate.isScriptSuccessor && base1 === 0n) continue;
    if (candidate.isTerminal && base1 !== 0n) continue;
    return;
  }

  throw new Error(
    `Spending tx ${spendingTx} has no successor/terminal output matching this auction and exact active price`,
  );
}

while (Date.now() < deadline) {
  const spendingTx = await findSpendingTx();
  if (spendingTx) {
    if (!agentSawAuctionExecution()) {
      throw new Error(
        `Auction UTxO ${txHash}#${outputIndex} was spent, but agent log has no AuctionOrder::exec after verifier start ${startedAt}`,
      );
    }
    await verifyAuctionSpend(spendingTx);
    console.log(
      JSON.stringify(
        {
          status: "spent_by_agent_flow",
          txHash,
          outputIndex,
          spendingTx,
          agentLog,
        },
        null,
        2,
      ),
    );
    Deno.exit(0);
  }

  const isPresent = env.provider === "koios"
    ? await koiosUtxoIsPresent()
    : (await lucid.utxosByOutRef([{
      txHash: auctionTxHash,
      outputIndex: auctionOutputIndex,
    }])).length > 0;
  if (!isPresent) {
    console.log(
      JSON.stringify(
        {
          status: "not_seen_as_unspent_yet",
          txHash,
          outputIndex,
          note:
            "The provider has not reported the auction UTxO as unspent and no spending transaction is indexed yet.",
        },
        null,
        2,
      ),
    );
    await new Promise((resolve) => setTimeout(resolve, pollSecs * 1000));
    continue;
  }
  console.log(
    JSON.stringify({ status: "still_unspent", txHash, outputIndex }, null, 2),
  );
  await new Promise((resolve) => setTimeout(resolve, pollSecs * 1000));
}

throw new Error(
  `Auction UTxO ${txHash}#${outputIndex} was not spent by agent flow before timeout`,
);
