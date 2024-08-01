import {Lucid, Script, TxComplete} from "https://deno.land/x/lucid@0.10.7/mod.ts";
import {BuiltValidators, DeployedValidators, ScriptNames} from "./types.ts";
import {
  AdminValidateAdmin, AssetsMintIdentifier,
  FactoryValidateFactory, PoolsAmm,
  PoolValidatePool
} from "./../plutus.ts";
import {getLucid} from "./lucid.ts";
import {generateConfigJson} from "./config.ts";
import {setupWallet} from "./wallet.ts";

export class Deployment {
  lucid: Lucid;

  constructor(lucid: Lucid) {
    this.lucid = lucid;
  }

  build(): BuiltValidators {
    const adminScript = new AdminValidateAdmin(
        [
          "3703634e3e34c0e7fd9a7348ad213f9272279535c73f4ec00efe00bd",
          "f3a12554ca0ccd1a220b1de839a1940bc1006f08768b636fa98c66c9",
          "518a9c32deedc0b82604972692a2b7eb6c10b020d77c3c72e764b156",
          "68aa59a87dbdbf8f78386dc6b83e63149d8c13939a1b0cda39707f9a",
          "1826edf8011dc6a084214b87a4ff690665692f4243e5bb5ff5aad536",
          "d350803d45e327f8808469d5dde9e0f6fdc6e6637d85ed44cc37a12c"
        ], 5n
    );
    const adminScriptHash = this.lucid.utils.validatorToScriptHash(adminScript);
    const poolScript = new PoolValidatePool();
    const poolHash = this.lucid.utils.validatorToScriptHash(poolScript);
    const factoryScript = new FactoryValidateFactory();
    const factoryHash = this.lucid.utils.validatorToScriptHash(factoryScript);
    const assetsScript = new AssetsMintIdentifier();
    const assetsHash = this.lucid.utils.validatorToScriptHash(assetsScript);
    const ammPoolScript = new PoolsAmm();
    const ammPoolHash = this.lucid.utils.validatorToScriptHash(ammPoolScript);
    return {
      admin: {
        script: adminScript,
        hash: adminScriptHash,
      },
      pool: {
        script: poolScript,
        hash: poolHash,
      },
      factory: {
        script: factoryScript,
        hash: factoryHash,
      },
      ammPool: {
        script: ammPoolScript,
        hash: ammPoolHash
      }
    }
  }

  async deploy(builtValidators: BuiltValidators): Promise<TxComplete> {
    const ns: Script = this.lucid.utils.nativeScriptFromJson({
      type: 'before',
      slot: 0,
    });
    const lockScript = this.lucid.utils.validatorToAddress(ns);
    const factoryRewardAddress = this.lucid.utils.credentialToRewardAddress({
      type: "Script",
      hash: builtValidators.factory.hash
    });
    const adminRewardAddress = this.lucid.utils.credentialToRewardAddress({
      type: "Script",
      hash: builtValidators.admin.hash
    });
    const tx = await this.lucid
        .newTx()
        .payToAddressWithData(
            lockScript,
            {scriptRef: builtValidators.pool.script},
            {},
        )
        .payToAddressWithData(
            lockScript,
            {scriptRef: builtValidators.ammPool.script},
            {},
        )
        // .registerStake(factoryRewardAddress)
        // .registerStake(adminRewardAddress)
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
      const {script, hash} = builtValidators[key];
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