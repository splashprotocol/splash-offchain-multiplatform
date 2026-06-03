import { readFlowEnv } from "./src/env.ts";
import { makeLucid } from "./src/lucid.ts";
import { jsonStringify } from "./src/auction.ts";
import { submitSignedTx } from "./src/submit.ts";
import { UTxO } from "@lucid-evolution/lucid";

const env = await readFlowEnv();
const address = Deno.env.get("AGENT_FUNDING_ADDRESS");
const addresses = (Deno.env.get("AGENT_FUNDING_ADDRESS_LIST") ?? "")
  .split(",")
  .map((item) => item.trim())
  .filter((item) => item.length > 0);
if (addresses.length === 0 && address) {
  addresses.push(address);
}
if (addresses.length === 0) {
  throw new Error("AGENT_FUNDING_ADDRESS or AGENT_FUNDING_ADDRESSES is required");
}

const lovelace = BigInt(Deno.env.get("AGENT_FUNDING_LOVELACE") ?? "50000000");
const lucid = await makeLucid(env);
const seed = await Deno.readTextFile(env.walletSeedFile);
lucid.selectWallet.fromSeed(seed.trim());

function isPlainWalletUtxo(utxo: UTxO): boolean {
  return !utxo.scriptRef && !utxo.datum && !utxo.datumHash;
}

function lovelaceOf(utxo: UTxO): bigint {
  return BigInt(utxo.assets.lovelace ?? 0);
}

const reservedOrderInputTxHash = env.orderInputTxHash ??
  env.auctionValidatorRefTxHash;
const walletUtxos = await lucid.wallet().getUtxos();
const selectedInputs: UTxO[] = [];
let selectedLovelace = 0n;
const requiredLovelace = lovelace * BigInt(addresses.length);
const feeBuffer = 5_000_000n;

for (const utxo of walletUtxos
  .filter((utxo) => utxo.txHash !== reservedOrderInputTxHash)
  .filter(isPlainWalletUtxo)
  .sort((left, right) => Number(lovelaceOf(right) - lovelaceOf(left)))) {
  selectedInputs.push(utxo);
  selectedLovelace += lovelaceOf(utxo);
  if (selectedLovelace >= requiredLovelace + feeBuffer) {
    break;
  }
}

if (selectedLovelace < requiredLovelace + feeBuffer) {
  throw new Error(
    `Wallet has ${selectedLovelace} spendable lovelace outside reserved ` +
      `order tx ${reservedOrderInputTxHash}; need at least ` +
      `${requiredLovelace + feeBuffer} for agent funding`,
  );
}

let txBuilder = lucid.newTx();
txBuilder = txBuilder.collectFrom(selectedInputs);
for (const fundingAddress of addresses) {
  txBuilder = txBuilder.pay.ToAddress(fundingAddress, { lovelace });
}
const tx = await txBuilder.complete();

if (!env.submit) {
  console.log(jsonStringify({ submit: false, addresses, value: { lovelace } }));
} else {
  const signed = await tx.sign.withWallet().complete();
  const txHash = await submitSignedTx(env, signed);
  console.log(jsonStringify({
    submit: true,
    txHash,
    addresses,
    value: { lovelace },
    selectedInputs: selectedInputs.map((utxo) => ({
      txHash: utxo.txHash,
      outputIndex: utxo.outputIndex,
      lovelace: lovelaceOf(utxo),
    })),
  }));
}
