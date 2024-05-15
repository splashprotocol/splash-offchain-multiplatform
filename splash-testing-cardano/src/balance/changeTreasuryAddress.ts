import {UTxO, Data, Assets} from "https://deno.land/x/lucid@0.10.7/mod.ts";
import {getLucid} from "../lucid.ts";
import {setupWallet} from "../wallet.ts";
import {getConfig} from "../config.ts";
import {BuiltValidators} from "../types.ts";
import {BalanceContract, SquirtContract} from "../../plutus.ts";
import { encoder } from 'npm:js-encoding-utils';
import {getUtxoWithAda} from "./balancePool.ts";

// Available for edit

let squirtNft = "1b6e2890dfcc731b9556bb6a34a1f0c779ac11367603bfd24ee1358e"
let minAdaValue = 5000000

// Don't edit below

const stringifyBigIntReviewer = (_: any, value: any) =>
    typeof value === 'bigint'
        ? {value: value.toString(), _bigint: true}
        : value;

export const hexToString = (hex: HexString): string =>
    encoder.arrayBufferToString(encoder.hexStringToArrayBuffer(hex));

export function extractFullTokenName(assets: Assets, token2find: string) {
    return (Object.keys(assets) as Array<String>).find(key => key.includes(token2find))
}

export function getUtxoWithToken(utxos: UTxO[], token2find: string) {
    return utxos.find( utxo =>
        {
            return (Object.keys(utxo.assets) as Array<String>).find(key => key.includes(token2find)) !== undefined
        }
    )
}

export async function getUtxoWithAda(utxos: UTxO[]) {
    return utxos.find( utxo =>
        {
            return ((Object.keys(utxo.assets).length == 1) && utxo.assets["lovelace"] >= minAdaValue)
        }
    )
}

enum DAOV1Action {
    WithdrawTreasury,
    ChangeStakePart,
    ChangeTreasuryFee,
    ChangeTreasuryAddress,
    ChangeAdminAddress,
    ChangePoolFee
}

function updatePoolDatum(prevDatum: String, action: DAOV1Action, newValue: String): String {
    let parsedDatum = Data.from(
        prevDatum,
        BalanceContract.conf,
    );
    let toAdd;
    switch (action) {
        case DAOV1Action.ChangeAdminAddress:
            toAdd = {daoPolicy: [{"Inline":[{"ScriptCredential":[`${newValue}`]}]}]};
            break;
        case DAOV1Action.ChangePoolFee:
            toAdd = {feenum: BigInt(newValue)};
            break;
        case DAOV1Action.ChangeTreasuryAddress:
            toAdd = {treasuryAddress: newValue};
            break;
        case DAOV1Action.ChangeTreasuryFee:
            toAdd = {treasuryFee: BigInt(newValue)};
            break;
        case DAOV1Action.ChangeStakePart:
            toAdd = {};
            break;
        case DAOV1Action.WithdrawTreasury:
            toAdd = {
                treasuryx: BigInt(0),
                treasuryy: BigInt(0)
            };
            break;
    }
    return Data.to({...parsedDatum, ...toAdd}, BalanceContract.conf);
}

function updatePoolValue(prevPoolDatum: String, prevPoolAssetsValue: Assets, action: DAOV1Action): Assets {
    let parsedDatum = Data.from(
        prevPoolDatum,
        BalanceContract.conf,
    );
    let tokenXKey = extractFullTokenName(prevPoolAssetsValue, parsedDatum.poolx.policy);
    let tokenYKey = extractFullTokenName(prevPoolAssetsValue, parsedDatum.poolY.policy);
    let toRemove: { [key: string]: BigInt; } = {};
    toRemove[tokenXKey] = BigInt(parsedDatum.treasuryx);
    toRemove[tokenYKey] = BigInt(parsedDatum.treasuryy);
    switch (action) {
        case DAOV1Action.ChangeAdminAddress:
            return prevPoolAssetsValue;
        case DAOV1Action.ChangePoolFee:
            return prevPoolAssetsValue;
        case DAOV1Action.ChangeTreasuryAddress:
            return prevPoolAssetsValue;
        case DAOV1Action.ChangeTreasuryFee:
            return prevPoolAssetsValue;
        case DAOV1Action.ChangeStakePart:
            return prevPoolAssetsValue;
        case DAOV1Action.WithdrawTreasury:
            let newValue = (Object.keys(prevPoolAssetsValue)
                .reduce((result, key) => ({ ...result, [key]: toRemove[key] === undefined
                        ? result[key] : result[key] - toRemove[key] }), prevPoolAssetsValue));
            return newValue;
    }
}

function updatePools(poolsUtxo: UTxO[], action: DAOV1Action, newValue: String): [UTxO, UTxO][] {
    console.log(`Going to ${DAOV1Action[action]} in ${poolsUtxo.length} pools for new value: ${newValue}`)
    return poolsUtxo.map(utxo => {
            let updatedDatum = updatePoolDatum(utxo.datum, action, newValue)
            let updatedValue = updatePoolValue(utxo.datum, utxo.assets, action)
            //console.log(`newPoolUtxo: ${JSON.stringify({...utxo, ...{datum:updatedDatum, value: updatedValue}}, stringifyBigIntReviewer)}`)
            return [utxo, {...utxo, ...{datum:updatedDatum, value: updatedValue}}]
        }
    )
}

async function produceTx(lucid: Lucid, prevPoolUtxo: UTxO, newPoolUtxo: UTxO, action: DAOV1Action, withdrawAddress: RewardAddress) {

    let utxoWithWithdrawContract = (await lucid.utxosByOutRef([{
        txHash: "6903a4779e75d9f5796dbede201604184524a7ef277dd09341cb1141856a7a84",
        outputIndex: 0
    }]))

    const utxos = (await lucid.wallet.getUtxos());

    const boxWithAda  = await getUtxoWithAda(utxos)

    const daoRedeemer = Data.to({
        action: BigInt(action),
        poolinidx: BigInt(0)
    }, SquirtContract._)

    const tx = await lucid.newTx().collectFrom([prevPoolUtxo!, boxWithAda!])
        .payToAddressWithData(prevPoolUtxo.address, newPoolUtxo.datum, newPoolUtxo.assets)
        .withdraw(withdrawAddress, 0, daoRedeemer)
        .readFrom(utxoWithWithdrawContract)
        .complete();

    //todo: add signatures
}

async function main() {

    const lucid = await getLucid();
    await setupWallet(lucid);

    const conf = await getConfig<BuiltValidators>();

    const poolAddress = "addr1x8xw6pmmy8jcnpss6sg7za9c5lk2v9nflq684vzxyn70unaj764lvrxdayh2ux30fl0ktuh27csgmpevdu89jlxppvrs4tm5z7"

    let pools = (await lucid.utxosAt(poolAddress));

    let squirtPool = [getUtxoWithToken(pools, squirtNft)];

    console.log(`res ${JSON.stringify(squirtPool, stringifyBigIntReviewer)}`)

    let action   = DAOV1Action.WithdrawTreasury
    let newValue = "0"

    let prevPoolUtxosWithUpdated = updatePools(squirtPool, action, newValue)

    let daoWithdrawAddress = lucid.utils.validatorToRewardAddress(conf.validators!.squirt.script);

    let txs = prevPoolUtxosWithUpdated.map(poolsUtxo => {
        produceTx(lucid, poolsUtxo[0], poolsUtxo[1], action, daoWithdrawAddress)
    })
}

main()