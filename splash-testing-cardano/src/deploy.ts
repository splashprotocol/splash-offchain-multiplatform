import {
    credentialToRewardAddress,
    Lucid,
    Script,
    scriptFromNative,
    TxComplete,
    validatorToAddress
} from "@lucid-evolution/lucid";
import {validatorToScriptHash} from "@lucid-evolution/utils";
import {BuiltValidators, DeployedValidators, ScriptNames} from "./types.ts";
import {getLucid} from "./lucid.ts";
import {generateConfigJson} from "./config.ts";
import {setupWallet} from "./wallet.ts";
import {
    FactoryValidateFactory,
    GridGridNative,
    LimitOrderBatchWitness,
    LimitOrderLimitOrder,
    RoyaltyPoolDaoV1RequestValidate,
    RoyaltyPoolDaoV1Validate,
    RoyaltyPoolDepositValidate,
    RoyaltyPoolFeeSwitchValidate,
    RoyaltyPoolPoolValidatePool,
    RoyaltyPoolRedeemValidate, RoyaltyPoolRoyaltyWithdrawPoolValidate, RoyaltyPoolWithdrawRoyaltyRequestValidate,
} from "../plutus.ts";

export class Deployment {
    lucid: Lucid;

    constructor(lucid: Lucid) {
        this.lucid = lucid;
    }

    build(): BuiltValidators {
        const witnessScript = new LimitOrderBatchWitness();
        const witnessScriptHash = validatorToScriptHash(witnessScript);
        const orderScript = new LimitOrderLimitOrder({
            Inline: [
                {
                    ScriptCredential: [witnessScriptHash],
                },
            ],
        });
        const orderScriptHash = validatorToScriptHash(orderScript);
        const gridOrderNativeScript = new GridGridNative();
        const gridOrderNativeHash = validatorToScriptHash(gridOrderNativeScript);
        const royaltyPool = new RoyaltyPoolPoolValidatePool();
        const royaltyPoolHash = validatorToScriptHash(royaltyPool);
        const royaltyPoolWithdraw = new RoyaltyPoolRoyaltyWithdrawPoolValidate();
        const royaltyPoolWithdrawHash = validatorToScriptHash(royaltyPoolWithdraw);
        const royaltyWithdrawRequest = new RoyaltyPoolWithdrawRoyaltyRequestValidate();
        const royaltyWithdrawRequestHash = validatorToScriptHash(royaltyWithdrawRequest);
        const royaltyDeposit = new RoyaltyPoolDepositValidate();
        const royaltyDepositHash = validatorToScriptHash(royaltyDeposit);
        const royaltyRedeem = new RoyaltyPoolRedeemValidate();
        const royaltyRedeemHash = validatorToScriptHash(royaltyRedeem);
        const royaltyDAOV1Pool = new RoyaltyPoolDaoV1Validate();
        const royaltyDAOV1PoolHash = validatorToScriptHash(royaltyDAOV1Pool);
        const royaltyDAOV1Request = new RoyaltyPoolDaoV1RequestValidate();
        const royaltyDAOV1RequestHash = validatorToScriptHash(royaltyDAOV1Request);
        const degenFactory = new FactoryValidateFactory();
        const degenFactoryHash = validatorToScriptHash(degenFactory);
        return {
            royaltyPool: {
                script: royaltyPool,
                hash: royaltyPoolHash,
            },
            royaltyWithdrawPool: {
                script: royaltyPoolWithdraw,
                hash: royaltyPoolWithdrawHash,
            },
            royaltyWithdrawRequest: {
                script: royaltyWithdrawRequest,
                hash: royaltyWithdrawRequestHash,
            },
            royaltyDeposit: {
                script: royaltyDeposit,
                hash: royaltyDepositHash,
            },
            royaltyRedeem: {
                script: royaltyRedeem,
                hash: royaltyRedeemHash,
            },
            royaltyDAOV1Pool: {
                script: royaltyDAOV1Pool,
                hash: royaltyDAOV1PoolHash,
            },
            royaltyDAOV1Request: {
                script: royaltyDAOV1Request,
                hash: royaltyDAOV1RequestHash,
            },
            factory: {
                script: degenFactory,
                hash: degenFactoryHash,
            }
        }
    }

    async deploy(builtValidators: BuiltValidators): Promise<TxComplete> {
        const ns: Script = scriptFromNative({
            type: 'before',
            slot: 0,
        });
        const lockScript = validatorToAddress("Preprod", ns);
        const daoV1Addr = credentialToRewardAddress("Preprod", {
            type: "Script",
            hash: builtValidators.royaltyDAOV1Pool.hash
        });
        const royaltyWithdrawV1Addr = credentialToRewardAddress("Preprod", {
            type: "Script",
            hash: builtValidators.royaltyWithdrawPool.hash
        });
        const degenFactoryAddr = credentialToRewardAddress("Preprod", {
            type: "Script",
            hash: builtValidators.factory.hash
        });
        const tx = await this.lucid
            .newTx()
            .pay.ToAddressWithData(
                lockScript,
                {kind: "inline", value: "00"},
                undefined,
                builtValidators.royaltyDAOV1Pool.script,
            )
            .pay.ToAddressWithData(
                lockScript,
                {kind: "inline", value: "00"},
                undefined,
                builtValidators.factory.script,
            )
            .pay.ToAddressWithData(
                lockScript,
                {kind: "inline", value: "00"},
                undefined,
                builtValidators.royaltyDAOV1Request.script,
            )
            // .pay.ToAddressWithData(
            //     lockScript,
            //     {kind: "inline", value: "00"},
            //     undefined,
            //     builtValidators.royaltyWithdrawPool.script,
            // )
            // .pay.ToAddressWithData(
            //     lockScript,
            //     {kind: "inline", value: "00"},
            //     undefined,
            //     builtValidators.royaltyWithdrawRequest.script,
            // )
            .registerStake(degenFactoryAddr)
            .registerStake(daoV1Addr)
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
    const deployTxId = await (await deployTx.sign.withWallet().complete()).submit();
    console.log('Deployment Tx ID:', deployTxId);
    // Here we need to wait until contracts are deployed
    await lucid.awaitTx(deployTxId);
    const deployedValidators = await getDeployedValidators(lucid, builtValidators, deployTxId);
    await generateConfigJson(deployedValidators);
}

main();