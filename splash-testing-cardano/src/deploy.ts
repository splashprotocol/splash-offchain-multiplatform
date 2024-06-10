import { Lucid, Script, TxComplete } from "https://deno.land/x/lucid@0.10.7/mod.ts";
import { BuiltValidators, DeployedValidators, ScriptNames } from "./types.ts";
import { LimitOrderBatchWitness, LimitOrderLimitOrder, PoolT2tExactValidateStablePoolTransitionT2tExact, StabledepositContract, StableredeemContract } from "./../plutus.ts";
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
    const stablePoolT2T = new PoolT2tExactValidateStablePoolTransitionT2tExact('00000000000000000000000000000000000000000000000000000000')
    const stablePoolT2TScriptHash = this.lucid.utils.validatorToScriptHash(stablePoolT2T);
    const stablePoolT2TDeposit = new StabledepositContract()
    const stablePoolT2TDepositScriptHash = this.lucid.utils.validatorToScriptHash(stablePoolT2TDeposit);
    const stablePoolT2TRedeem = new StableredeemContract()
    const stablePoolT2TRedeemScriptHash = this.lucid.utils.validatorToScriptHash(stablePoolT2TRedeem);
    return {
      limitOrder: {
        script: orderScript,
        hash: orderScriptHash,
      },
      limitOrderWitness: {
        script: witnessScript,
        hash: witnessScriptHash,
      },
      stablePoolT2T: {
        script: stablePoolT2T,
        hash: stablePoolT2TScriptHash,
      },
      RedeemT2tStableRedeemT2t: {
        script: stablePoolT2TRedeem,
        hash: stablePoolT2TRedeemScriptHash,
      },
      DepositT2t2tStableDepositT2t: {
        script: stablePoolT2TDeposit,
        hash: stablePoolT2TDepositScriptHash,
      }
    }
  }

  async deploy(builtValidators: BuiltValidators): Promise<TxComplete> {
    const ns: Script = this.lucid.utils.nativeScriptFromJson({
      type: 'before',
      slot: 0,
    });
    const lockScript = this.lucid.utils.validatorToAddress(ns);
    const witnessRewardAddress = this.lucid.utils.credentialToRewardAddress({
      type: "Script",
      hash: builtValidators.limitOrderWitness.hash
    });
    const tx = await this.lucid
      .newTx()
      .payToAddressWithData(
        lockScript,
        { scriptRef: builtValidators.limitOrder.script },
        {},
      )
      .payToAddressWithData(
        lockScript,
        { scriptRef: builtValidators.limitOrderWitness.script },
        {},
      )
      .payToAddressWithData(
        lockScript,
        { scriptRef: builtValidators.stablePoolT2T.script },
        {},
      )
      .payToAddressWithData(
        lockScript,
        { scriptRef: builtValidators.DepositT2t2tStableDepositT2t.script },
        {},
      )
      .payToAddressWithData(
        lockScript,
        { scriptRef: builtValidators.RedeemT2tStableRedeemT2t.script },
        {},
      )
      .registerStake(witnessRewardAddress)
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