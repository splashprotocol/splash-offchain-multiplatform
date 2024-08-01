import {Lucid, Data, TxHash, Assets, Unit} from "https://deno.land/x/lucid@0.10.7/mod.ts";
import {getLucid} from "../lucid.ts";
import {setupWallet} from "../wallet.ts";
import {Config, getConfig} from "../config.ts";
import {Asset, asUnit, BuiltValidators, PubKeyHash, MintingInfo, DegenPoolInfo} from "../types.ts";
import {PoolValidatePool, AssetsMintIdentifier, PoolsAmm, FactoryValidateFactory} from "../../plutus.ts";
import {all, ConfigOptions, create, FormatOptions} from 'npm:mathjs';
import {blake2b} from 'npm:hash-wasm';
import {encoder} from 'npm:js-encoding-utils';

// Constants. Allowed to change

const AmmPoolFee = 99000n
const TreasuryFee = 100n

// Constants. Do not change
const InitialLqCap = BigInt(0x7fffffffffffffff);
const DeployFeeWithTxFee = 310000000n
const NftEmission = 1n

const poolStakeHash = "b2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b07"

const tokenMintHash = "1e5cf9efd35a9d2bfee236880632207058ff07bafdeb7b69c0ee0163"
const tokenMintScript = "59027859027501000032323232323232323222325333006323232323232323232323253330113370e9000180800409919191919191919299980e180f80109919299980d99b88480000044c8c94ccc074cdc3a40006038002264646464a66604200420022940cdc380300519b8f006001379000264660026eb8c00cc070c00cc070028cc004dd998119812180e0051bb337500104466e2800800458c8cc004004048894ccc0840045300103d87a80001323253330203375e600a603c004018266e952000330240024bd7009980200200098128011811800918108008a51375a60380046eb8c06800458c074004c8c8c94ccc068cdc3a4004002297adef6c6013756603e60300046030002646600200200444a66603a0022980103d87a8000132323232533301e3371e018004266e95200033022374c00297ae01330060060033756603e0066eb8c074008c084008c07c004c8cc004004020894ccc07000452f5bded8c0264646464a66603a66e3d221000021003133021337606ea4008dd3000998030030019bab301e003375c60380046040004603c0026eb4c06c004c06c008c064004c044040dd7180b80098078040b1bab30150013015001301400130130013012002375860200026010006601c002601c0046018002600800429309b2b19299980319b87480000044c8c8c8c94ccc034c0400084c92632533300b3370e90000008991919192999809180a80109924c64a66602066e1d20000011323253330153018002149858dd7180b00098070020b18070018b1bad301300130130023011001300900416300900316375a601c002601c004601800260080062c60080044600a6ea80048c00cdd5000ab9a5573aaae7955cfaba05742ae881"

const treasuryAddress: PubKeyHash = "75c4570eb625ae881b32a34c52b159f6f3f3f2c7aaabf5bac4688133"
const daoAddress = "e5619f78d716cc24ea924e4412d83353ae390edd9905ecc6d5c85c2b"

const lovelaceStr = "lovelace"
const mathConf: ConfigOptions = {
    epsilon: 1e-24,
    matrix: 'Matrix',
    number: 'BigNumber',
    precision: 64,
};
const math = create(all, mathConf);

async function findSuitableDegenPools2AMMCast(degenUtxos: UTxO[]) {
    let degenPoolsWithDatums = degenUtxos.map((degenUtxo: UTxO) => {
            return {
                degenDatum: Data.from(
                    degenUtxo.datum,
                    PoolValidatePool.inputDatum
                ),
                degenValue: degenUtxo.assets,
                txHash: degenUtxo.txHash,
                outputIndex: degenUtxo.outputIndex,
                utxo: degenUtxo
            }
        }
    )
    console.log(`All degen pools qty is: ${degenPoolsWithDatums.length}`)

    let suitableDegenPools = degenPoolsWithDatums.filter((degenPoolInfo) => {
        degenPoolInfo.degenValue[lovelaceStr] >= degenPoolInfo.degenDatum.adaCapThr
    })
    console.log(`Suitable degen pools qty to be casted into amm pools is: ${suitableDegenPools.length}`)
    return suitableDegenPools;
}

async function calculateTN(txId: TxHash, outputIndex: number, emission: BigInt) {

    const dataIdx = Data.to(BigInt(outputIndex), Data.Integer())
    const dataEmission = Data.to(emission, Data.Integer())

    return blake2b(
        Uint8Array.from([
            ...encoder.stringToArrayBuffer(txId),
            ...encoder.stringToArrayBuffer(dataIdx),
            ...encoder.stringToArrayBuffer(dataEmission),
        ]),
        224,
    )
}

async function computeMintingData(degenPool: DegenPoolInfo, emission: BigInt): Promise<MintingInfo> {

    const tn = await calculateTN(degenPool.txHash, degenPool.outputIndex, emission)
    const asset: Asset = {
        policy: tokenMintHash,
        name: tn
    }
    const mintingValue: Record<Unit | "lovelace", bigint> = {
        [asUnit(asset)]: emission
    }
    const mintingRedeemer = Data.to({
        inputOref: {
            transactionId: {hash: degenPool.txHash},
            outputIndex: BigInt(degenPool.outputIndex)
        },
        emission: emission
    }, AssetsMintIdentifier.mintInfo)

    return {
        asset,
        mintingValue,
        mintingRedeemer
    }
}

async function degen2ammPool(degenPool: DegenPoolInfo, lucid: Lucid, config: Config<BuiltValidators>) {

    console.log(`Going to cast ${degenPool.txHash}#${degenPool.outputIndex} pool into amm`)

    const tokenXValue = degenPool.degenValue[lovelaceStr]
    const tokenYValue = degenPool.degenValue[asUnit(degenPool.degenDatum.assetY)]

    console.log(`Pool lovelace: ${tokenXValue}, tokenY: ${tokenYValue}`)

    const lq2burn = BigInt(math.nthRoot(math.evaluate(
        `${tokenXValue} * ${tokenYValue}`,
    ), 2).floor().toFixed())

    console.log(`lq to burn: ${lq2burn}`)

    const tokensMintingQty =
        {
            type: "PlutusV2",
            script: tokenMintScript
        }

    const nftMintingData= await computeMintingData(degenPool, NftEmission)
    const lqMintingData = await computeMintingData(degenPool, InitialLqCap - lq2burn)

    const ammPoolDatum = Data.to({
        poolnft: nftMintingData.asset,
        poolx: degenPool.degenDatum.assetX,
        poolY: degenPool.degenDatum.assetY,
        poolLq: lqMintingData.asset,
        feenum: AmmPoolFee,
        treasuryFee: TreasuryFee,
        treasuryx: BigInt(0),
        treasuryy: BigInt(0),
        daoPolicy: [{
            Inline: [{ ScriptCredential: [daoAddress] }]
        }],
        treasuryAddress: treasuryAddress
    }, PoolsAmm.conf)

    const poolAddress = lucid.utils.credentialToAddress(
        {hash: config.validators!.ammPool.hash, type: 'Script'},
    );

    const poolDepositedValue = {
        lovelace: BigInt(tokenXValue - DeployFeeWithTxFee),
        [asUnit(degenPool.degenDatum.assetY)]: BigInt(tokenYValue),
        [asUnit(nftMintingData.asset)]: NftEmission,
        [asUnit(lqMintingData.asset)]: InitialLqCap - lq2burn
    }

    const degenRedeemer = Data.to(
        {
            poolInIx: BigInt(0),
            action: "FactoryAction"
        }, PoolValidatePool.redeemer
    )

    const tx = await lucid.newTx().collectFrom([degenPool.utxo], degenRedeemer)
        .attachMintingPolicy(tokensMintingQty)
        .mintAssets(nftMintingData.mintingValue, nftMintingData.mintingRedeemer)
        .attachMintingPolicy(tokensMintingQty)
        .mintAssets(lqMintingData.mintingValue, lqMintingData.mintingRedeemer)
        .payToContract(poolAddress, {inline: ammPoolDatum}, poolDepositedValue)
        .attachWithdrawalValidator(new FactoryValidateFactory())
        .withdraw(
            lucid.utils.credentialToRewardAddress(
                {hash: config.validators!.factory.hash, type: 'Script'},
            ),
            0,
            Data.void()
        )
        .complete();
}

async function main() {

    const lucid = await getLucid();
    await setupWallet(lucid);
    const conf = await getConfig<BuiltValidators>();

    const degenPoolAddress = lucid.utils.credentialToAddress(
        {hash: conf.validators!.pool.hash, type: 'Script'},
        {hash: poolStakeHash, type: 'Script'}
    )

    console.log("degen address: ", degenPoolAddress);
    const degenPools = await lucid.provider.getUtxos(degenPoolAddress);
    let suitableDegenPoolsToTransfer = await findSuitableDegenPools2AMMCast(degenPools);

    suitableDegenPoolsToTransfer.forEach(async (degenPool2Transfer) =>
        await degen2ammPool(degenPool2Transfer, lucid, conf)
    )
}

main()