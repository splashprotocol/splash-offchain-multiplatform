import { unitOf } from "../amm-limit-auction-flow/src/auction.ts";
import { readDemoEnv } from "../amm-limit-auction-flow/src/demo-env.ts";
import { makeLucid } from "../amm-limit-auction-flow/src/lucid.ts";
import type { Asset } from "../amm-limit-auction-flow/src/env.ts";

const { env, vars } = await readDemoEnv();
const lucid = await makeLucid(env);
const seed = await Deno.readTextFile(env.walletSeedFile);
lucid.selectWallet.fromSeed(seed.trim());

const policy = vars["WAIT_ASSET_POLICY"] ?? "";
const nameHex = vars["WAIT_ASSET_NAME_HEX"] ?? "";
const minAmount = BigInt(vars["WAIT_MIN_AMOUNT"] ?? "1");
const minLovelace = BigInt(vars["WAIT_MIN_LOVELACE"] ?? "0");
const timeoutSecs = Number(vars["WAIT_TIMEOUT_SECS"] ?? "240");
const pollSecs = Number(vars["WAIT_POLL_SECS"] ?? "5");

const unit = unitOf({ policy, nameHex } satisfies Asset);
const deadline = Date.now() + timeoutSecs * 1000;

while (Date.now() < deadline) {
  const utxos = await lucid.wallet().getUtxos();
  const plain = utxos.find((utxo) =>
    !utxo.scriptRef &&
    !utxo.datum &&
    !utxo.datumHash &&
    BigInt(utxo.assets[unit] ?? 0n) >= minAmount &&
    BigInt(utxo.assets.lovelace ?? 0n) >= minLovelace
  );
  if (plain) {
    console.log(JSON.stringify({
      status: "ok",
      txHash: plain.txHash,
      outputIndex: plain.outputIndex,
      unit,
      amount: String(plain.assets[unit] ?? 0n),
      lovelace: String(plain.assets.lovelace ?? 0n),
    }, null, 2));
    Deno.exit(0);
  }
  await new Promise((resolve) => setTimeout(resolve, pollSecs * 1000));
}

throw new Error(
  `Timed out waiting for plain wallet UTxO with at least ${minAmount} ${unit} and ${minLovelace} lovelace`,
);
