type ChainPoint = {
  slot: number;
  hash: string;
  resolvedFrom: string;
  provider: string;
  blockTime?: number;
  blockHeight?: number;
  requestedSlot?: number;
  requestedBlockHeight?: number;
  fastForwardedToScriptActivity?: {
    address: string;
    txHash: string;
    txBlockHeight: number;
    direction: "afterRequestedPoint" | "beforeRequestedPoint";
  };
};

const PREPROD_SYSTEM_START = 1_655_683_200;

function requiredEnv(name: string): string {
  const value = Deno.env.get(name);
  if (!value) {
    throw new Error(`${name} is required`);
  }
  return value;
}

function parseStartDatetime(value: string): number {
  if (value === "now") {
    return Math.floor(Date.now() / 1000);
  }
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) {
    throw new Error(
      `Invalid start datetime "${value}". Use "now" or ISO-8601, e.g. 2026-06-04T00:00:00Z.`,
    );
  }
  return Math.floor(parsed / 1000);
}

async function blockfrostGet(path: string): Promise<unknown | null> {
  const projectId = requiredEnv("BLOCKFROST_PROJECT_ID");
  const response = await fetch(`https://cardano-preprod.blockfrost.io/api/v0/${path}`, {
    headers: { project_id: projectId },
  });
  if (response.status === 404) {
    return null;
  }
  if (!response.ok) {
    const body = await response.text();
    throw new Error(`Blockfrost ${path} failed: HTTP ${response.status} ${body}`);
  }
  return await response.json();
}

async function resolveByBlockfrostSlot(slot: number, resolvedFrom: string): Promise<ChainPoint> {
  const maxLookbackSlots = Number(Deno.env.get("BATCHER_INDEXER_BLOCK_LOOKBACK_SLOTS") ?? "600");
  for (let current = slot; current >= 0 && current >= slot - maxLookbackSlots; current--) {
    const block = await blockfrostGet(`blocks/slot/${current}`) as {
      hash: string;
      slot: number;
      time?: number;
      height?: number;
    } | null;
    if (block) {
      return {
        slot: block.slot,
        hash: block.hash,
        resolvedFrom,
        provider: "blockfrost",
        blockTime: block.time,
        blockHeight: block.height,
      };
    }
  }
  throw new Error(
    `No preprod block found at or before slot ${slot} within ${maxLookbackSlots} slots.`,
  );
}

async function fastForwardToFirstScriptActivity(point: ChainPoint): Promise<ChainPoint> {
  const address = Deno.env.get("BATCHER_INDEXER_FAST_FORWARD_ADDRESS");
  if (!address || point.blockHeight === undefined) {
    return point;
  }
  let direction: "afterRequestedPoint" | "beforeRequestedPoint" = "afterRequestedPoint";
  let txs = await blockfrostGet(
    `addresses/${address}/transactions?from=${point.blockHeight}&order=asc&count=1`,
  ) as Array<{ tx_hash: string; block_height: number }> | null;
  let firstTx = txs?.[0];
  if (!firstTx) {
    if (Deno.env.get("BATCHER_INDEXER_ALLOW_BACKWARD_ACTIVITY_FALLBACK") !== "1") {
      return point;
    }
    direction = "beforeRequestedPoint";
    txs = await blockfrostGet(
      `addresses/${address}/transactions?to=${point.blockHeight}&order=desc&count=1`,
    ) as Array<{ tx_hash: string; block_height: number }> | null;
    firstTx = txs?.[0];
  }
  if (!firstTx) {
    return point;
  }
  let startBlockRef: string | number;
  if (direction === "beforeRequestedPoint") {
    const lookback = Number(Deno.env.get("BATCHER_INDEXER_FALLBACK_LOOKBACK_BLOCKS") ?? "1000");
    startBlockRef = Math.max(0, firstTx.block_height - lookback);
  } else {
    const tx = await blockfrostGet(`txs/${firstTx.tx_hash}`) as { block: string } | null;
    if (!tx?.block) {
      return point;
    }
    const txBlock = await blockfrostGet(`blocks/${tx.block}`) as {
      previous_block?: string;
    } | null;
    if (!txBlock?.previous_block) {
      return point;
    }
    startBlockRef = txBlock.previous_block;
  }
  const previous = await blockfrostGet(`blocks/${startBlockRef}`) as {
    hash: string;
    slot: number;
    height: number;
    time?: number;
  } | null;
  if (!previous) {
    return point;
  }
  return {
    slot: previous.slot,
    hash: previous.hash,
    resolvedFrom: point.resolvedFrom,
    provider: "blockfrost",
    blockTime: previous.time,
    blockHeight: previous.height,
    requestedSlot: point.slot,
    requestedBlockHeight: point.blockHeight,
    fastForwardedToScriptActivity: {
      address,
      txHash: firstTx.tx_hash,
      txBlockHeight: firstTx.block_height,
      direction,
    },
  };
}

async function resolveTipFromCardanoCli(resolvedFrom: string): Promise<ChainPoint> {
  const tipPath = requiredEnv("TIP_JSON");
  const tip = JSON.parse(await Deno.readTextFile(tipPath)) as {
    slot?: number;
    hash?: string;
  };
  if (typeof tip.slot !== "number" || typeof tip.hash !== "string" || tip.hash.length === 0) {
    throw new Error(`cardano-cli tip JSON does not contain slot/hash: ${tipPath}`);
  }
  return {
    slot: tip.slot,
    hash: tip.hash,
    resolvedFrom,
    provider: "cardano-cli",
  };
}

async function main(): Promise<void> {
  const from = requiredEnv("BATCHER_INDEXER_FROM");
  if (from === "now") {
    console.log(JSON.stringify(await resolveTipFromCardanoCli(from), null, 2));
    return;
  }

  const posix = parseStartDatetime(from);
  if (posix < PREPROD_SYSTEM_START) {
    throw new Error(`Start datetime ${from} is before preprod system start.`);
  }
  const slot = posix - PREPROD_SYSTEM_START;
  console.log(JSON.stringify(await fastForwardToFirstScriptActivity(await resolveByBlockfrostSlot(slot, from)), null, 2));
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  Deno.exit(1);
});
