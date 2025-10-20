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
    AdminT2tValidateAdmin,
    DoubleRoyaltyPoolDaoV1RequestValidate,
    DoubleRoyaltyPoolDaoV1Validate,
    DoubleRoyaltyPoolDepositValidatePool,
    DoubleRoyaltyPoolPoolValidatePool,
    DoubleRoyaltyPoolRedeemValidatePool,
    DoubleRoyaltyWithdrawPoolDoubleRoyaltyWithdrawPool,
    FactoryT2tValidateFactory,
    FactoryValidateFactory,
    FeeWithdrawerT2tValidateFeeWithdraw,
    GridGridNative,
    InstantOrderBatchWitness,
    InstantOrderInstantOrder,
    LimitOrderBatchWitness,
    LimitOrderLimitOrder,
    PoolT2tValidatePoolT2t,
    RoyaltyPoolDaoV1RequestValidate,
    RoyaltyPoolDaoV1Validate,
    RoyaltyPoolDepositValidate,
    RoyaltyPoolPoolValidatePool,
    RoyaltyPoolRedeemValidate,
    RoyaltyPoolWithdrawRoyaltyRequestValidate,
    SingleRoyaltyWithdrawPoolRoyaltyWithdrawPool,
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
        const royaltyPoolWithdraw = new SingleRoyaltyWithdrawPoolRoyaltyWithdrawPool();
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


        const doubleRoyaltyPool = new DoubleRoyaltyPoolPoolValidatePool();
        const doubleRoyaltyPoolHash = validatorToScriptHash(doubleRoyaltyPool);
        const doubleRoyaltyPoolWithdraw = new DoubleRoyaltyWithdrawPoolDoubleRoyaltyWithdrawPool();
        const doubleRoyaltyPoolWithdrawHash = validatorToScriptHash(doubleRoyaltyPoolWithdraw);
        const doubleRoyaltyDeposit = new DoubleRoyaltyPoolDepositValidatePool();
        const doubleRoyaltyDepositHash = validatorToScriptHash(doubleRoyaltyDeposit);
        const doubleRoyaltyRedeem = new DoubleRoyaltyPoolRedeemValidatePool();
        const doubleRoyaltyRedeemHash = validatorToScriptHash(doubleRoyaltyRedeem);
        const doubleRoyaltyDAOV1Pool = new DoubleRoyaltyPoolDaoV1Validate();
        const doubleRoyaltyDAOV1PoolHash = validatorToScriptHash(doubleRoyaltyDAOV1Pool);
        const doubleRoyaltyDAOV1ActionOrder = new DoubleRoyaltyPoolDaoV1RequestValidate();
        const doubleRoyaltyDAOV1ActionOrderHash = validatorToScriptHash(doubleRoyaltyDAOV1ActionOrder);

        let admins = [
            "0bb1d2db22f9b641f0afe8d8a398279cb778d8f86167500f7e63ebbdc35b4d69",
            "4e8221615500dbf6737b02992610ffeed82da6826dc3d9729febf1d32d766615",
            "ae536160ccec4f078982396125773d509072397e35ed6fab7af2a762ca147318",
            "04e3a257bcb0306c27e796bc16d1b7bde8f2306dc1d6aa344f6043ef48bd7fd8",
            "83d3aa4ccd1c72ff7a27c032394f44b699778b6050290e37fd82bc295f5caf18",
            "a7d30e99673c57638bdb65b5a0554ddee3135131940a41bbd3534b0d4c709506"
        ]

        // Mainnet
        // - b527cc6c83645ff7a3118c15cde03e17b1a29a2203d2c35c408f4bea
        // Mainnet
        // - 43163508e66b0163c569e4b537d40616ec701662453eb2631d59bab8
        let chakraPubKeyHash = "43163508e66b0163c569e4b537d40616ec701662453eb2631d59bab8"

        let testKeyHash = "92afc9fcd474b74b36bf10aa60af2a3a2c6790ea4b764f42293f05bc"

        const degenT2TPool = new PoolT2tValidatePoolT2t();
        const degenT2TPoolHash = validatorToScriptHash(degenT2TPool);
        const degenT2TAdmin = new AdminT2tValidateAdmin(admins, 4n);
        const degenT2TAdminHash = validatorToScriptHash(degenT2TAdmin);
        const degenT2TFactory = new FactoryT2tValidateFactory();
        const degenT2TFactoryHash = validatorToScriptHash(degenT2TFactory);
        const degenT2TFeeWithdraw = new FeeWithdrawerT2tValidateFeeWithdraw(testKeyHash);
        const degenT2TFeeWithdrawHash = validatorToScriptHash(degenT2TFeeWithdraw);

        const instantOrderWitness = new InstantOrderBatchWitness();
        const instantOrderWitnessHash = validatorToScriptHash(instantOrderWitness);

        const instantOrder = new InstantOrderInstantOrder({
            Inline: [
                {
                    ScriptCredential: [instantOrderWitnessHash],
                },
            ],
        });
        const instantOrderHash = validatorToScriptHash(instantOrder);

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
            },
            doubleRoyaltyPool: {
                script: doubleRoyaltyPool,
                hash: doubleRoyaltyPoolHash
            },
            doubleRoyaltyDeposit: {
                script: doubleRoyaltyDeposit,
                hash: doubleRoyaltyDepositHash
            },
            doubleRoyaltyRedeem: {
                script: doubleRoyaltyRedeem,
                hash: doubleRoyaltyRedeemHash
            },
            doubleRoyaltyDAOV1Pool: {
                script: doubleRoyaltyDAOV1Pool,
                hash: doubleRoyaltyDAOV1PoolHash
            },
            degenT2TPool: {
                script: degenT2TPool,
                hash: degenT2TPoolHash
            },
            degenT2TFactory: {
                script: degenT2TFactory,
                hash: degenT2TFactoryHash
            },
            degenT2TAdmin: {
                script: degenT2TAdmin,
                hash: degenT2TAdminHash
            },
            degenT2TFeeWithdraw: {
                script: degenT2TFeeWithdraw,
                hash: degenT2TFeeWithdrawHash
            },
            instantOrder: {
                script: instantOrder,
                hash: instantOrderHash
            },
            instantOrderWitness: {
                script: instantOrderWitness,
                hash: instantOrderWitnessHash
            },
            singleRoyaltyWithdrawPool: {
                script: royaltyPoolWithdraw,
                hash: royaltyPoolWithdrawHash
            },
            doubleRoyaltyWithdrawPool: {
                script: doubleRoyaltyPoolWithdraw,
                hash: doubleRoyaltyPoolWithdrawHash
            },
            doubleRoyaltyDAOV1Request: {
                script: doubleRoyaltyDAOV1ActionOrder,
                hash: doubleRoyaltyDAOV1ActionOrderHash
            }
        }
    }

    async deploy(builtValidators: BuiltValidators): Promise<TxComplete> {
        const ns: Script = scriptFromNative({
            type: 'before',
            slot: 0,
        });
        const lockScript = validatorToAddress("Mainnet", ns);
        const degenFactoryAddr = credentialToRewardAddress("Mainnet", {
            type: "Script",
            hash: builtValidators.degenT2TFactory.hash
        });
        const degenAdminAddress = credentialToRewardAddress("Mainnet", {
            type: "Script",
            hash: builtValidators.degenT2TAdmin.hash
        });
        const degenFeeWithdrawAddress = credentialToRewardAddress("Mainnet", {
            type: "Script",
            hash: builtValidators.degenT2TFeeWithdraw.hash
        });
        const instantOrderWitnessAddr = credentialToRewardAddress("Mainnet", {
            type: "Script",
            hash: builtValidators.instantOrderWitness.hash
        });
        const singleRoyaltyWithdraw = credentialToRewardAddress("Mainnet", {
            type: "Script",
            hash: builtValidators.singleRoyaltyWithdrawPool.hash
        });
        const doubleRoyaltyWithdraw = credentialToRewardAddress("Mainnet", {
            type: "Script",
            hash: builtValidators.doubleRoyaltyWithdrawPool.hash
        });
        const tx = await this.lucid
            .newTx()
            .pay.ToAddressWithData(
                lockScript,
                {kind: "inline", value: "00"},
                undefined,
                builtValidators.degenT2TFactory.script,
            )
            // .pay.ToAddressWithData(
            //     lockScript,
            //     {kind: "inline", value: "00"},
            //     undefined,
            //     builtValidators.degenT2TFeeWithdraw.script,
            // )
            // .pay.ToAddressWithData(
            //     lockScript,
            //     {kind: "inline", value: "00"},
            //     undefined,
            //     builtValidators.singleRoyaltyWithdrawPool.script,
            // )
            // .pay.ToAddressWithData(
            //     lockScript,
            //     {kind: "inline", value: "00"},
            //     undefined,
            //     builtValidators.degenT2TFactory.script,
            // )
            // .pay.ToAddressWithData(
            //     lockScript,
            //     {kind: "inline", value: "00"},
            //     undefined,
            //     builtValidators.doubleRoyaltyPool.script,
            // )
            // .pay.ToAddressWithData(
            //     lockScript,
            //     {kind: "inline", value: "00"},
            //     undefined,
            //     builtValidators.doubleRoyaltyWithdrawPool.script,
            // )
            // .registerStake(degenFeeWithdrawAddress)
            .registerStake(degenFactoryAddr)
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