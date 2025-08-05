import {getCSAndSсript, getUtxoWithAda, TokenInfo} from "../balance/balancePool.ts";
import {getConfig} from "../config.ts";
import {getLucid} from "../lucid.ts";
import {Asset, asUnit, BuiltValidators, PubKeyHash} from "../types.ts";
import {setupWallet} from "../wallet.ts";
import {credentialToAddress, Data, Datum, Lucid, MintingPolicy, ScriptHash, Unit, UTxO} from "@lucid-evolution/lucid";
import {encoder} from 'npm:js-encoding-utils';
import {NftDegenNftValidatorDegen, PoolT2tValidatePoolT2t} from "../../plutus.ts";
import {sha256} from "hash-wasm";

//const nftTNBase16 = `6e6674`;
const tokenAPolicy = `77cb34f72da105bd0cab41c2a10e2fa2fe97a181e6771a62d0c9673e`;
const tokenABase16 = `74657374546f6b656e`;
const tokenDegenTNBase16 = `746f6b656e`;
const aNum = 133079166694n;
const bNum = 3750000n;
const batcherPk = "15772e8f1fdcf12d59636caf42522b7d6249ccb223253eb7e9b6d509";
export const adminWitness = "78dab68b25456933968fd3f6566a2294f5254000d969d954b61366a6"
export const factoryWitness = "afc37fed8f5e6ba6e6212eb165ab0ad616b63e0873889338bbb38c23"
export const feeWithdrawerWitness = "e95b2d9d1cc16f8fd76b5b0743e796b48ae6f448f2e00e96a2214b07"
const tokenAEmission = 1_000_000_000_000_000n;
const degenTokenEmission = 1_000_000_000n;
const nftEmission = 1n;
const startLovelaceValue = 100_000_000;
const cap_thr = 20121951225n;

export type HexString = string;

export const hexToBytes = (hex: HexString): Uint8Array =>
  encoder.hexStringToArrayBuffer(hex);

export type DegenT2TPoolConfig = {
    poolNft: Asset,
    assetX: Asset,
    assetY: Asset,
    aNum: bigint,
    bNum: bigint,
    batcherPk: PubKeyHash,
    cntCapThr: bigint,
    collectedXTokenFee: bigint,
    adminWitness: ScriptHash,
    factoryWitness: ScriptHash,
    feeWithdrawerWitness: ScriptHash
}

export const stringifyBigIntReviewer = (_: any, value: any) =>
  typeof value === 'bigint'
    ? { value: value.toString(), _bigint: true }
    : value;

const POOL_NFT_MINTING_SCRIPT =
  '59023c01000032323232323232323222325333006323232323232323232323253330113370e90001808004099191919299980c180d80109919299980b99b88480000044c8c94ccc064cdc3a40006030002264646464a66603aa66603a004200229405288a503370e00c900119b8f006001372400264660026eb8c00cc060c00cc06005ccc004dd9980f9810180c00b9bb34c0101010022337140040022c646600200201c44a66603a002298103d87a800013232533301c3375e600a6034004032266e952000330200024bd700998020020009810801180f8009180e8008a51375a60300046eb8c05800458c064004c8c8c94ccc058cdc3a4004002297adef6c6013756603660280046028002646600200200444a6660320022980103d87a8000132323232533301a3371e010004266e9520003301e374c00297ae0133006006003375660360066eb8c064008c074008c06c004c8cc004004010894ccc06000452f5bded8c0264646464a66603266e3d22100002100313301d337606ea4008dd3000998030030019bab301a003375c6030004603800460340026eb8c05c004c03c02058dd5980a800980a800980a000980980098090011bac30100013008003300e001300e002300c001300400214984d958c94ccc018cdc3a4000002264646464a66601a60200042649319299980599b87480000044c8c94ccc040c04c00852616375c602200260120082c60120062c6eb4c038004c038008c030004c01000c58c0100088c014dd5000918019baa0015734aae7555cf2ab9f5740ae855d101';

export const POOL_NFT_MINTING_POLICY =
  '63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b8';

const getPoolNftName = async (
  uTxO: UTxO,
  emission: bigint,
): Promise<string> => {
  const uint8array = Uint8Array.from([
    ...hexToBytes(uTxO.txHash),
    ...hexToBytes(Data.to(BigInt(uTxO.outputIndex))),
    ...hexToBytes(Data.to(BigInt(emission))),
  ]);
  return sha256(uint8array);
};

async function buildPoolConfig(lucid: Lucid, nftTN: string, tokenACS: string, tokenBCS: string): Promise<DegenT2TPoolConfig> {

    // const myAddr = await lucid.wallet.address();

    return {
        poolNft: {
            policy: POOL_NFT_MINTING_POLICY,
            name: nftTN,
        },
        // for tests pool x is always ada?
        assetX: {
            policy: tokenACS,
            name: tokenABase16,
        },
        assetY: {
            policy: tokenBCS,
            name: tokenDegenTNBase16,
        },
        aNum: aNum,
        bNum: bNum,
        batcherPk: batcherPk,
        cntCapThr: cap_thr,
        collectedXTokenFee: 0n,
        adminWitness: adminWitness,
        factoryWitness: factoryWitness,
        feeWithdrawerWitness: feeWithdrawerWitness
    }
}

function buildPoolDatum(conf: DegenT2TPoolConfig): Datum {
    return Data.to({
        poolNft: conf.poolNft,
        assetX: conf.assetX,
        assetY: conf.assetY,
        aNum: conf.aNum,
        bNum: conf.bNum,
        batcherPk: conf.batcherPk,
        cntCapThr: conf.cntCapThr,
        collectedXTokenFee: conf.collectedXTokenFee,
        adminWitness: conf.adminWitness,
        factoryWitness: conf.factoryWitness,
        feeWithdrawWitness: conf.feeWithdrawerWitness
    }, PoolT2tValidatePoolT2t.inputDatum)
}

async function main() {

    const lucid = await getLucid();
    await setupWallet(lucid);

    const conf = await getConfig<BuiltValidators>();

    const myAddr = await lucid.wallet().address();

    const utxos = (await lucid.wallet().getUtxos());

    const boxWithAda = getUtxoWithAda(utxos)

    const nftInfo: TokenInfo = {
        policyId: POOL_NFT_MINTING_POLICY,
        script: POOL_NFT_MINTING_SCRIPT,
    }

    //const tokenATokenInfo  = await getCSAndSсript(boxWithAda.txHash, boxWithAda.outputIndex, tokenABase16, `${tokenAEmission}`);
    const degenTokenInfo  = await getCSAndSсript(boxWithAda.txHash, boxWithAda.outputIndex, tokenDegenTNBase16, `${degenTokenEmission}`);

    console.log(`nft info: ${nftInfo}`);

    console.log(`address: ${await lucid.wallet().address()}`);

    const poolAddress = credentialToAddress(
        "Preprod",
        { hash: conf.validators!.degenT2TPool.hash, type: 'Script' },
        { hash: "b2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b07", type: 'Script' },
      );

    const nftMintingPolicy: MintingPolicy =
        {
            type: "PlutusV2",
            script: nftInfo.script
        }

    // const tokenAMintingPolicy: MintingPolicy =
    //     {
    //         type: "PlutusV2",
    //         script: tokenATokenInfo.script
    //     }

    const degenTokenMintingPolicy: MintingPolicy =
        {
            type: "PlutusV2",
            script: degenTokenInfo.script
        }

    const CNTTokenUnit: Unit  = `${tokenAPolicy.concat(tokenABase16)}`;
    const tokenAUnit: Unit  = `${tokenAPolicy.concat(tokenABase16)}`;
    const tokenBUnit: Unit  = `${degenTokenInfo.policyId.concat(tokenDegenTNBase16)}`;

    const nftTNBase16 = await getPoolNftName(boxWithAda, 1n);

    const nftUnit: Unit = `${nftInfo.policyId.concat(nftTNBase16)}`;

    console.log(`tokenAUnit: ${tokenAUnit}`);
    console.log(`degenTokenUnit: ${tokenBUnit}`);
    console.log(`nftUnit: ${nftUnit}`);

    const tokenAAssets: Record<Unit | "lovelace", bigint> =
        {
            [tokenAUnit]: tokenAEmission,
            
        }

    const degenTokenAssets: Record<Unit | "lovelace", bigint> =
        {
            [tokenBUnit]: degenTokenEmission,
            
        }

    const mintingNftAssets: Record<Unit | "lovelace", bigint> =
        {
            [nftUnit]: nftEmission
        }

    const poolConfig = await buildPoolConfig(lucid, nftTNBase16, tokenAPolicy, degenTokenInfo.policyId);

    console.log(`degen t2t poolConfig: ${JSON.stringify(poolConfig, stringifyBigIntReviewer)}`)

    const depositedValue = {
        lovelace: BigInt(startLovelaceValue),
        [asUnit(poolConfig.assetY)]: degenTokenEmission,
        [asUnit(poolConfig.poolNft)]: nftEmission,
    }

    console.log(`depositedValue: ${JSON.stringify(depositedValue, stringifyBigIntReviewer)}`)

    console.log(`ada box: ${JSON.stringify(boxWithAda, stringifyBigIntReviewer)}`);

    const nftMintingRedeemer = 
        Data.to({
            transactionId: {
                hash: boxWithAda.txHash
            },
            outputIndex: BigInt(boxWithAda.outputIndex)
        }, NftDegenNftValidatorDegen.inputOref)

    const tx = await lucid.newTx()
        .collectFrom([boxWithAda!])
        .attach.MintingPolicy(nftMintingPolicy)
        .attach.MintingPolicy(degenTokenMintingPolicy)
        //.attach.MintingPolicy(tokenAMintingPolicy)
        .mintAssets(mintingNftAssets, nftMintingRedeemer)
        .mintAssets(degenTokenAssets, Data.to(0n))
        //.mintAssets(tokenAAssets, Data.to(0n))
        .pay.ToContract(poolAddress, { kind: "inline", value: buildPoolDatum(poolConfig) }, depositedValue)
        .complete({changeAddress: myAddr});

    console.log(`poolConfig: ${JSON.stringify(poolConfig, stringifyBigIntReviewer)}`)

    console.log(`tx: ${JSON.stringify(tx, stringifyBigIntReviewer)}`)

    const txId = await (await tx.sign.withWallet().complete()).submit();

    console.log(`tx: ${txId}`)
}

main()