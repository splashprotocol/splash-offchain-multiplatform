import {
  Blockfrost,
  Lucid,
  LucidEvolution,
  Script,
  scriptFromNative,
  UTxO,
  validatorToAddress,
} from "@lucid-evolution/lucid";
import { validatorToScriptHash } from "@lucid-evolution/utils";

const network = "Preprod" as const;
const artifactSetArg = Deno.args.find((arg) => arg.startsWith("--artifact-set="));
const artifactSet = artifactSetArg?.split("=", 2)[1] ?? "preprod-royalty-2026-10-05";
if (!/^[a-z0-9][a-z0-9._-]*$/.test(artifactSet)) {
  throw new Error(`invalid --artifact-set value: ${artifactSet}`);
}
if (artifactSet.startsWith("mainnet-")) {
  throw new Error("mainnet artifact sets cannot be used with the Preprod deployer");
}
const artifactDir = new URL(`./${artifactSet}/`, import.meta.url);
const manifestUrl = new URL("./script-hashes.txt", artifactDir);
const submit = Deno.args.includes("--submit");
const reportPath = artifactSet === "preprod-royalty-2026-10-05"
  ? "./preprod-royalty-reference-scripts.json"
  : `./${artifactSet}-reference-scripts.json`;
const pendingPath = artifactSet === "preprod-royalty-2026-10-05"
  ? "./preprod-royalty-reference-scripts.pending.json"
  : `./${artifactSet}-reference-scripts.pending.json`;
const maxNewBatchesArg = Deno.args.find((arg) =>
  arg.startsWith("--max-new-batches=")
);
const maxNewBatches = maxNewBatchesArg === undefined
  ? Number.POSITIVE_INFINITY
  : Number(maxNewBatchesArg.split("=", 2)[1]);
// A transaction also carries inputs, change, datum and witness bytes. Keep the
// aggregate reference-script payload comfortably below the 16 KiB ledger cap.
const maxReferenceScriptBytesPerTx = 10_000;

type Artifact = {
  name: string;
  file: string;
  expectedHash: string;
  script: Script;
};

type Deployment = Record<string, {
  hash: string;
  referenceUtxo: { txHash: string; outputIndex: number };
}>;

type PendingBatch = {
  txHash: string;
  cbor: string;
  artifacts: { name: string; hash: string; outputIndex: number }[];
};

function parseManifest(text: string): Map<string, string> {
  const entries = text.trim().split(/\r?\n/).map((line) => line.split("="));
  return new Map(entries.map(([name, hash]) => {
    if (!name || !hash || !/^[0-9a-f]{56}$/.test(hash)) {
      throw new Error(`invalid manifest line: ${name}=${hash}`);
    }
    return [name, hash];
  }));
}

async function loadArtifacts(): Promise<Artifact[]> {
  const hashes = parseManifest(await Deno.readTextFile(manifestUrl));
  const artifacts: Artifact[] = [];
  for (const [name, expectedHash] of hashes) {
    const file = `${name}.uplc`;
    const script = {
      type: "PlutusV2" as const,
      script: Array.from(await Deno.readFile(new URL(`./${file}`, artifactDir)))
        .map((byte) => byte.toString(16).padStart(2, "0"))
        .join(""),
    };
    const actualHash = validatorToScriptHash(script);
    if (actualHash !== expectedHash) {
      throw new Error(`${file}: expected ${expectedHash}, got ${actualHash}`);
    }
    artifacts.push({ name, file, expectedHash, script });
  }
  return artifacts;
}

async function getLucid(): Promise<LucidEvolution> {
  const projectId = blockfrostProjectId();
  return Lucid(
    new Blockfrost("https://cardano-preprod.blockfrost.io/api/v0", projectId),
    network,
  );
}

function blockfrostProjectId(): string {
  const projectId = Deno.env.get("BLOCKFROST_PREPROD_PROJECT_ID");
  if (!projectId) {
    throw new Error("BLOCKFROST_PREPROD_PROJECT_ID is required with --submit");
  }
  return projectId;
}

function scriptBytes(artifact: Artifact): number {
  return artifact.script.script.length / 2;
}

function batches(artifacts: Artifact[]): Artifact[][] {
  const result: Artifact[][] = [];
  let batch: Artifact[] = [];
  let batchBytes = 0;
  for (const artifact of artifacts) {
    const bytes = scriptBytes(artifact);
    if (bytes > maxReferenceScriptBytesPerTx) {
      throw new Error(
        `${artifact.name} exceeds the per-transaction script budget`,
      );
    }
    if (batch.length > 0 && batchBytes + bytes > maxReferenceScriptBytesPerTx) {
      result.push(batch);
      batch = [];
      batchBytes = 0;
    }
    batch.push(artifact);
    batchBytes += bytes;
  }
  if (batch.length > 0) result.push(batch);
  return result;
}

async function readDeployment(): Promise<Deployment> {
  try {
    return JSON.parse(await Deno.readTextFile(reportPath)) as Deployment;
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return {};
    throw error;
  }
}

async function writeDeployment(deployment: Deployment): Promise<void> {
  await Deno.writeTextFile(
    reportPath,
    `${JSON.stringify(deployment, null, 2)}\n`,
  );
}

async function readPending(): Promise<PendingBatch | undefined> {
  try {
    return JSON.parse(await Deno.readTextFile(pendingPath)) as PendingBatch;
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return undefined;
    throw error;
  }
}

async function writePending(pending: PendingBatch): Promise<void> {
  await Deno.writeTextFile(
    pendingPath,
    `${JSON.stringify(pending, null, 2)}\n`,
  );
}

async function clearPending(): Promise<void> {
  try {
    await Deno.remove(pendingPath);
  } catch (error) {
    if (!(error instanceof Deno.errors.NotFound)) throw error;
  }
}

function hexBytes(hex: string): Uint8Array {
  if (!/^(?:[0-9a-f]{2})+$/i.test(hex)) {
    throw new Error("invalid transaction CBOR");
  }
  return Uint8Array.from(hex.match(/.{2}/g)!.map((byte) => parseInt(byte, 16)));
}

async function broadcastPending(pending: PendingBatch): Promise<void> {
  const response = await fetch(
    "https://cardano-preprod.blockfrost.io/api/v0/tx/submit",
    {
      method: "POST",
      headers: {
        project_id: blockfrostProjectId(),
        "content-type": "application/cbor",
      },
      body: hexBytes(pending.cbor),
    },
  );
  if (!response.ok) {
    const detail = await response.text();
    // Blockfrost can return this after a previous successful broadcast. The
    // caller polls the exact signed hash before deciding that recovery failed.
    if (!detail.includes("All inputs are spent")) {
      throw new Error(`Blockfrost submit failed: ${response.status} ${detail}`);
    }
  }
}

async function verifyBatchOnBlockfrost(
  txHash: string,
  batch: Artifact[],
): Promise<boolean> {
  const response = await fetch(
    `https://cardano-preprod.blockfrost.io/api/v0/txs/${txHash}/utxos`,
    { headers: { project_id: blockfrostProjectId() } },
  );
  if (response.status === 404) return false;
  if (!response.ok) {
    throw new Error(
      `Blockfrost query for ${txHash} failed: ${response.status} ${await response
        .text()}`,
    );
  }
  const tx = await response.json() as {
    outputs: { output_index: number; reference_script_hash: string | null }[];
  };
  return batch.every((artifact, outputIndex) =>
    tx.outputs.some((output) =>
      output.output_index === outputIndex &&
      output.reference_script_hash === artifact.expectedHash
    )
  );
}

async function verifyRecordedBatch(
  batch: Artifact[],
  deployment: Deployment,
): Promise<boolean> {
  const transactions = new Map<string, {
    outputs: { output_index: number; reference_script_hash: string | null }[];
  }>();
  for (const artifact of batch) {
    const recorded = deployment[artifact.name];
    if (!recorded || recorded.hash !== artifact.expectedHash) return false;

    let transaction = transactions.get(recorded.referenceUtxo.txHash);
    if (!transaction) {
      const response = await fetch(
        `https://cardano-preprod.blockfrost.io/api/v0/txs/${recorded.referenceUtxo.txHash}/utxos`,
        { headers: { project_id: blockfrostProjectId() } },
      );
      if (response.status === 404) return false;
      if (!response.ok) {
        throw new Error(
          `Blockfrost query for ${recorded.referenceUtxo.txHash} failed: ${response.status} ${await response
            .text()}`,
        );
      }
      const fetched = await response.json() as {
        outputs: {
          output_index: number;
          reference_script_hash: string | null;
        }[];
      };
      transactions.set(recorded.referenceUtxo.txHash, fetched);
      transaction = fetched;
    }
    const output = transaction.outputs.find((candidate) =>
      candidate.output_index === recorded.referenceUtxo.outputIndex
    );
    if (!output || output.reference_script_hash !== artifact.expectedHash) {
      return false;
    }
  }
  return true;
}

async function waitForBatchOnBlockfrost(
  txHash: string,
  batch: Artifact[],
): Promise<void> {
  for (let attempt = 1; attempt <= 24; attempt += 1) {
    if (await verifyBatchOnBlockfrost(txHash, batch)) return;
    await new Promise((resolve) => setTimeout(resolve, 5_000));
  }
  throw new Error(
    `transaction ${txHash} did not expose all reference outputs within two minutes`,
  );
}

function pendingArtifacts(
  pending: PendingBatch,
  artifacts: Artifact[],
): Artifact[] {
  return pending.artifacts.map((recorded, expectedIndex) => {
    if (recorded.outputIndex !== expectedIndex) {
      throw new Error("pending batch has non-sequential output indexes");
    }
    const artifact = artifacts.find((candidate) =>
      candidate.name === recorded.name
    );
    if (!artifact || artifact.expectedHash !== recorded.hash) {
      throw new Error(
        `pending batch does not match manifest artifact ${recorded.name}`,
      );
    }
    return artifact;
  });
}

function recordConfirmedBatch(
  deployment: Deployment,
  txHash: string,
  batch: Artifact[],
): void {
  batch.forEach((artifact, outputIndex) => {
    deployment[artifact.name] = {
      hash: artifact.expectedHash,
      referenceUtxo: { txHash, outputIndex },
    };
  });
}

async function recoverPendingBatch(
  artifacts: Artifact[],
  deployment: Deployment,
): Promise<void> {
  const pending = await readPending();
  if (!pending) return;
  const batch = pendingArtifacts(pending, artifacts);
  if (!await verifyBatchOnBlockfrost(pending.txHash, batch)) {
    console.log(`Rebroadcasting pending batch ${pending.txHash}.`);
    await broadcastPending(pending);
    await waitForBatchOnBlockfrost(pending.txHash, batch);
  }
  recordConfirmedBatch(deployment, pending.txHash, batch);
  await writeDeployment(deployment);
  await clearPending();
  console.log(`Recovered confirmed batch ${pending.txHash}.`);
}

async function fundingUtxo(lucid: LucidEvolution): Promise<UTxO> {
  const address = await lucid.wallet().address();
  const response = await fetch(
    `https://cardano-preprod.blockfrost.io/api/v0/addresses/${address}/utxos?count=100`,
    { headers: { project_id: blockfrostProjectId() } },
  );
  if (!response.ok) {
    throw new Error(
      `Blockfrost funding-UTxO query failed: ${response.status} ${await response
        .text()}`,
    );
  }
  const utxos = await response.json() as {
    tx_hash: string;
    output_index: number;
    amount: { unit: string; quantity: string }[];
    data_hash: string | null;
    inline_datum: string | null;
    reference_script_hash: string | null;
  }[];
  const candidates = utxos
    .filter((utxo) =>
      utxo.data_hash === null && utxo.inline_datum === null &&
      utxo.reference_script_hash === null
    )
    .map((utxo) => ({
      ...utxo,
      lovelace: BigInt(
        utxo.amount.find((asset) => asset.unit === "lovelace")?.quantity ?? "0",
      ),
    }))
    .sort((
      a,
      b,
    ) => (a.lovelace > b.lovelace ? -1 : a.lovelace < b.lovelace ? 1 : 0));
  const selected = candidates[0];
  if (!selected) {
    throw new Error("no plain funding UTxO is available in the Preprod wallet");
  }
  return {
    txHash: selected.tx_hash,
    outputIndex: selected.output_index,
    address,
    assets: Object.fromEntries(
      selected.amount.map(({ unit, quantity }) => [unit, BigInt(quantity)]),
    ),
  };
}

async function main(): Promise<void> {
  if (
    maxNewBatches !== Number.POSITIVE_INFINITY &&
    (!Number.isInteger(maxNewBatches) || maxNewBatches < 1)
  ) {
    throw new Error("--max-new-batches must be a positive integer");
  }
  const artifacts = await loadArtifacts();
  console.log(`Validated ${artifacts.length} Plutus V2 artifacts:`);
  for (const artifact of artifacts) {
    console.log(
      `  ${artifact.name}=${artifact.expectedHash} (${
        artifact.script.script.length / 2
      } bytes)`,
    );
  }

  const artifactBatches = batches(artifacts);
  console.log(
    `\nDeployment batches (${maxReferenceScriptBytesPerTx} script bytes maximum):`,
  );
  artifactBatches.forEach((batch, index) => {
    console.log(
      `  #${index + 1}: ${batch.map((artifact) => artifact.name).join(", ")} (${
        batch.reduce((sum, artifact) => sum + scriptBytes(artifact), 0)
      } bytes)`,
    );
  });

  if (!submit) {
    console.log("\nDry run complete. No transaction was built or submitted.");
    console.log(
      "To deploy: set BLOCKFROST_PREPROD_PROJECT_ID and PREPROD_WALLET_SEED, then add --submit.",
    );
    return;
  }

  const seed = Deno.env.get("PREPROD_WALLET_SEED");
  if (!seed) {
    throw new Error("PREPROD_WALLET_SEED is required with --submit");
  }

  const lucid = await getLucid();
  lucid.selectWallet.fromSeed(seed);

  // A native script works as a stable, network-specific carrier address. Every
  // output contains one reference script and a trivial inline datum.
  const carrier = scriptFromNative({ type: "before", slot: 0 });
  const carrierAddress = validatorToAddress(network, carrier);

  const deployment = await readDeployment();
  await recoverPendingBatch(artifacts, deployment);
  let submittedBatches = 0;
  for (const [batchIndex, batch] of artifactBatches.entries()) {
    const existing = batch.filter((artifact) =>
      deployment[artifact.name] !== undefined
    );
    if (existing.length > 0 && existing.length !== batch.length) {
      throw new Error(
        `batch #${
          batchIndex + 1
        } is only partially recorded; resolve its reference outputs before resuming`,
      );
    }
    if (existing.length === batch.length) {
      if (!await verifyRecordedBatch(batch, deployment)) {
        throw new Error(
          `batch #${
            batchIndex + 1
          } does not match the recorded reference outputs`,
        );
      }
      console.log(
        `Verified existing batch #${batchIndex + 1}; not re-submitting it.`,
      );
      continue;
    }

    const input = await fundingUtxo(lucid);
    console.log(`Using funding input ${input.txHash}#${input.outputIndex}.`);
    let tx = lucid.newTx().collectFrom([input]);
    for (const artifact of batch) {
      tx = tx.pay.ToAddressWithData(
        carrierAddress,
        { kind: "inline", value: "00" },
        undefined,
        artifact.script,
      );
    }

    const completed = await tx.complete();
    const signed = await completed.sign.withWallet().complete();
    const txHash = signed.toHash();
    const pending: PendingBatch = {
      txHash,
      cbor: signed.toCBOR(),
      artifacts: batch.map((artifact, outputIndex) => ({
        name: artifact.name,
        hash: artifact.expectedHash,
        outputIndex,
      })),
    };
    // Store an independently re-broadcastable transaction before submission.
    // The confirmed report changes only after Blockfrost verification.
    await writePending(pending);
    await broadcastPending(pending);
    console.log(`Submitted batch #${batchIndex + 1}: ${txHash}`);
    await waitForBatchOnBlockfrost(txHash, batch);
    recordConfirmedBatch(deployment, txHash, batch);
    await writeDeployment(deployment);
    await clearPending();
    submittedBatches += 1;
    if (submittedBatches === maxNewBatches) {
      console.log(
        `Stopped after ${submittedBatches} newly submitted batch(es).`,
      );
      return;
    }
  }
  console.log(`Verified all references. Wrote ${reportPath}`);
}

await main();
