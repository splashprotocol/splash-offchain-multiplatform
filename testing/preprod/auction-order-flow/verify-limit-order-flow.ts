import { credentialToAddress } from "@lucid-evolution/utils";
import { blockfrostBaseUrl, cardanoNetwork } from "./src/env.ts";
import { readDemoEnv } from "./src/demo-env.ts";

type Deployment = {
  limitOrder: {
    hash: string;
  };
};

const { env } = await readDemoEnv();
const txHash = Deno.env.get("LIMIT_TX_HASH");
const outputIndexText = Deno.env.get("LIMIT_OUTPUT_INDEX");
if (!txHash || !outputIndexText) {
  throw new Error("LIMIT_TX_HASH and LIMIT_OUTPUT_INDEX are required");
}
const outputIndex = Number(outputIndexText);
const agentLogEnv = Deno.env.get("AGENT_LOG_FILE");
if (!agentLogEnv) {
  throw new Error("AGENT_LOG_FILE is required");
}
const agentLog = agentLogEnv;

const deployment = JSON.parse(
  await Deno.readTextFile(env.deploymentConfig),
) as Deployment;
const orderAddress = credentialToAddress(cardanoNetwork(env), {
  hash: deployment.limitOrder.hash,
  type: "Script",
});

async function blockfrostJson<T>(path: string): Promise<T | undefined> {
  if (env.provider !== "blockfrost") return undefined;
  const headers = { project_id: env.blockfrostProjectId! };
  const res = await fetch(`${blockfrostBaseUrl(env)}${path}`, { headers });
  return res.ok ? await res.json() as T : undefined;
}

async function koiosJson<T>(
  path: string,
  body: unknown,
): Promise<T | undefined> {
  if (env.provider !== "koios") return undefined;
  const res = await fetch(`https://preprod.koios.rest/api/v1${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  return res.ok ? await res.json() as T : undefined;
}

async function findSpendingTx(): Promise<string | undefined> {
  if (env.provider === "blockfrost") {
    const txs = await blockfrostJson<Array<{ tx_hash: string }>>(
      `/addresses/${orderAddress}/transactions?order=desc&count=100`,
    ) ?? [];
    for (const item of txs) {
      const body = await blockfrostJson<{
        inputs?: Array<{ tx_hash: string; output_index: number }>;
      }>(`/txs/${item.tx_hash}/utxos`);
      if (
        body?.inputs?.some((input) =>
          input.tx_hash === txHash && input.output_index === outputIndex
        )
      ) {
        return item.tx_hash;
      }
    }
    return undefined;
  }

  const txs = await koiosJson<Array<{ tx_hash: string }>>("/address_txs", {
    _addresses: [orderAddress],
  }) ?? [];
  for (const item of txs.reverse()) {
    const body = await koiosJson<
      Array<{
        inputs?: Array<{ tx_hash?: string; tx_index?: number }>;
      }>
    >("/tx_info", { _tx_hashes: [item.tx_hash] });
    if (
      body?.[0]?.inputs?.some((input) =>
        input.tx_hash === txHash && input.tx_index === outputIndex
      )
    ) {
      return item.tx_hash;
    }
  }
  return undefined;
}

function agentSawLimitExecution(): boolean {
  try {
    return Deno.readTextFileSync(agentLog).includes("LimitOrder::exec(");
  } catch (_) {
    return false;
  }
}

const timeoutSecs = Number(Deno.env.get("VERIFY_TIMEOUT_SECS") ?? "600");
const pollSecs = Number(Deno.env.get("VERIFY_POLL_SECS") ?? "10");
const deadline = Date.now() + timeoutSecs * 1000;

while (Date.now() < deadline) {
  const spendingTx = await findSpendingTx();
  if (spendingTx && agentSawLimitExecution()) {
    console.log(JSON.stringify(
      {
        status: "spent_by_agent_flow",
        txHash,
        outputIndex,
        spendingTx,
        agentLog,
      },
      null,
      2,
    ));
    Deno.exit(0);
  }
  console.log(JSON.stringify({ status: "still_unspent", txHash, outputIndex }));
  await new Promise((resolve) => setTimeout(resolve, pollSecs * 1000));
}

throw new Error(
  `Timed out waiting for limit order ${txHash}#${outputIndex} to be spent by agent flow`,
);
