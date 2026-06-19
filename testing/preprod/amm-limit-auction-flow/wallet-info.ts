import { load } from "@std/dotenv";
import { walletFromSeed } from "@lucid-evolution/wallet";
import { jsonStringify } from "./src/auction.ts";

function required(vars: Record<string, string>, key: string): string {
  const value = vars[key];
  if (!value) throw new Error(`Missing required env: ${key}`);
  return value;
}

const envPath = Deno.env.get("FLOW_ENV_FILE") ??
  "testing/preprod/amm-limit-auction-flow/.env";
const dotenv = await load({ envPath, export: false });
const vars = { ...dotenv, ...Deno.env.toObject() };
const seed = await Deno.readTextFile(required(vars, "WALLET_SEED_FILE"));
const wallet = walletFromSeed(seed.trim(), { network: "Preprod" });
console.log(jsonStringify({ wallet: wallet.address }));
