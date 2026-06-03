import { CML, fromHex } from "@lucid-evolution/lucid";
import { load } from "@std/dotenv";
import { mnemonicToEntropy } from "npm:bip39@3.1.0";
import { jsonStringify } from "./src/auction.ts";
import { readFlowEnv } from "./src/env.ts";
import { makeLucid } from "./src/lucid.ts";

function required(vars: Record<string, string>, key: string): string {
  const value = vars[key];
  if (!value) throw new Error(`Missing required env: ${key}`);
  return value;
}

function rootKeyFromSeed(seed: string): CML.Bip32PrivateKey {
  const entropy = mnemonicToEntropy(seed.trim());
  return CML.Bip32PrivateKey.from_bip39_entropy(
    fromHex(entropy),
    new Uint8Array(),
  );
}

function addresses(rootKey: CML.Bip32PrivateKey, wallet: string) {
  const operatorKey = rootKey.to_bech32();
  const operatorPk = rootKey.to_public();
  const mainPkh = operatorPk.to_raw_key().hash();
  const mainCred = CML.Credential.new_pub_key(mainPkh);
  const collateral = CML.EnterpriseAddress.new(0, mainCred)
    .to_address()
    .to_bech32(undefined);
  const funding = [1, 2, 3, 4].map((ix) => {
    const stakePkh = operatorPk.derive(ix).to_raw_key().hash();
    return CML.BaseAddress.new(
      0,
      mainCred,
      CML.Credential.new_pub_key(stakePkh),
    ).to_address().to_bech32(undefined);
  });
  return { operatorKey, wallet, collateral, funding };
}

const envPath = Deno.env.get("FLOW_ENV_FILE") ??
  "testing/preprod/amm-limit-auction-flow/.env";
const dotenv = await load({ envPath, export: false });
const vars = { ...dotenv, ...Deno.env.toObject() };
const seed = await Deno.readTextFile(required(vars, "WALLET_SEED_FILE"));
const env = await readFlowEnv();
const lucid = await makeLucid(env);
lucid.selectWallet.fromSeed(seed.trim());
const wallet = await lucid.wallet().address();
const operatorKey = vars["OPERATOR_KEY_CBOR_HEX"]
  ? CML.Bip32PrivateKey.from_bech32(vars["OPERATOR_KEY_CBOR_HEX"])
  : rootKeyFromSeed(seed);
console.log(jsonStringify(addresses(operatorKey, wallet)));
