import { getConfig } from "../config.ts";
import { getLucid } from "../lucid.ts";
import { Asset, BuiltValidator, BuiltValidators } from "../types.ts";
import { setupWallet } from "../wallet.ts";
import { PubKeyHash } from "../types.ts";
import { Data, Datum, Lucid, TxComplete } from "@lucid-evolution/lucid";
import { StabledepositContract, StableredeemContract } from "../../plutus.ts";
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
  }, StabledepositContract.conf);
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
  }, StableredeemContract.conf);
}

export function deposit(lucid: Lucid, validator: BuiltValidator, conf: DepositConf): Promise<TxComplete> {
  const orderAddress = lucid.utils.credentialToAddress(
    { hash: validator.hash, type: 'Script' },
  );
  const lovelaceTotal = conf.exFee;
  const depositedValue = conf.x[0].policy == ""
    ? { lovelace: lovelaceTotal + conf.x[1], [asUnit(conf.y[0])]: conf.y[1] } : conf.y[0].policy == ""
    ? { lovelace: lovelaceTotal + conf.y[1], [asUnit(conf.x[0])]: conf.x[1] } : { lovelace: lovelaceTotal, [asUnit(conf.x[0])]: conf.x[1], [asUnit(conf.y[0])]: conf.y[1] };
  const tx = lucid.newTx().payToContract(orderAddress, { inline: buildDepositDatum(conf) }, depositedValue);
  return tx.complete();
}

export function redeem(lucid: Lucid, validator: BuiltValidator, conf: RedeemConf): Promise<TxComplete> {
  const orderAddress = lucid.utils.credentialToAddress(
    { hash: validator.hash, type: 'Script' },
  );
  const lovelaceTotal = conf.exFee;
  const depositedValue = { lovelace: lovelaceTotal, [asUnit(conf.lq[0])]: conf.lq[1] };
  const tx = lucid.newTx().payToContract(orderAddress, { inline: buildRedeemDatum(conf) }, depositedValue);
  return tx.complete();
}

const lucid = await getLucid();
await setupWallet(lucid);
const conf = await getConfig<BuiltValidators>();