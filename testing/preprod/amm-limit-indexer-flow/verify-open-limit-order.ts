import { credentialToAddress } from "@lucid-evolution/utils";
import { blockfrostBaseUrl, cardanoNetwork } from "../amm-limit-auction-flow/src/env.ts";
import { readDemoEnv } from "../amm-limit-auction-flow/src/demo-env.ts";

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

async function findSpendingTx(): Promise<string | undefined> {
  if (env.provider !== "blockfrost") {
    throw new Error("verify-open-limit-order.ts currently supports only PROVIDER=blockfrost");
  }
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

const timeoutSecs = Number(Deno.env.get("VERIFY_TIMEOUT_SECS") ?? "180");
const pollSecs = Number(Deno.env.get("VERIFY_POLL_SECS") ?? "10");
const deadline = Date.now() + timeoutSecs * 1000;

while (Date.now() < deadline) {
  const spendingTx = await findSpendingTx();
  if (spendingTx) {
    console.log(JSON.stringify(
      {
        status: "unexpectedly_spent",
        txHash,
        outputIndex,
        spendingTx,
      },
      null,
      2,
    ));
    Deno.exit(1);
  }
  console.log(JSON.stringify({ status: "still_unspent", txHash, outputIndex }));
  await new Promise((resolve) => setTimeout(resolve, pollSecs * 1000));
}

console.log(JSON.stringify(
  {
    status: "still_open_after_window",
    txHash,
    outputIndex,
    observedForSecs: timeoutSecs,
  },
  null,
  2,
));
