import { LimitOrderLimitOrder } from "../plutus.ts";
import { getLucid } from "./lucid.ts";
import { asUnit } from "./types.ts";
import { BuiltValidator } from "./types.ts";
import { Asset, PubKeyHash, Rational} from "./types.ts";
import { Address, Data, Datum, Lovelace, Lucid, PolicyId, TxComplete, UTxO, fromHex, toHex } from 'https://deno.land/x/lucid@0.10.7/mod.ts';
import { setupWallet } from "./wallet.ts";
import { getConfig } from "./config.ts";
import { BuiltValidators } from "./types.ts";
import { hash_blake2b224 } from "https://deno.land/x/lucid@0.10.7/src/core/libs/cardano_multiplatform_lib/cardano_multiplatform_lib.generated.js";
import { sleep } from "https://deno.land/x/sleep/mod.ts"

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

const txb = "84a70082825820480206002a0f169aea35f906b2fb358dccf52eb62bf0bf14dc19399af45e1eca00825820f69d38aacbfd12cdee8ad0eefb36f403bc23ff72b59437cee28e1dad703cd363000182a2005839004be4fa25f029d14c0d723af4a1e6fa7133fc3a610f880336ad685cba5bda73043d43ad8df5ce75639cf48e1f2b4545403be92f0113e3753701821a001e8480a1581cfd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3aa14974657374746f6b656e1a000186a0a2005839004be4fa25f029d14c0d723af4a1e6fa7133fc3a610f880336ad685cba5bda73043d43ad8df5ce75639cf48e1f2b4545403be92f0113e37537011a06146580021a000f424005a1581df0b1b6d801f3925f6f55248ce445f7fff0152136abc8a045f1164f9cd8000b5820fe0601ad51d6ea7ca5e02abf3200ce0c7926d5cab1b118cc151d41ca1d798df00d818258202730de014f65658ae457f4d299cc80f4d0fc5c07ccf50528c78907d52f832e81001282825820f7454c8ef770078ea385c797715d75ac5628829d60e4a34f1cd190f85021360300825820f7454c8ef770078ea385c797715d75ac5628829d60e4a34f1cd190f85021360301a20081825820ca91ad1721ca4f58f32de66256ba8f56c7dffdd4f869ab83e95448101c48e1fc5840b6748958916990131dc8cc586547b2e6e4b29d5b0afe84a4ef261e1007480ad83fbb73a5a7869d3dcd06c6cd623d1a26427a488d5566efc54834f218638379060583840000d87a80821a0007a1201a0bebc200840001d87a80821a0007a1201a0bebc20084030080821a000dbba01a0ee6b280f5f6";

async function submit() {
    const lucid = await getLucid();
    await setupWallet(lucid);
    const id = await lucid.wallet.submitTx(txb);
    console.log(id);
}

async function main() {
    const lucid = await getLucid();
    await setupWallet(lucid);
    const conf = await getConfig<BuiltValidators>();
    const myAddr = await lucid.wallet.address();
    console.log("My address: ", lucid.utils.getAddressDetails(myAddr));
    const txBid = await createLimitOrder(lucid, conf.validators!.limitOrder, {
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
    const txBidId = await (await txBid.sign().complete()).submit();
    console.log(txBidId);
    await sleep(120);
    const txAsk = await createLimitOrder(lucid, conf.validators!.limitOrder, {
        input: {
            policy: "",
            name: "",
        },
        output: {
            policy: "fd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3a",
            name: "74657374746f6b656e",
        },
        tradableInput: 100_000_000n,
        minMarginalOutput: 1_000n,
        costPerExStep: 500_000n,
        basePrice: {
            num: 1n,
            denom: 1000n,
        },
        fee: 500_000n,
        redeemerAddr: myAddr,
        cancellationPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
        permittedExecutors: [],
    });
    const txAskId = await (await txAsk.sign().complete()).submit();
    console.log(txAskId);
}

main();
