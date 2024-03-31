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

const txb = "84a70083825820181885cd3f9183199ff0bb1eb7fc48f4cee2922fede579565a7c13eadf962528008258202d90c15191e8ff3d8cd3db074c7db4723d63516b9d2fbee8e59e39ef1a6d592600825820c3e93caa62a624c323d8ef6225bd9cc9e6adc5f4654773d175e1ad0bc360feb1000184a2005839004be4fa25f029d14c0d723af4a1e6fa7133fc3a610f880336ad685cba5bda73043d43ad8df5ce75639cf48e1f2b4545403be92f0113e3753701821a0016e360a1581cfd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3aa14974657374746f6b656e1a000186a0a300581d70dca27c481d7864e3a42ce075095295cde3de08e843aaf4b731a3d57801821a0281bdb7a1581cfd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3aa14974657374746f6b656e197530028201d81858ddd8799f4100581c28af8197f9c986da51bb3af38006ffb64c72a3f1a7608b871b4f780cd8799f581cfd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3a4974657374746f6b656eff1975301a000927c01903e8d8799f4040ffd8799f1903e801ff1a0003ec77d8799fd8799f581c4be4fa25f029d14c0d723af4a1e6fa7133fc3a610f880336ad685cbaffd8799fd8799fd8799f581c5bda73043d43ad8df5ce75639cf48e1f2b4545403be92f0113e37537ffffffff581c4be4fa25f029d14c0d723af4a1e6fa7133fc3a610f880336ad685cba80ffa2005839004be4fa25f029d14c0d723af4a1e6fa7133fc3a610f880336ad685cba5bda73043d43ad8df5ce75639cf48e1f2b4545403be92f0113e37537011a03aefe40825839004be4fa25f029d14c0d723af4a1e6fa7133fc3a610f880336ad685cba5bda73043d43ad8df5ce75639cf48e1f2b4545403be92f0113e375371a00262dc5021a0008406405a1581df0b1b6d801f3925f6f55248ce445f7fff0152136abc8a045f1164f9cd8000b58203776b6767a7408cbeed3dcb2c43b7812e57d2f70eaaed7f4d18a1df06aa68c910d818258202730de014f65658ae457f4d299cc80f4d0fc5c07ccf50528c78907d52f832e81001282825820f7454c8ef770078ea385c797715d75ac5628829d60e4a34f1cd190f85021360300825820f7454c8ef770078ea385c797715d75ac5628829d60e4a34f1cd190f85021360301a20081825820ca91ad1721ca4f58f32de66256ba8f56c7dffdd4f869ab83e95448101c48e1fc5840d2cd5831ec9c46cc79457df6a3adc6008ddd69cca7ebe4c882128f21f4bfb7b0cac930529fb9ed96f817ab02f9e757eeee939f7da5204483e5d19a3ad066ce0c0584840000d87a80821a0007a1201a0bebc200840001d87a80821a0007a1201a0bebc200840002d87a80821a0007a1201a0bebc20084030080821a002932e01a2cb41780f5f6";

const lucid = await getLucid();
await setupWallet(lucid);
const conf = await getConfig<BuiltValidators>();

async function submit() {
    const id = await lucid.wallet.submitTx(txb);
    console.log(id);
}

async function main() {
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

async function createMToNOrders() {
    const myAddr = await lucid.wallet.address();
    console.log("My address: ", lucid.utils.getAddressDetails(myAddr));
    const txBid1 = await createLimitOrder(lucid, conf.validators!.limitOrder, {
        input: {
            policy: "fd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3a",
            name: "74657374746f6b656e",
        },
        output: {
            policy: "",
            name: "",
        },
        tradableInput: 70_000n,
        minMarginalOutput: 1_000n,
        costPerExStep: 600_000n,
        basePrice: {
            num: 1000n,
            denom: 1n,
        },
        fee: 600_000n,
        redeemerAddr: myAddr,
        cancellationPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
        permittedExecutors: [],
    });
    const txBidId1 = await (await txBid1.sign().complete()).submit();
    console.log("Bid #0: " + txBidId1);
    await sleep(180);
    const txBid2 = await createLimitOrder(lucid, conf.validators!.limitOrder, {
        input: {
            policy: "fd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3a",
            name: "74657374746f6b656e",
        },
        output: {
            policy: "",
            name: "",
        },
        tradableInput: 60_000n,
        minMarginalOutput: 1_000n,
        costPerExStep: 600_000n,
        basePrice: {
            num: 1000n,
            denom: 1n,
        },
        fee: 500_000n,
        redeemerAddr: myAddr,
        cancellationPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
        permittedExecutors: [],
    });
    const txBidId2 = await (await txBid2.sign().complete()).submit();
    console.log("Bid #1: " + txBidId2);
    await sleep(180);
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
    console.log("Ask #0: " + txAskId);
}

submit();
