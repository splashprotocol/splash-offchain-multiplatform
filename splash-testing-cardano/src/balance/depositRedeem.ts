import { getConfig } from "../config.ts";
import { getLucid } from "../lucid.ts";
import { Asset, BuiltValidator, BuiltValidators } from "../types.ts";
import { setupWallet } from "../wallet.ts";
import { PubKeyHash } from "../types.ts";
import { credentialToAddress, Data, Datum, Lucid, LucidEvolution, TxComplete } from "@lucid-evolution/lucid";
import { DepositDeposit, RedeemRedeem, RoyaltyPoolDepositValidate, RoyaltyPoolRedeemValidate } from "../../plutus.ts";
import { asUnit } from "../types.ts";

export type DepositConf = {
  poolnft: Asset,
  x: [Asset, bigint],
  y: [Asset, bigint],
  lq: Asset,
  exFee: bigint,
  rewardPkh: PubKeyHash,
  stakePkh: PubKeyHash | null,
  collateralAda: bigint
};

function buildDepositDatum(conf: DepositConf): Datum {
  return Data.to({
    poolnft: conf.poolnft,
    x: conf.x[0],
    y: conf.y[0],
    lq: conf.lq,
    exFee: conf.exFee,
    rewardPkh: conf.rewardPkh,
    stakePkh: conf.stakePkh,
    collateralAda: conf.collateralAda,
  }, RoyaltyPoolDepositValidate.conf);
}

export type RedeemConf = {
  poolnft: Asset,
  x: Asset,
  y: Asset,
  lq: [Asset, bigint],
  exFee: bigint,
  rewardPkh: PubKeyHash,
  stakePkh: PubKeyHash | null,
};

function buildRedeemDatum(conf: RedeemConf): Datum {
  return Data.to({
    poolnft: conf.poolnft,
    x: conf.x,
    y: conf.y,
    lq: conf.lq[0],
    exFee: conf.exFee,
    rewardPkh: conf.rewardPkh,
    stakePkh: conf.stakePkh,
  }, RoyaltyPoolRedeemValidate.conf);
}

export function deposit(lucid: LucidEvolution, validator: BuiltValidator, conf: DepositConf): Promise<TxComplete> {
  const orderAddress = credentialToAddress(
    "Preprod",
    { hash: validator.hash, type: 'Script' },
  );
  const lovelaceTotal = conf.exFee;
  const depositedValue = conf.x[0].policy == ""
    ? { lovelace: lovelaceTotal + conf.x[1], [asUnit(conf.y[0])]: conf.y[1] } : conf.y[0].policy == ""
    ? { lovelace: lovelaceTotal + conf.y[1], [asUnit(conf.x[0])]: conf.x[1] } : { lovelace: lovelaceTotal, [asUnit(conf.x[0])]: conf.x[1], [asUnit(conf.y[0])]: conf.y[1] };
  console.log(`depositedValue: ${depositedValue}`)
  const tx = lucid.newTx().pay.ToContract(orderAddress, { kind: "inline",  value: buildDepositDatum(conf) }, depositedValue);
  return tx.complete();
}

export function redeem(lucid: LucidEvolution, validator: BuiltValidator, conf: RedeemConf): Promise<TxComplete> {
  const orderAddress = credentialToAddress(
     "Preprod",
    { hash: validator.hash, type: 'Script' },
  );
  const lovelaceTotal = conf.exFee + 3_000_000n;
  const depositedValue = { lovelace: lovelaceTotal, [asUnit(conf.lq[0])]: conf.lq[1] };
  const tx = lucid.newTx().pay.ToContract(orderAddress, { kind: "inline",  value: buildRedeemDatum(conf) }, depositedValue);
  return tx.complete();
}