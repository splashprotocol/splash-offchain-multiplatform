import { CML, Data, fromHex, paymentCredentialOf } from "@lucid-evolution/lucid";
import {
  credentialToAddress,
  mintingPolicyToId,
  scriptFromNative,
} from "@lucid-evolution/utils";
import { mnemonicToEntropy } from "npm:bip39@3.1.0";
import { jsonStringify, unitOf } from "./src/auction.ts";
import { Asset, cardanoNetwork } from "./src/env.ts";
import { assetVar, bigintVar, readDemoEnv } from "./src/demo-env.ts";
import { makeLucid } from "./src/lucid.ts";
import { RoyaltyPoolPoolValidatePool } from "../../../splash-testing-cardano/plutus.ts";
import { fetchSubmittedOutputIndex, submitSignedTx } from "./src/submit.ts";

const MAX_LQ_CAP = 9_223_372_036_854_775_807n;

type Deployment = {
  royaltyPoolLedgerFixed?: {
    hash: string;
  };
  royaltyPool?: {
    hash: string;
  };
  royaltyPoolDaoContract?: {
    hash: string;
  };
};

function optional(
  vars: Record<string, string>,
  key: string,
): string | undefined {
  return vars[key] || undefined;
}

function textToHex(text: string): string {
  return Array.from(new TextEncoder().encode(text))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

function assetFromNative(policy: string, nameHex: string): Asset {
  return { policy, nameHex };
}

function rootKeyFromSeed(seed: string): CML.Bip32PrivateKey {
  const entropy = mnemonicToEntropy(seed.trim());
  return CML.Bip32PrivateKey.from_bip39_entropy(
    fromHex(entropy),
    new Uint8Array(),
  );
}

function configuredOrMintedAsset(
  vars: Record<string, string>,
  prefix: string,
  fallbackPrefix: string,
  policy: string,
  defaultNameHex: string,
): { asset: Asset; shouldMint: boolean } {
  if (vars["POOL_MINT_DEMO_ASSETS"] === "1") {
    return {
      asset: assetFromNative(policy, defaultNameHex),
      shouldMint: true,
    };
  }
  if (
    optional(vars, `${prefix}_POLICY`) !== undefined ||
    optional(vars, `${fallbackPrefix}_POLICY`) !== undefined
  ) {
    return {
      asset: assetVar(vars, prefix, fallbackPrefix),
      shouldMint: false,
    };
  }
  return {
    asset: assetFromNative(policy, defaultNameHex),
    shouldMint: true,
  };
}

const { env, vars } = await readDemoEnv();
const lucid = await makeLucid(env);
const seed = await Deno.readTextFile(env.walletSeedFile);
lucid.selectWallet.fromSeed(seed.trim());

const walletAddress = await lucid.wallet().address();
const paymentKeyHash = paymentCredentialOf(walletAddress).hash;
const nativePolicy = scriptFromNative({ type: "sig", keyHash: paymentKeyHash });
const nativePolicyId = mintingPolicyToId(nativePolicy);
const runSuffix = optional(vars, "POOL_TOKEN_SUFFIX") ??
  new Date().toISOString().replaceAll(/[-:.TZ]/g, "").slice(0, 14);

const poolNft = assetFromNative(
  nativePolicyId,
  optional(vars, "POOL_NFT_NAME_HEX") ?? textToHex(`poolNft-${runSuffix}`),
);
const poolLq = assetFromNative(
  nativePolicyId,
  optional(vars, "POOL_LQ_NAME_HEX") ?? textToHex(`poolLq-${runSuffix}`),
);
const x = configuredOrMintedAsset(
  vars,
  "POOL_X",
  "AUCTION_BASE",
  nativePolicyId,
  textToHex(`poolX-${runSuffix}`),
);
const y = configuredOrMintedAsset(
  vars,
  "POOL_Y",
  "AUCTION_QUOTE",
  nativePolicyId,
  textToHex(`poolY-${runSuffix}`),
);

const xAmount = bigintVar(vars, "POOL_X_AMOUNT", "1000000");
const yAmount = bigintVar(vars, "POOL_Y_AMOUNT", "1000000");
const walletXAmount = bigintVar(vars, "POOL_WALLET_X_AMOUNT", "0");
const walletYAmount = bigintVar(vars, "POOL_WALLET_Y_AMOUNT", "26000");
const poolLovelace = bigintVar(vars, "POOL_LOVELACE", "12000000");
const initialLiquidity = bigintVar(vars, "POOL_INITIAL_LIQUIDITY", "1000000");
const lpFeeNum = bigintVar(vars, "POOL_LP_FEE_NUM", "99700");
const treasuryFeeNum = bigintVar(vars, "POOL_TREASURY_FEE_NUM", "0");
const royaltyFeeNum = bigintVar(vars, "POOL_ROYALTY_FEE_NUM", "0");
const treasuryX = bigintVar(vars, "POOL_TREASURY_X", "0");
const treasuryY = bigintVar(vars, "POOL_TREASURY_Y", "0");
const royaltyX = bigintVar(vars, "POOL_ROYALTY_X", "0");
const royaltyY = bigintVar(vars, "POOL_ROYALTY_Y", "0");
const royaltyNonce = bigintVar(vars, "POOL_ROYALTY_NONCE", "0");

if (initialLiquidity <= 0n || initialLiquidity >= MAX_LQ_CAP) {
  throw new Error(
    "POOL_INITIAL_LIQUIDITY must be between 1 and MAX_LQ_CAP - 1",
  );
}

const deployment = JSON.parse(
  await Deno.readTextFile(env.deploymentConfig),
) as Deployment;
const poolHash = deployment.royaltyPoolLedgerFixed?.hash ??
  deployment.royaltyPool?.hash;
const daoHash = deployment.royaltyPoolDaoContract?.hash;
if (!poolHash) {
  throw new Error(
    "Deployment config does not contain royaltyPoolLedgerFixed or royaltyPool",
  );
}
if (!daoHash) {
  throw new Error("Deployment config does not contain royaltyPoolDaoContract");
}
const poolAddress = credentialToAddress(cardanoNetwork(env), {
  hash: poolHash,
  type: "Script",
});
const lqInPool = MAX_LQ_CAP - initialLiquidity;
const operatorRootKey = vars["OPERATOR_KEY_CBOR_HEX"]
  ? CML.Bip32PrivateKey.from_bech32(vars["OPERATOR_KEY_CBOR_HEX"])
  : rootKeyFromSeed(seed);
const derivedRoyaltyPubKeyHex = Array.from(
  operatorRootKey.to_public().to_raw_key().to_raw_bytes(),
)
  .map((byte) => byte.toString(16).padStart(2, "0"))
  .join("");
const royaltyPubKeyHex = optional(vars, "POOL_ROYALTY_PUB_KEY_HEX") ??
  derivedRoyaltyPubKeyHex;
if (!/^[0-9a-fA-F]{64}$/.test(royaltyPubKeyHex)) {
  throw new Error("POOL_ROYALTY_PUB_KEY_HEX must be 32-byte public-key hex");
}

const datum = Data.to({
  poolnft: { policy: poolNft.policy, name: poolNft.nameHex },
  poolx: { policy: x.asset.policy, name: x.asset.nameHex },
  poolY: { policy: y.asset.policy, name: y.asset.nameHex },
  poolLq: { policy: poolLq.policy, name: poolLq.nameHex },
  feenum: lpFeeNum,
  treasuryFee: treasuryFeeNum,
  royaltyFee: royaltyFeeNum,
  treasuryx: treasuryX,
  treasuryy: treasuryY,
  royaltyx: royaltyX,
  royaltyy: royaltyY,
  daoPolicy: [{ Inline: [{ ScriptCredential: [daoHash] }] }],
  treasuryAddress: daoHash,
  royaltyPubKeyHash_256: royaltyPubKeyHex,
  royaltyNonce,
}, RoyaltyPoolPoolValidatePool.conf);

const poolValue: Record<string, bigint> = {
  lovelace: poolLovelace,
  [unitOf(poolNft)]: 1n,
  [unitOf(poolLq)]: lqInPool,
  [unitOf(x.asset)]: xAmount,
  [unitOf(y.asset)]: yAmount,
};
const minted: Record<string, bigint> = {
  [unitOf(poolNft)]: 1n,
  [unitOf(poolLq)]: MAX_LQ_CAP,
};
if (x.shouldMint) minted[unitOf(x.asset)] = xAmount;
if (y.shouldMint) minted[unitOf(y.asset)] = yAmount;
if (x.shouldMint) minted[unitOf(x.asset)] += walletXAmount;
if (y.shouldMint) minted[unitOf(y.asset)] += walletYAmount;

const walletValue: Record<string, bigint> = {};
if (walletXAmount > 0n) walletValue[unitOf(x.asset)] = walletXAmount;
if (walletYAmount > 0n) walletValue[unitOf(y.asset)] = walletYAmount;

let txBuilder = lucid
  .newTx()
  .attach.MintingPolicy(nativePolicy)
  .mintAssets(minted)
  .pay.ToAddressWithData(
    poolAddress,
    { kind: "inline", value: datum },
    poolValue,
  );
if (Object.keys(walletValue).length > 0) {
  txBuilder = txBuilder.pay.ToAddress(walletAddress, walletValue);
}
const tx = await txBuilder.complete();

const output = {
  submit: env.submit,
  address: poolAddress,
  datum,
  value: poolValue,
  walletValue,
  minted,
  poolNft,
  assetX: x.asset,
  assetY: y.asset,
  assetLq: poolLq,
  initialLiquidity,
  lpFeeNum,
  treasuryFeeNum,
  royaltyFeeNum,
  treasuryX,
  treasuryY,
  royaltyX,
  royaltyY,
  daoHash,
  royaltyPubKeyHex,
  royaltyNonce,
};

if (!env.submit) {
  console.log(jsonStringify({
    ...output,
    message:
      "Dry-run only. Set SUBMIT=1 to sign and submit the royalty pool transaction.",
  }));
} else {
  const signed = await tx.sign.withWallet().complete();
  const txHash = await submitSignedTx(env, signed);
  const outputIndex = await fetchSubmittedOutputIndex(
    env,
    txHash,
    poolAddress,
    datum,
  );
  console.log(jsonStringify({ ...output, txHash, outputIndex }));
}
