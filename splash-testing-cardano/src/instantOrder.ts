import {
    Address,
    CML,
    credentialToAddress,
    Data,
    Datum,
    getAddressDetails,
    Lovelace,
    LucidEvolution,
    paymentCredentialOf,
    PolicyId,
    stakeCredentialOf,
    TxSignBuilder,
    UTxO
} from "@lucid-evolution/lucid";
import {Asset, asUnit, BuiltValidator, BuiltValidators, PubKeyHash, Rational} from "./types.ts";
import {InstantOrderInstantOrder} from "../plutus.ts";
import {getLucid} from "./lucid.ts";
import {setupWallet} from "./wallet.ts";
import {getConfig} from "./config.ts";
import {blake2b} from "hash-wasm";
import {Uint64BE} from 'int64-buffer';
import {getUtxoWithToken} from "./balance/balancePool.ts";
import {encoder} from "js-encoding-utils";

const tokenAPolicy = `77cb34f72da105bd0cab41c2a10e2fa2fe97a181e6771a62d0c9673e`;
const tokenABase16 = `74657374546f6b656e`;

export type HexString = string;

export const bytesToHex = (bytes: Uint8Array): HexString =>
  encoder.arrayBufferToHexString(bytes);

export const hexToBytes = (hex: HexString): Uint8Array =>
  encoder.hexStringToArrayBuffer(hex);

export type InstantOrderConf = {
    redeemerAddress: Address
    input: Asset,
    output: Asset,
    basePrice: Rational,
    fee: Lovelace,
    permittedExecutors: PubKeyHash,
    cancellationAfter: bigint,
    cancellationPkh: PubKeyHash,
    redeemerAddr: Address,
    minLovelace: bigint,
    tradableInput: bigint
}

function buildInstantOrderDatum(lucid: LucidEvolution, conf: InstantOrderConf, beacon: PolicyId): Datum {
    return Data.to({
        tag: "01",
        redeemerAddress: {
            paymentCredential: { VerificationKeyCredential: [paymentCredentialOf(conf.redeemerAddr).hash] },
            stakeCredential: {
              Inline: [{ VerificationKeyCredential: [stakeCredentialOf(conf.redeemerAddr).hash] }],
            },
          },
        input: conf.input,
        output: conf.output,
        basePrice: conf.basePrice,
        fee: conf.fee,
        permittedExecutors: conf.permittedExecutors,
        cancellationAfter: 0n,
        cancellationPkh: conf.cancellationPkh,
        minLovelace: 1_500_000n
    }, InstantOrderInstantOrder.conf)
}

async function createInstantOrder(lucid: LucidEvolution, validator: BuiltValidator, conf: InstantOrderConf): Promise<TxSignBuilder> {
    const orderAddress = credentialToAddress(
        "Preprod",
        { hash: validator.hash, type: 'Script' },
    );

    const utxos = (await lucid.wallet().getUtxos());

    const input = await getUtxoWithToken(utxos, tokenABase16)
    const beacon = await beaconFromInput(lucid, input, conf);
    console.log("Beacon: " + beacon);
    const lovelaceTotal = conf.fee + conf.minLovelace * 4n;
    const depositedValue = conf.input.policy == "" ? { lovelace: lovelaceTotal + conf.tradableInput } : { lovelace: lovelaceTotal, [asUnit(conf.input)]: conf.tradableInput};
    const tx = lucid.newTx().collectFrom([input]).pay.ToAddressWithData(orderAddress, { kind: "inline", value: buildInstantOrderDatum(lucid, conf, beacon) }, depositedValue);
    return tx.complete();
}

const EMPTY_BEACON = bytesToHex(Uint8Array.from(new Array(28).fill(0)));

async function beaconFromInput(
  lucid: LucidEvolution,
  boxWithToken: UTxO,
  instantOrderConfig: InstantOrderConf
): Promise<PolicyId> {
    const C = await CML;

  let confWithEmptyBeacon = buildInstantOrderDatum(lucid, instantOrderConfig, EMPTY_BEACON);

  let hashForEmptyBeacon = await blake2b(hexToBytes(confWithEmptyBeacon), 224);

  console.log(`hashForEmptyBeacon: ${hashForEmptyBeacon}`)

  let test_vector = Uint8Array.from([
      ...C.TransactionHash.from_hex(boxWithToken.txHash).to_raw_bytes(),
      ...new Uint64BE(Number(boxWithToken.outputIndex)).toArray(),
      ...new Uint64BE(Number(0)).toArray(),
      ...hexToBytes(hashForEmptyBeacon),
    ])

  console.log(`test vector: ${bytesToHex(test_vector)}`)

  return blake2b(
    test_vector,
    224
  )
}

async function main() {

    const lucid = await getLucid();
    await setupWallet(lucid);
    const conf = await getConfig<BuiltValidators>();

    const myAddr = await lucid.wallet().address();
    console.log("My address: ", getAddressDetails(myAddr));
    const txBid = await createInstantOrder(lucid, conf.validators!.instantOrder, {
        input: {
            policy: tokenAPolicy,
            name: tokenABase16,
        },
        output: {
            policy: "ce93f37e1b9da84739be6b32d266f5c7eef5b56ee20173e64fc6ec89",
            name: "746f6b656e",
        },
        tradableInput: 10_000_000n,
        basePrice: {
            num: 0n,
            denom: 1n,
        },
        fee: 700000n,
        redeemerAddr: myAddr,
        cancellationPkh: getAddressDetails(myAddr).paymentCredential!.hash,
        permittedExecutors: "15772e8f1fdcf12d59636caf42522b7d6249ccb223253eb7e9b6d509",
        redeemerAddress: myAddr,
        cancellationAfter: 0n,
        minLovelace: 1_500_000n
    });
    const txBidId = await (await txBid.sign.withWallet().complete()).submit();
    console.log(txBidId);
}

main()