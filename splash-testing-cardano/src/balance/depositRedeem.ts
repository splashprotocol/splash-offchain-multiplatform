import { Config, getConfig } from "../config.ts";
import { getLucid } from "../lucid.ts";
import { Asset, BuiltValidator, BuiltValidators } from "../types.ts";
import { setupWallet } from "../wallet.ts";
import { PubKeyHash } from "../types.ts";
import { getUtxoWithToken, getUtxoWithAda } from "./balancePool.ts";
import { Data, Datum, Lucid, TxComplete } from "https://deno.land/x/lucid@0.10.7/mod.ts";
import { BalancedepositContract, BalanceredeemContract } from "../../plutus.ts";
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

const poolNftCS = "62d20c0f9b1486f2b27a89d76c2c0992bb3933720d2ba5e42f9eb065";
const poolNftTN = "6e6674";

const encodedTestB  = "7465737443";
const TokenBCS = "4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26";

const TokenLQTn = "6c71";
const TokenLQCs = "8dcb9153bddbac9558d41c8e3b103f858ecfda1381b7b15cbd9ab25e";

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
  }, BalancedepositContract.conf);
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
  }, BalanceredeemContract.conf);
}

const stringifyBigIntReviewer = (_: any, value: any) =>
  typeof value === 'bigint'
    ? { value: value.toString(), _bigint: true }
    : value;

async function deposit(lucid: Lucid, validator: BuiltValidator, conf: DepositConf): Promise<TxComplete> {

  const utxos = (await lucid.wallet.getUtxos());

  const boxWithToken = await getUtxoWithToken(utxos, encodedTestB);
  const boxWithAda   = await getUtxoWithAda(utxos)

  const orderAddress = lucid.utils.credentialToAddress(
    { hash: validator.hash, type: 'Script' },
  );
  const lovelaceTotal = conf.exFee;
  const depositedValue = conf.x[0].policy == ""
    ? { lovelace: lovelaceTotal + conf.x[1], [asUnit(conf.y[0])]: conf.y[1] } : conf.y[0].policy == ""
    ? { lovelace: lovelaceTotal + conf.y[1], [asUnit(conf.x[0])]: conf.x[1] } : { lovelace: lovelaceTotal, [asUnit(conf.x[0])]: conf.x[1], [asUnit(conf.y[0])]: conf.y[1] };
  
  console.log(`ada utxo: ${JSON.stringify(boxWithAda!, stringifyBigIntReviewer)}`)

  console.log(`boxWithToken: ${JSON.stringify(boxWithToken!, stringifyBigIntReviewer)}`)

  console.log(`depositedValue: ${JSON.stringify(depositedValue, stringifyBigIntReviewer)}`)

  const tx = await lucid.newTx()
    .collectFrom([boxWithToken!, boxWithAda!])
    .payToContract(orderAddress, { inline: buildDepositDatum(conf) }, depositedValue)
    .complete();
  const txId = await (await tx.sign().complete()).submit();
  return txId
}

async function redeem(lucid: Lucid, validator: BuiltValidator, conf: RedeemConf): Promise<TxComplete> {
  const orderAddress = lucid.utils.credentialToAddress(
    { hash: validator.hash, type: 'Script' },
  );

  const utxos = (await lucid.wallet.getUtxos());

  console.log(`utxos: ${JSON.stringify(utxos, stringifyBigIntReviewer)}`);


  const boxWithToken = await getUtxoWithToken(utxos, TokenLQCs);
  const boxWithAda   = await getUtxoWithAda(utxos)

  // should bew the same or greather than value in rust config
  const collateralAda = 1_500_000n

  const lovelaceTotal = conf.exFee + 1_500_000n;
  const depositedValue = { lovelace: lovelaceTotal, [asUnit(conf.lq[0])]: conf.lq[1] };

  console.log(`ada utxo: ${JSON.stringify(boxWithAda!, stringifyBigIntReviewer)}`)

  console.log(`boxWithToken: ${JSON.stringify(boxWithToken!, stringifyBigIntReviewer)}`)

  console.log(`depositedValue: ${JSON.stringify(depositedValue, stringifyBigIntReviewer)}`)

  const tx = await lucid.newTx()
    .collectFrom([boxWithAda!, boxWithToken!])
    .payToContract(orderAddress, { inline: buildRedeemDatum(conf) }, depositedValue)
    .complete();

  const txId = await (await tx.sign().complete()).submit();
  return txId
}

async function doDeposit(lucid: Lucid, conf: Config<BuiltValidators>) {

  const myAddr = await lucid.wallet.address();
  
  const depositConf: DepositConf = {
    poolnft: {
      policy: poolNftCS,
      name: poolNftTN,
    },
    x: [
      {
        policy: "",
        name: "",
      }, 1000_000_000n
    ],
    y: [
      {
        policy: TokenBCS,
        name: encodedTestB,
      }, 1000_000_000n
    ],
    lq: {
      policy: TokenLQCs,
      name: TokenLQTn,
    },
    exFee: 2_000_000n,
    rewardPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
    stakePkh: null,
    collateralAda: 5_000_000n
  };

  const depositTxId = await deposit(lucid, conf.validators!.balanceDeposit, depositConf);

  console.log(`txId: ${depositTxId}`);
}

async function doRedeem(lucid: Lucid, conf: Config<BuiltValidators>) {

  const myAddr = await lucid.wallet.address();
  
  const redeemConf: RedeemConf = {
    poolnft: {
      policy: poolNftCS,
      name: poolNftTN,
    },
    x: {
      policy: "",
      name: "",
    },
    y: {
      policy: TokenBCS,
      name: encodedTestB,
    },
    lq: [{
      policy: TokenLQCs,
      name: TokenLQTn,
    }, 3_000_000n],
    exFee: 2_000_000n,
    rewardPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
    stakePkh: null
  };

  const redeemTxId = await redeem(lucid, conf.validators!.balanceRedeem, redeemConf);

  console.log(`txId: ${redeemTxId}`);
}

async function main() {

  const lucid = await getLucid();
  await setupWallet(lucid);
  const conf = await getConfig<BuiltValidators>();

  //doDeposit(lucid, conf);

  doRedeem(lucid, conf);
}

//main();
