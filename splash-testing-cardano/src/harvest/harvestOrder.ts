import { getLucid } from "../lucid.ts";
import { HarvestHarvest } from "../../plutus.ts";
import { BuiltValidator, BuiltValidators, PubKeyHash } from "../../src/types.ts";
import {
    Address,
    Data,
    Datum,
    Lovelace,
    Lucid,
    PolicyId,
    UTxO,
    fromHex,
    toHex,
    paymentCredentialOf, stakeCredentialOf, LucidEvolution, TxSignBuilder,
    credentialToAddress,
    Assets,
    getAddressDetails
} from '@lucid-evolution/lucid';
import { setupWallet } from "../../src/wallet.ts";
import { getConfig } from "../../src/config.ts";

export type HarvestOrder = {
    refundKey: PubKeyHash,
    distributionAgentKey: PubKeyHash
}

const distributionAgentKey = "79c7b50d79c32ea7b6bde64d4dfd5f595a725966bfdf1155385bddac"

function buildHarvestOrderDatum(lucid: Lucid, conf: HarvestOrder): Datum {
    return Data.to({
        refundKey: conf.refundKey,
        distributionAgentKey: conf.distributionAgentKey
    }, HarvestHarvest.state)
}

async function createHarvestOrder(lucid: LucidEvolution, validators: BuiltValidators, conf: HarvestOrder): Promise<TxSignBuilder> {

    const harvestValidator = validators!.harvest

    const orderAddress = credentialToAddress(
        "Preprod",
        { hash: harvestValidator.hash, type: 'Script' },
    );

    const input = (await lucid.wallet().getUtxos()).filter( utxo =>
        utxo.assets.lovelace >= 10_000_000
    )[0];

    const lovelaceTotal = 2_000_0000n;
    const depositedValue: Assets = { lovelace: lovelaceTotal };
    const tx = lucid.newTx()
        .collectFrom([input])
        .pay
        .ToAddressWithData(
            orderAddress, 
            { 
                kind: "inline",
                value: buildHarvestOrderDatum(lucid, conf)
            }, 
            depositedValue
        );
    return tx.complete();
}

async function main() {

    const lucid = await getLucid();
    await setupWallet(lucid);
    const conf = await getConfig<BuiltValidators>();

    const myAddr = await lucid.wallet().address();
    console.log("My address: ", getAddressDetails(myAddr));

    const harvestOrder = await createHarvestOrder(lucid, conf.validators, {
        refundKey: getAddressDetails(myAddr).paymentCredential!.hash,
        distributionAgentKey,
    });

    const harvestOrderId = await (await harvestOrder.sign.withWallet().complete()).submit();
    console.log(harvestOrderId);
}

main()