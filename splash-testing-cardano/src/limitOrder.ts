import { LimitOrderLimitOrder } from "../plutus.ts";
import { getLucid } from "./lucid.ts";
import { asUnit } from "./types.ts";
import { BuiltValidator } from "./types.ts";
import { Asset, PubKeyHash, Rational} from "./types.ts";
import { Address, Data, Datum, Lovelace, Lucid, PolicyId, TxComplete, UTxO, applyDoubleCborEncoding, fromHex, toHex } from 'https://deno.land/x/lucid@0.10.7/mod.ts';
import { setupWallet } from "./wallet.ts";
import { getConfig } from "./config.ts";
import { BuiltValidators } from "./types.ts";
import { hash_blake2b224 } from "https://deno.land/x/lucid@0.10.7/src/core/libs/cardano_multiplatform_lib/cardano_multiplatform_lib.generated.js";
import { decodeString } from "https://deno.land/std@0.100.0/encoding/hex.ts";

export type LimitOrderConf = {
    input: Asset,
    output: Asset,
    tradableInput: bigint,
    minMarginalOutput: bigint,
    costPerExStep: Lovelace,
    basePrice: Rational,
    fee: Lovelace,
    redeemerAddr: Address,
    cancellationPkh: PubKeyHash,
    permittedExecutors: PubKeyHash[],
}

function buildLimitOrderDatum(lucid: Lucid, conf: LimitOrderConf, beacon: PolicyId): Datum {
    return Data.to({
        tag: "00",
        beacon: beacon,
        input: conf.input,
        tradableInput: conf.tradableInput,
        costPerExStep: conf.costPerExStep,
        minMarginalOutput: conf.minMarginalOutput,
        output: conf.output,
        basePrice: conf.basePrice,
        fee: conf.fee,
        redeemerAddress: {
            paymentCredential: { VerificationKeyCredential: [lucid.utils.paymentCredentialOf(conf.redeemerAddr).hash] },
            stakeCredential: {
              Inline: [{ VerificationKeyCredential: [lucid.utils.stakeCredentialOf(conf.redeemerAddr).hash] }],
            },
          },
        cancellationPkh: conf.cancellationPkh,
        permittedExecutors: conf.permittedExecutors,
    }, LimitOrderLimitOrder.conf)
}

function beaconFromInput(utxo: UTxO): PolicyId {
    const txHash = fromHex(utxo.txHash);
    const index = new TextEncoder().encode(utxo.outputIndex.toString())
    return toHex(hash_blake2b224(new Uint8Array([...txHash, ...index])))
}

async function createLimitOrder(lucid: Lucid, validator: BuiltValidator, conf: LimitOrderConf): Promise<TxComplete> {
    const orderAddress = lucid.utils.credentialToAddress(
        { hash: validator.hash, type: 'Script' },
      );
    const input = (await lucid.wallet.getUtxos())[0];
    const beacon = beaconFromInput(input);
    console.log("Beacon: " + beacon);
    const lovelaceTotal = conf.fee + conf.costPerExStep * 4n;
    const depositedValue = conf.input.policy == "" ? { lovelace: lovelaceTotal + conf.tradableInput } : { lovelace: lovelaceTotal, [asUnit(conf.input)]: conf.tradableInput};
    const tx = lucid.newTx().collectFrom([input]).payToContract(orderAddress, { inline: buildLimitOrderDatum(lucid, conf, beacon) }, depositedValue);
    return tx.complete();
}

async function main() {
    const lucid = await getLucid();
    await setupWallet(lucid);
    const conf = await getConfig<BuiltValidators>();
    const myAddr = await lucid.wallet.address();
    const tx = await createLimitOrder(lucid, conf.validators!.limitOrder, {
        input: {
            policy: "fd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3a",
            name: "74657374746f6b656e",
        },
        output: {
            policy: "",
            name: "",
        },
        tradableInput: 100_000n,
        minMarginalOutput: 1_000n,
        costPerExStep: 500_000n,
        basePrice: {
            num: 1000n,
            denom: 1n,
        },
        fee: 500_000n,
        redeemerAddr: myAddr,
        cancellationPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
        permittedExecutors: [],
    });
    console.log(myAddr);
    const txId = await (await tx.sign().complete()).submit();
    console.log(txId);
}

main();
