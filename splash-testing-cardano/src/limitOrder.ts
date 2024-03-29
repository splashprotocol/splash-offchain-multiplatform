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

const txb = "84a700828258208de7964661b3b2e869fb50ee08327bafbf0c81730dbeb00540438a0f82a5af17008258209d719768f3e90570cfd0e5f0dcc506c611a388ea845a79de37d5544f3610d3f6000183a2005839001e5b525041f0d70ad830f1d7dbd2ed7012c1d89788b4385d7bdd0c376b6723106d7725d57913612286514abb81148d344b1675df297ee224011a0016e360a300581d70dfaa80c9732ed3b7752ba189786723c6709e2876a024f8f4d9910fb301821a00305714a1581cfd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3aa14974657374746f6b656e01028201d81858d0d8798c4100581c302a2ccae50e11631d9f058033f20ef8f0e298149da19720f40ce0a5d8798240400a1a0007a12000d87982581cfd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3a4974657374746f6b656ed87982192710011a0007a120d87982d87981581c1e5b525041f0d70ad830f1d7dbd2ed7012c1d89788b4385d7bdd0c37d87981d87981d87981581c6b6723106d7725d57913612286514abb81148d344b1675df297ee224581c1e5b525041f0d70ad830f1d7dbd2ed7012c1d89788b4385d7bdd0c3780825839004be4fa25f029d14c0d723af4a1e6fa7133fc3a610f880336ad685cba5bda73043d43ad8df5ce75639cf48e1f2b4545403be92f0113e375371a001224fc021a0004be6405a1581df02be9e0e775b72db02ab618a03ccbe70c357a47bcd8437323e7e0f51a000b5820dc16c98540bea4a17932685dd0200c0b388ded6e0f5591b52a1a6855752a9a6a0d818258202730de014f65658ae457f4d299cc80f4d0fc5c07ccf50528c78907d52f832e81001282825820651244494300b196458fe5667f84929641dd1fb743308012ffd7ec8e86b9d53700825820651244494300b196458fe5667f84929641dd1fb743308012ffd7ec8e86b9d53701a20081825820ca91ad1721ca4f58f32de66256ba8f56c7dffdd4f869ab83e95448101c48e1fc58406487c02c5d654feef323dd23b7c3e26cc3498b7dcc1dfc988b5d646c6ce056e6d9aa63d1299c146b7dee82fee2a8c64292f9c121271dfb96226850056049bc0f0583840000d87a80821a000298101a03dfd240840001d87a80821a000298101a03dfd24084030080821a000fde801a17d78400f5f6";

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
    // const txBid1 = await createLimitOrder(lucid, conf.validators!.limitOrder, {
    //     input: {
    //         policy: "fd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3a",
    //         name: "74657374746f6b656e",
    //     },
    //     output: {
    //         policy: "",
    //         name: "",
    //     },
    //     tradableInput: 70_000n,
    //     minMarginalOutput: 1_000n,
    //     costPerExStep: 600_000n,
    //     basePrice: {
    //         num: 1000n,
    //         denom: 1n,
    //     },
    //     fee: 600_000n,
    //     redeemerAddr: myAddr,
    //     cancellationPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
    //     permittedExecutors: [],
    // });
    // const txBidId1 = await (await txBid1.sign().complete()).submit();
    // console.log("Bid #0: " + txBidId1);
    // await sleep(60);
    // const txBid2 = await createLimitOrder(lucid, conf.validators!.limitOrder, {
    //     input: {
    //         policy: "4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26",
    //         name: "7465737443",
    //     },
    //     output: {
    //         policy: "",
    //         name: "",
    //     },
    //     tradableInput: 3_000_000n,
    //     minMarginalOutput: 3n,
    //     costPerExStep: 600_000n,
    //     basePrice: {
    //         num: 13000n,
    //         denom: 1n,
    //     },
    //     fee: 500_000n,
    //     redeemerAddr: myAddr,
    //     cancellationPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
    //     permittedExecutors: [],
    // });
    // const txBidId2 = await (await txBid2.sign().complete()).submit();
    //console.log("Bid #1: " + txBidId2);
    //await sleep(60);
    const txAsk = await createLimitOrder(lucid, conf.validators!.limitOrder, {
        input: {
            policy: "",
            name: "",
        },
        output: {
            policy: "95a427e384527065f2f8946f5e86320d0117839a5e98ea2c0b55fb00",
            name: "48554e54",
        },
        tradableInput: 10_000_000n,
        minMarginalOutput: 1_000n,
        costPerExStep: 5_000n,
        basePrice: {
            num: 1n,
            denom: 1000n,
        },
        fee: 250_000n,
        redeemerAddr: myAddr,
        cancellationPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
        permittedExecutors: [],
    });
    const txAskId = await (await txAsk.sign().complete()).submit();
    console.log("Ask #0: " + txAskId);
}

createMToNOrders();
