import {
  Blockfrost,
  Koios,
  Lucid,
  LucidEvolution,
  Maestro,
} from "@lucid-evolution/lucid";
import { blockfrostBaseUrl, cardanoNetwork, FlowEnv } from "./env.ts";

export async function makeLucid(env: FlowEnv): Promise<LucidEvolution> {
  if (env.provider === "koios") {
    return await Lucid(
      new Koios("https://preprod.koios.rest/api/v1"),
      cardanoNetwork(env),
    );
  }
  if (env.provider === "maestro") {
    if (!env.maestroApiKey) {
      throw new Error("MAESTRO_API_KEY is required for PROVIDER=maestro");
    }
    return await Lucid(
      new Maestro({
        network: "Preprod",
        apiKey: env.maestroApiKey,
        turboSubmit: false,
      }),
      cardanoNetwork(env),
    );
  }
  if (env.provider !== "blockfrost") {
    throw new Error(`Unsupported provider: ${env.provider}`);
  }
  if (!env.blockfrostProjectId) {
    throw new Error(
      "BLOCKFROST_PROJECT_ID is required for PROVIDER=blockfrost",
    );
  }
  return await Lucid(
    new Blockfrost(blockfrostBaseUrl(env), env.blockfrostProjectId),
    cardanoNetwork(env),
  );
}
