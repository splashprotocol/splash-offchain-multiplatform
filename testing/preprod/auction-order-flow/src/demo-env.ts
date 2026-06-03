import { load } from "@std/dotenv";
import { Asset, ProviderEnv } from "./env.ts";

export type DemoEnv = ProviderEnv;

function required(vars: Record<string, string>, key: string): string {
  const value = vars[key];
  if (!value) throw new Error(`Missing required env: ${key}`);
  return value;
}

function optional(
  vars: Record<string, string>,
  key: string,
): string | undefined {
  return vars[key] || undefined;
}

export function bigintVar(
  vars: Record<string, string>,
  key: string,
  fallback: string,
): bigint {
  const value = optional(vars, key) ?? fallback;
  return BigInt(value);
}

export function boolVar(vars: Record<string, string>, key: string): boolean {
  return optional(vars, key) === "1";
}

export function assetVar(
  vars: Record<string, string>,
  prefix: string,
  fallbackPrefix?: string,
  allowAda = false,
): Asset {
  const policy = optional(vars, `${prefix}_POLICY`) ??
    (fallbackPrefix ? optional(vars, `${fallbackPrefix}_POLICY`) : undefined);
  const nameHex = optional(vars, `${prefix}_NAME_HEX`) ??
    (fallbackPrefix ? optional(vars, `${fallbackPrefix}_NAME_HEX`) : undefined);
  if (policy === undefined || nameHex === undefined) {
    throw new Error(
      `Missing ${prefix}_POLICY/${prefix}_NAME_HEX` +
        (fallbackPrefix ? ` or ${fallbackPrefix}_* fallback` : ""),
    );
  }
  if (policy === "" && !allowAda) {
    throw new Error(`${prefix}_POLICY cannot be empty for this script`);
  }
  if (policy !== "" && !/^[0-9a-fA-F]{56}$/.test(policy)) {
    throw new Error(`${prefix}_POLICY must be 28-byte hex`);
  }
  if (!/^[0-9a-fA-F]*$/.test(nameHex) || nameHex.length % 2 !== 0) {
    throw new Error(`${prefix}_NAME_HEX must be even-length hex`);
  }
  return { policy, nameHex };
}

export async function readDemoEnv(): Promise<{
  env: DemoEnv;
  vars: Record<string, string>;
}> {
  const envPath = Deno.env.get("FLOW_ENV_FILE") ??
    "testing/preprod/auction-order-flow/.env";
  const dotenv = await load({ envPath, export: false });
  const vars = { ...dotenv, ...Deno.env.toObject() };
  const network = optional(vars, "NETWORK") ?? "preprod";
  if (network !== "preprod") {
    throw new Error("These demo scripts are preprod-only; set NETWORK=preprod");
  }
  const provider = (optional(vars, "PROVIDER") ?? "blockfrost") as
    | "blockfrost"
    | "maestro"
    | "koios";
  return {
    vars,
    env: {
      network: "preprod",
      submit: boolVar(vars, "SUBMIT"),
      provider,
      blockfrostProjectId: optional(vars, "BLOCKFROST_PROJECT_ID"),
      maestroApiKey: optional(vars, "MAESTRO_API_KEY"),
      walletSeedFile: required(vars, "WALLET_SEED_FILE"),
      deploymentConfig: optional(vars, "DEPLOYMENT_CONFIG") ??
        "bloom-cardano-agent/resources/preprod.deployment.json",
    },
  };
}
