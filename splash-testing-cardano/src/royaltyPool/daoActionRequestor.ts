import {getLucid} from "../lucid.ts";
import {getPrivateKey, setupWallet} from "../wallet.ts";
import {Config, getConfig} from "../config.ts";
import {BuiltValidators, DeployedValidators} from "../types.ts";
import {credentialToAddress} from "@lucid-evolution/utils";
import * as CML from "@anastasia-labs/cardano-multiplatform-lib-nodejs";
import {Data, Datum, fromHex, Lucid, LucidEvolution, toHex} from "@lucid-evolution/lucid";
import {
    RoyaltyPoolDaoV1DummyValidate, RoyaltyPoolDaoV1RequestValidate,
    RoyaltyPoolWithdrawRoyaltyRequestDummyValidate,
    RoyaltyPoolWithdrawRoyaltyRequestValidate
} from "../../plutus.ts";
import {WithdrawRoyalty} from "./withdrawRoyalty.ts";
import {blake2b} from "hash-wasm";

const nftCSBase16 = `34d9e2ffb87551fa04191ed8e5fb2af78e29d023d48dcf429c1590ea`;
const nftTNBase16 = `6e6674`;
const toWithdrawX = 0n;
const toWithdrawY = 0n;
const startLovelaceValue = 10_000_000n;
const fee = 8_330_000n;
const dao_action = 2n;
const lqFee = 95000n
const treasuryFee = 10000n
const royaltyFee = 10000n

export type DaoRequestDataToSign = {
    daoAction: bigint,
    poolNft: { policy: string; name: string };
    poolFee: bigint,
    treasuryFee: bigint,
    adminAddress: Array<{
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
    poolAddress: {
        paymentCredential: { VerificationKeyCredential: [string] } | {
            ScriptCredential: [string];
        };
        stakeCredential: {
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
        } | null;
    },
    treasuryaddress: string;
    treasuryxwithdraw: bigint;
    treasuryywithdraw: bigint;
    poolNonce: bigint;
}

export type DaoRequestConfig = {
    daoAction: bigint,
    poolNft: { policy: string; name: string };
    poolFee: bigint,
    treasuryFee: bigint,
    adminAddress: Array<{
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
    poolAddress: {
        paymentCredential: { VerificationKeyCredential: [string] } | {
            ScriptCredential: [string];
        };
        stakeCredential: {
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
        } | null;
    },
    treasuryaddress: string;
    treasuryxwithdraw: bigint;
    treasuryywithdraw: bigint;
    requestorpkh: string;
    signatures: Array<string>;
    exfee: bigint;
}

async function createConfig(
    privateKey: CML.Bip32PrivateKey,
    validators: Config<BuiltValidators>,
): Promise<DaoRequestConfig> {

    let dataToSign = Data.to(
        {
            daoaction: dao_action,
            poolnft: {policy: nftCSBase16, name: nftTNBase16},
            poolfee: lqFee,
            treasuryfee: treasuryFee,
            adminaddress: [{
                Inline: [{ ScriptCredential: [validators.validators!.royaltyDAOV1Pool.hash] }]
            }],
            pooladdress: {
                paymentCredential: { ScriptCredential: [validators.validators!.royaltyPool.hash] },
                stakeCredential: null
            },
            //todo: incorrect, just for test
            treasuryaddress: validators.validators.royaltyPool.hash,
            treasuryxwithdraw: toWithdrawX,
            treasuryywithdraw: toWithdrawY,
            nonce: 1n
        }
        , RoyaltyPoolDaoV1DummyValidate.conf)
// d8799f02d8799f581c367c57eb3bbc5361bb6d171450612d0cee0e0c3183a2c969dd5afc3b436e6674ff1a000173181927109fd8799fd87a9f581c3eaf2125c8433f3798465b5fd022d587babb8477d776a4ef41539b09ffffffd8799fd87a9f581cb09d1ccde30a0c65316c8e5dada41ee4d86ca0d0bb5370542693dafaffd87a80ff581cb09d1ccde30a0c65316c8e5dada41ee4d86ca0d0bb5370542693dafa000001ff
// d8799f02d8799f581c367c57eb3bbc5361bb6d171450612d0cee0e0c3183a2c969dd5afc3b436e6674ff1a000173181927109fd8799fd87a9f581c3eaf2125c8433f3798465b5fd022d587babb8477d776a4ef41539b09ffffffd8799fd87a9f581cb09d1ccde30a0c65316c8e5dada41ee4d86ca0d0bb5370542693dafaffd87a80ff581cb09d1ccde30a0c65316c8e5dada41ee4d86ca0d0bb5370542693dafa000001ff
    let dataToSignHex = fromHex(dataToSign)

    console.log(`hex: ${dataToSign}`)
    console.log(`public key: ${toHex(privateKey.to_public().to_raw_bytes())}`)
    console.log(`public key: ${toHex(privateKey.to_public().to_raw_key().to_raw_bytes())}`)

    let signature = privateKey.to_raw_key().sign(dataToSignHex).to_hex()

    let pkh = await blake2b(
        Uint8Array.from([...privateKey.to_public().to_raw_key().to_raw_bytes()]),
        224,
    )

    return {
        daoAction: dao_action,
        poolNft: {policy: nftCSBase16, name: nftTNBase16},
        poolFee: lqFee,
        treasuryFee: treasuryFee,
        adminAddress: [{
            Inline: [{ ScriptCredential: [validators.validators!.royaltyDAOV1Pool.hash] }]
        }],
        poolAddress: {
            paymentCredential: { ScriptCredential: [validators.validators!.royaltyPool.hash] },
            stakeCredential: null
        },
        treasuryaddress: validators.validators!.royaltyPool.hash,
        treasuryxwithdraw: toWithdrawX,
        treasuryywithdraw: toWithdrawY,
        requestorpkh: pkh,
        signatures: [signature],
        exfee: fee
    }
}

function buildDaoRequestDatum(lucid: Lucid, conf: DaoRequestConfig): Datum {

    console.log(`conf: ${conf}`)

    return Data.to({
        daoaction: conf.daoAction,
        poolnft: conf.poolNft,
        poolfee: conf.poolFee,
        treasuryfee: conf.treasuryFee,
        adminaddress: conf.adminAddress,
        pooladdress: conf.poolAddress,
        treasuryaddress: conf.treasuryaddress,
        treasuryxwithdraw: conf.treasuryxwithdraw,
        treasuryywithdraw: conf.treasuryywithdraw,
        requestorpkh: conf.requestorpkh,
        signatures: conf.signatures,
        exfee: conf.exfee
    }, RoyaltyPoolDaoV1RequestValidate.conf)
}

async function main() {

    const lucid = await getLucid();
    await setupWallet(lucid);

    let privateKey = await getPrivateKey();

    const conf: Config<BuiltValidators> = await getConfig<BuiltValidators>();

    const utxos = (await lucid.wallet().getUtxos());

    const daoRequestAddress = credentialToAddress(
        "Preprod",
        {hash: conf.validators!.royaltyDAOV1Request.hash, type: 'Script'},
    );

    const depositedValue = {
        lovelace: BigInt(startLovelaceValue),
    }

    const daoRequestConfig: DaoRequestConfig = await createConfig(privateKey, conf)

    let cfg = buildDaoRequestDatum(lucid, daoRequestConfig)

    const tx = await lucid.newTx()
        .pay.ToContract(
            daoRequestAddress,
            {kind: "inline", value: cfg},
            depositedValue
        ).complete();

    const txId = await (await tx.sign.withWallet().complete()).submit();

    console.log(`tx: ${txId}`)

    await lucid.awaitTx(txId);
}

main()