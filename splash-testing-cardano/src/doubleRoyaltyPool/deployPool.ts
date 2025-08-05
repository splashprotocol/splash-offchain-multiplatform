import {DoubleRoyaltyPoolPoolValidatePool} from "../../plutus.ts";
import {
    getCSAndSсript,
    getDAOPolicy,
    getUtxoWithAda,
    getUtxoWithToken,
    stringifyBigIntReviewer
} from "../balance/balancePool.ts";
import {getConfig} from "../config.ts";
import {getLucid} from "../lucid.ts";
import {Asset, asUnit, BuiltValidators} from "../types.ts";
import {setupWallet} from "../wallet.ts";
import {Data, Datum, Lucid, MintingPolicy, Unit} from "@lucid-evolution/lucid";
import {credentialToAddress} from "@lucid-evolution/utils";
import {encoder} from 'npm:js-encoding-utils'

export const TokenB   = "636e74546f6b656e746f6b656e"
export const TokenBCS = "f357c6f00f0496fcd01851a7a8d909a1d9d1c9d7ba9bc021ac3bc3fe"

const lqFee = 95000n
const treasuryFee = 10000n
const firstRoyaltyFee = 10000n
const secondRoyaltyFee = 50000n

const startLovelaceValue = 500000000
const startTokenB        = 500000000

// do not touch
const lqEmission = 9223372036854775807n;
const nftEmission = 1n;

const nftTNBase16 = `6e6674`;
const lqTNBase16 = `6c71`;
const encodedTestB = TokenB;// stringToHex(TokenB);
// ad977b5cfaf87b549051e5eab6bde917ed1f24037ced562b0f0449f981f9dda8
export type DoubleRoyaltyPoolConfig = {
    poolNft: Asset,
    poolX: Asset,
    poolY: Asset,
    poolLq: Asset,
    lpFeeIsEditable: boolean,
    lpFeeNum: bigint,
    protocolFeeNum: bigint,
    firstRoyaltyFeeNum: bigint,
    secondRoyaltyFeeNum: bigint,
    treasuryX: bigint,
    treasuryY: bigint,
    firstRoyaltyX: bigint,
    firstRoyaltyY: bigint,
    secondRoyaltyX: bigint,
    secondRoyaltyY: bigint,
    DAOPolicy: Array<{
        Inline: [
                { VerificationKeyCredential: [string] } | {
                ScriptCredential: [string];
            },
        ];
    } | {
        Pointer: {
            slotNumber: bigint;
            transactionIndex: bigint;
            certificateIndex: bigint;
        };
    }>,
    // treasuryAddress - is contract
    treasuryAddress: string,
    firstRoyaltyPubKeyHash256: string,
    secondRoyaltyPubKeyHash256: string,
    royaltyNonce: bigint,
}

function buildRoyaltyPoolDatum(lucid: Lucid, conf: DoubleRoyaltyPoolConfig): Datum {
    return Data.to({
        poolnft: conf.poolNft,
        poolx: conf.poolX,
        poolY: conf.poolY,
        poolLq: conf.poolLq,
        feenum: conf.lpFeeNum,
        treasuryFee: conf.protocolFeeNum,
        firstRoyaltyFee: conf.firstRoyaltyFeeNum,
        secondRoyaltyFee: conf.secondRoyaltyFeeNum,
        treasuryx: conf.treasuryX,
        treasuryy: conf.treasuryY,
        firstroyaltyx: conf.firstRoyaltyX,
        firstroyaltyy: conf.firstRoyaltyY,
        secondroyaltyx: conf.secondRoyaltyX,
        secondroyaltyy: conf.secondRoyaltyY,
        daoPolicy: conf.DAOPolicy,
        treasuryAddress: conf.treasuryAddress,
        firstRoyaltyPubKeyHash_256: conf.firstRoyaltyPubKeyHash256,
        _royaltyPubKeyHash_256: conf.secondRoyaltyPubKeyHash256,
        royaltyNonce: conf.royaltyNonce
    }, DoubleRoyaltyPoolPoolValidatePool.conf)
}

async function main() {

    const lucid = await getLucid();
    await setupWallet(lucid);

    const conf = await getConfig<BuiltValidators>();

    const utxos = (await lucid.wallet().getUtxos());

    const boxWithToken = await getUtxoWithToken(utxos, encodedTestB);
    console.log(`box with token: ${JSON.stringify(utxos, stringifyBigIntReviewer)}`);
    const boxWithAda   = await getUtxoWithAda(utxos)

    if (!boxWithToken) {
        console.log("No box with token!");
        return
    }

    const nftInfo = await getCSAndSсript(boxWithToken.txHash, boxWithToken.outputIndex, nftTNBase16, `${nftEmission}`);
    const lqInfo  = await getCSAndSсript(boxWithToken.txHash, boxWithToken.outputIndex, lqTNBase16, `${lqEmission}`);

    console.log(`nft info: ${nftInfo}`);

    console.log(`address: ${await lucid.wallet().address()}`);

    const poolAddress = credentialToAddress(
        "Preprod",
        { hash: conf.validators!.doubleRoyaltyPool.hash, type: 'Script' },
    );

    const nftMintingPolicy: MintingPolicy =
        {
            type: "PlutusV2",
            script: nftInfo.script
        }

    const lqMintingPolicy: MintingPolicy =
        {
            type: "PlutusV2",
            script: lqInfo.script
        }

    const lqUnit: Unit  = `${lqInfo.policyId.concat(lqTNBase16)}`;
    const nftUnit: Unit = `${nftInfo.policyId.concat(nftTNBase16)}`;

    console.log(`lq: ${lqUnit}`);
    console.log(`nftUnit: ${nftUnit}`);

    const mintingLqAssets: Record<Unit | "lovelace", bigint> =
        {
            [lqUnit]: lqEmission
        }

    const mintingNftAssets: Record<Unit | "lovelace", bigint> =
        {
            [nftUnit]: nftEmission
        }

    const poolConfig: DoubleRoyaltyPoolConfig = {
        poolNft: {
            policy: nftInfo.policyId,
            name: nftTNBase16,
        },
        poolX: {
            policy: "",
            name: "",
        },
        poolY: {
            policy: TokenBCS,
            name: encodedTestB,
        },
        poolLq: {
            policy: lqInfo.policyId,
            name: lqTNBase16,
        },
        lpFeeIsEditable: true,
        lpFeeNum: BigInt(lqFee),
        protocolFeeNum: BigInt(treasuryFee),
        firstRoyaltyFeeNum: BigInt(firstRoyaltyFee),
        secondRoyaltyFeeNum: BigInt(secondRoyaltyFee),
        treasuryX: 0n,
        treasuryY: 0n,
        firstRoyaltyX: 0n,
        firstRoyaltyY: 0n,
        secondRoyaltyX: 0n,
        secondRoyaltyY: 0n,
        DAOPolicy: [{
            Inline: [{ ScriptCredential: [conf.validators!.royaltyDAOV1Pool.hash] }]
        }],
        // treasuryAddress - is contract
        treasuryAddress: conf.validators.royaltyPool.hash,
        firstRoyaltyPubKeyHash256: "b2f97417886f87a990b7f2a6c78c8210d6bcf8c93498d6a5c54f7026a9948d75",
        secondRoyaltyPubKeyHash256: "369e55aff55ee447554bc2ac1a89e92acfb6a06a56a228ca8b90bd82e9a77008",
        royaltyNonce: 0n,
    }

    console.log(`mintingLqAssets: ${JSON.stringify(mintingLqAssets, stringifyBigIntReviewer)}`)
    console.log(`mintingNftAssets: ${JSON.stringify(mintingNftAssets, stringifyBigIntReviewer)}`)

    console.log(`poolConfig: ${JSON.stringify(poolConfig, stringifyBigIntReviewer)}`)

    const depositedValue = {
        lovelace: BigInt(startLovelaceValue),
        [asUnit(poolConfig.poolY)]: BigInt(startTokenB),
        [asUnit(poolConfig.poolNft)]: nftEmission,
        [asUnit(poolConfig.poolLq)]: (lqEmission - BigInt(startLovelaceValue * 2))
    }

    const tx = await lucid.newTx().collectFrom([boxWithToken!])
        .attach.MintingPolicy(nftMintingPolicy)
        .mintAssets(mintingNftAssets, Data.to(0n))
        .attach.MintingPolicy(lqMintingPolicy)
        .mintAssets(mintingLqAssets, Data.to(0n))
        .attachMetadata(42, '008d4be10d934b60a22f267699ea3f7ebdade1f8e535d1bd0ef7ce18b681d87f73eca0cc06a5a85cc9b418fb735410050e5dedadcc1d20a51d'.match(/.{1,64}/g)!.map(chunk => encoder.hexStringToArrayBuffer(chunk)))
		.attachMetadata(43, '00b369499b1daa8a62fea98ae02c623738eda3542e87d3b3a1faf0b7d2fb2c8a3961dcea2ed2461921e0b02890ea7266fb84517b75f99e44e7'.match(/.{1,64}/g)!.map(chunk => encoder.hexStringToArrayBuffer(chunk)))
        .pay.ToContract(
            poolAddress,
            { kind: "inline", value: buildRoyaltyPoolDatum(lucid, poolConfig) },
            depositedValue
        ).complete();

    const txId = await (await tx.sign.withWallet().complete()).submit();

    console.log(`tx: ${txId}`)

    await lucid.awaitTx(txId);
}

main()