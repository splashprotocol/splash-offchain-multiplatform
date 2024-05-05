import { Lucid, Script, TxComplete } from "https://deno.land/x/lucid@0.10.7/mod.ts";
import { BuiltValidators, DeployedValidators, ScriptNames } from "./types.ts";
import { BalanceContract, BalancedepositContract, BalanceredeemContract, FeeswitchContract, FeeswitchdepositContract, FeeswitchredeemContract, LimitOrderBatchWitness, LimitOrderLimitOrder } from "./../plutus.ts";
import { getLucid } from "./lucid.ts";
import { generateConfigJson } from "./config.ts";
import { setupWallet } from "./wallet.ts";

export class Deployment {
  lucid: Lucid;

  constructor(lucid: Lucid) {
    this.lucid = lucid;
  }

  build(): BuiltValidators {
    const witnessScript = new LimitOrderBatchWitness();
    const witnessScriptHash = this.lucid.utils.validatorToScriptHash(witnessScript);
    const orderScript = new LimitOrderLimitOrder({
      Inline: [
        {
          ScriptCredential: [witnessScriptHash],
        },
      ],
    });
    const orderScriptHash = this.lucid.utils.validatorToScriptHash(orderScript);
    const balancePoolScript = new BalanceContract();
    const balancePoolScriptHash = this.lucid.utils.validatorToScriptHash(balancePoolScript);
    const balancePoolDeposit = new BalancedepositContract();
    const balanceDepositScriptHash = this.lucid.utils.validatorToScriptHash(balancePoolDeposit);
    const balancePoolRedeem = new BalanceredeemContract();
    const balanceRedeemScriptHash = this.lucid.utils.validatorToScriptHash(balancePoolRedeem);
    const feeSwitchPool = new FeeswitchContract();
    const feeSwitchPoolScriptHash = this.lucid.utils.validatorToScriptHash(feeSwitchPool);
    const feeSwitchPoolDeposit = new FeeswitchdepositContract();
    const feeSwitchPoolDepositScriptHash = this.lucid.utils.validatorToScriptHash(feeSwitchPoolDeposit);
    const feeSwitchPoolRedeem = new FeeswitchredeemContract();
    const feeSwitchPoolRedeemScriptHash = this.lucid.utils.validatorToScriptHash(feeSwitchPoolRedeem);
    return {
      // limitOrder: {
      //   script: orderScript,
      //   hash: orderScriptHash,
      // },
      // limitOrderWitness: {
      //   script: witnessScript,
      //   hash: witnessScriptHash,
      // },
      balancePool: {
        script: balancePoolScript,
        hash: balancePoolScriptHash,
      },
      balanceDeposit: {
        script: balancePoolDeposit,
        hash: balanceDepositScriptHash,
      },
      balanceRedeem: { // 2
        script: balancePoolRedeem,
        hash: balanceRedeemScriptHash,
      },
      // feeSwitchPool: {
      //   script: feeSwitchPool,
      //   hash: feeSwitchPoolScriptHash,
      // },
      // feeSwitchRedeem: { // 3
      //   script: feeSwitchPoolRedeem,
      //   hash: feeSwitchPoolRedeemScriptHash,
      // },
      // feeSwitchDeposit: {
      //   script: feeSwitchPoolDeposit,
      //   hash: feeSwitchPoolDepositScriptHash,
      // }
    }
  }

  async deploy(builtValidators: BuiltValidators): Promise<TxComplete> {
    const ns: Script = this.lucid.utils.nativeScriptFromJson({
      type: 'before',
      slot: 0,
    });
    const lockScript = this.lucid.utils.validatorToAddress(ns);
    // const witnessRewardAddress = this.lucid.utils.credentialToRewardAddress({
    //   type: "Script",
    //   hash: builtValidators.limitOrderWitness.hash
    // });
    const tx = await this.lucid
      .newTx()
      // .payToAddressWithData(
      //   lockScript,
      //   { scriptRef: builtValidators.limitOrder.script },
      //   {},
      // )
      // .payToAddressWithData(
      //   lockScript,
      //   { scriptRef: builtValidators.limitOrderWitness.script },
      //   {},
      // )
      .payToAddressWithData(
        lockScript,
        { scriptRef: builtValidators.balancePool.script },
        {},
      )
      .payToAddressWithData(
        lockScript,
        { scriptRef: builtValidators.balanceDeposit.script },
        {},
      )
      .payToAddressWithData(   // 2
        lockScript,
        { scriptRef: builtValidators.balanceRedeem.script },
        {},
      )
      // .payToAddressWithData(
      //   lockScript,
      //   { scriptRef: builtValidators.feeSwitchPool.script },
      //   {},
      // )
      // .payToAddressWithData(          // 3
      //   lockScript,
      //   { scriptRef: builtValidators.feeSwitchDeposit.script },
      //   {},
      // )
      // .payToAddressWithData(
      //   lockScript,
      //   { scriptRef: builtValidators.feeSwitchRedeem.script },
      //   {},
      // )
      //.registerStake(witnessRewardAddress)
      .complete();

    return tx;
  }
}

async function getDeployedValidators(
  lucid: Lucid,
  builtValidators: BuiltValidators,
  deployedValidatorsTxId: string,
): Promise<DeployedValidators> {
  try {
    const builtValidatorsKeys = Object.keys(builtValidators) as ScriptNames[];
    const utxosByOutRefsRequest = builtValidatorsKeys.map((_, index) => ({
      txHash: deployedValidatorsTxId,
      outputIndex: index,
    }));

    const validatorsUtxos = await lucid.utxosByOutRef(utxosByOutRefsRequest);

    return builtValidatorsKeys.reduce((
      acc,
      key: ScriptNames,
      index,
    ) => {
      const { script, hash } = builtValidators[key];
      const referenceUtxo = validatorsUtxos[index];

      return {
        [key]: {
          script,
          hash,
          referenceUtxo,
        },
        ...acc,
      };
    }, {} as DeployedValidators);
  } catch (error) {
    console.error('Failed to get deployed validators:', error);
    throw error;
  }
}

async function main() {
  const lucid = await getLucid();
  await setupWallet(lucid);
  const deployment = new Deployment(lucid);
  const builtValidators = deployment.build();
  const deployTx = await deployment.deploy(builtValidators);
  const deployTxId = await (await deployTx.sign().complete()).submit();
  console.log('Deployment Tx ID:', deployTxId);
  // Here we need to wait until contracts are deployed
  await lucid.awaitTx(deployTxId);
  const deployedValidators = await getDeployedValidators(lucid, builtValidators, deployTxId);
  await generateConfigJson(deployedValidators);
}

main();