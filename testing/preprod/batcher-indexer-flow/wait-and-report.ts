type Args = {
  baseUrl: string;
  fromMs: number;
  report: string;
  runId: string;
  chainPoint: string;
  logDir: string;
  timeoutSecs: number;
  allowPartial: boolean;
};

type Batcher = {
  pkh: string;
};

type Metrics = {
  batcher: string;
  eligibleOrders: number;
  executedOrders: number;
  stillOpenEligibleOrders: number;
  missedEligibleOrders: number;
  medianResponseMs?: number | null;
  p95ResponseMs?: number | null;
};

function parseArgs(): Args {
  const args = Deno.args;
  const get = (name: string): string => {
    const index = args.indexOf(name);
    if (index === -1 || index + 1 >= args.length) {
      throw new Error(`${name} is required`);
    }
    return args[index + 1];
  };
  return {
    baseUrl: get("--base-url").replace(/\/$/, ""),
    fromMs: Number(get("--from-ms")),
    report: get("--report"),
    runId: get("--run-id"),
    chainPoint: get("--chain-point"),
    logDir: get("--log-dir"),
    timeoutSecs: Number(get("--timeout-secs")),
    allowPartial: args.includes("--allow-partial"),
  };
}

async function getJson<T>(url: string): Promise<T> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`${url} returned HTTP ${response.status}`);
  }
  return await response.json() as T;
}

async function healthStatus(baseUrl: string): Promise<number | null> {
  try {
    const response = await fetch(`${baseUrl}/health`);
    return response.status;
  } catch {
    return null;
  }
}

function invariantHolds(metrics: Metrics): boolean {
  return metrics.eligibleOrders ===
    metrics.executedOrders + metrics.stillOpenEligibleOrders + metrics.missedEligibleOrders;
}

async function snapshot(baseUrl: string, fromMs: number): Promise<{ batchers: Batcher[]; metrics: Metrics[] }> {
  const batchers = await getJson<Batcher[]>(`${baseUrl}/batchers`);
  const metrics: Metrics[] = [];
  const toMs = Date.now();
  for (const batcher of batchers) {
    metrics.push(
      await getJson<Metrics>(
        `${baseUrl}/batchers/${batcher.pkh}/metrics?fromMs=${fromMs}&toMs=${toMs}`,
      ),
    );
  }
  return { batchers, metrics };
}

async function main(): Promise<void> {
  const args = parseArgs();
  const deadline = Date.now() + args.timeoutSecs * 1000;
  let lastError = "";
  let lastSnapshot: { batchers: Batcher[]; metrics: Metrics[] } = { batchers: [], metrics: [] };
  let lastHealthStatus: number | null = null;

  while (Date.now() < deadline) {
    lastHealthStatus = await healthStatus(args.baseUrl);
    try {
      lastSnapshot = await snapshot(args.baseUrl, args.fromMs);
      const hasBatcher = lastSnapshot.batchers.length > 0;
      const hasEligibleOrders = lastSnapshot.metrics.some((metrics) => metrics.eligibleOrders > 0);
      const invalid = lastSnapshot.metrics.find((metrics) => !invariantHolds(metrics));
      if (invalid) {
        throw new Error(`metrics invariant failed for batcher ${invalid.batcher}`);
      }
      const syncComplete = lastHealthStatus === 200;
      if (!syncComplete && !args.allowPartial) {
        lastError =
          `indexer is still syncing: health=${lastHealthStatus}, batchers=${lastSnapshot.batchers.length}`;
      } else if (hasBatcher && hasEligibleOrders) {
        const report = {
          status: "ok",
          completeness: syncComplete ? "synced" : "partial",
          runId: args.runId,
          baseUrl: args.baseUrl,
          healthStatus: lastHealthStatus,
          fromMs: args.fromMs,
          toMs: Date.now(),
          chainPoint: JSON.parse(await Deno.readTextFile(args.chainPoint)),
          batchers: lastSnapshot.batchers,
          metrics: lastSnapshot.metrics,
          logs: args.logDir,
        };
        await Deno.writeTextFile(args.report, `${JSON.stringify(report, null, 2)}\n`);
        console.log(JSON.stringify(report, null, 2));
        return;
      } else if (syncComplete) {
        lastError =
          `indexer is synced, but no eligible orders were observed from fromMs=${args.fromMs}`;
      } else {
        lastError = `observed ${lastSnapshot.batchers.length} batchers, but no eligible orders yet`;
      }
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error);
    }
    await new Promise((resolve) => setTimeout(resolve, 5_000));
  }

  const report = {
    status: "failed",
    reason: lastError,
    runId: args.runId,
    baseUrl: args.baseUrl,
    healthStatus: lastHealthStatus,
    fromMs: args.fromMs,
    chainPoint: JSON.parse(await Deno.readTextFile(args.chainPoint)),
    batchers: lastSnapshot.batchers,
    metrics: lastSnapshot.metrics,
    logs: args.logDir,
  };
  await Deno.writeTextFile(args.report, `${JSON.stringify(report, null, 2)}\n`);
  console.error(JSON.stringify(report, null, 2));
  Deno.exit(1);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  Deno.exit(1);
});
