type ChainPoint = {
  slot: number;
  hash: string;
};

type Deployment = {
  limitOrder?: {
    hash?: string;
  };
};

type Args = {
  chainPoint: string;
  nodeSocket: string;
  stateDir: string;
  httpPort: number;
  out: string;
  deployment: string;
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
    chainPoint: get("--chain-point"),
    nodeSocket: get("--node-socket"),
    stateDir: get("--state-dir"),
    httpPort: Number(get("--http-port")),
    out: get("--out"),
    deployment: get("--deployment"),
  };
}

async function limitOrderScriptHash(deploymentPath: string): Promise<string> {
  const explicit = Deno.env.get("LIMIT_ORDER_SCRIPT_HASH");
  if (explicit) {
    return explicit;
  }
  const deployment = JSON.parse(await Deno.readTextFile(deploymentPath)) as Deployment;
  const hash = deployment.limitOrder?.hash;
  if (!hash) {
    throw new Error(
      `Cannot resolve limit-order script hash from ${deploymentPath}. Set LIMIT_ORDER_SCRIPT_HASH.`,
    );
  }
  return hash;
}

async function main(): Promise<void> {
  const args = parseArgs();
  if (!Number.isInteger(args.httpPort) || args.httpPort <= 0) {
    throw new Error("--http-port must be a positive integer");
  }
  const point = JSON.parse(await Deno.readTextFile(args.chainPoint)) as ChainPoint;
  if (!Number.isInteger(point.slot) || !point.hash) {
    throw new Error(`Invalid chain point JSON: ${args.chainPoint}`);
  }
  const hash = await limitOrderScriptHash(args.deployment);
  const config = {
    chainSync: {
      startingPoint: {
        Specific: [point.slot, point.hash],
      },
      disableRollbacksUntil: 0,
      dbPath: `${args.stateDir}/chain-sync.rocksdb`,
    },
    node: {
      path: args.nodeSocket,
      magic: 1,
    },
    networkId: 0,
    trackedLimitOrderScriptHashes: [hash],
    indexDbPath: `${args.stateDir}/execution-index.rocksdb`,
    http: {
      host: "127.0.0.1",
      port: args.httpPort,
    },
  };
  await Deno.writeTextFile(args.out, `${JSON.stringify(config, null, 2)}\n`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  Deno.exit(1);
});
