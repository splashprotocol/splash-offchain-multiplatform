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

const txb = "84a700828258207f5a9e3d5997c594abc59e6cb0567a58c7a9d010397630dd1d9c877701c708d100825820f9602eafa9415daadb1ba5122c7d98fdb07cf894ab86176f6bbccee4496cf8f1000183a300581d70dca27c481d7864e3a42ce075095295cde3de08e843aaf4b731a3d57801821a01e2f0d0a1581c40079b8ba147fb87a00da10deff7ddd13d64daf48802bb3f82530c3ea14a53504c415348546573741a00011170028201d81858e0d8799f4100581c0896cb319806556fe598d40dcc625c74fa27d29e19a00188c8f830bdd8799f4040ff1a01c9c3801a0007a1201903e8d8799f581c40079b8ba147fb87a00da10deff7ddd13d64daf48802bb3f82530c3e4a53504c41534854657374ffd8799f011903e8ff1a000249f0d8799fd8799f581cab450d88aab97ff92b1614217e5e34b5710e201da0057d3aab684390ffd8799fd8799fd8799f581c1bc47eaccd81a6a13070fdf67304fc5dc9723d85cff31f0421c53101ffffffff581cab450d88aab97ff92b1614217e5e34b5710e201da0057d3aab68439080ffa200583900ab450d88aab97ff92b1614217e5e34b5710e201da0057d3aab6843901bc47eaccd81a6a13070fdf67304fc5dc9723d85cff31f0421c53101011a044794c0825839004be4fa25f029d14c0d723af4a1e6fa7133fc3a610f880336ad685cba5bda73043d43ad8df5ce75639cf48e1f2b4545403be92f0113e375371a0018dd8d021a00066a4305a1581df0b1b6d801f3925f6f55248ce445f7fff0152136abc8a045f1164f9cd8000b58208a04cffbcb7ebc6c94c64fca3aded5499f2cf1c0777425865d17ebecdabb40460d818258202730de014f65658ae457f4d299cc80f4d0fc5c07ccf50528c78907d52f832e81001282825820f7454c8ef770078ea385c797715d75ac5628829d60e4a34f1cd190f85021360300825820f7454c8ef770078ea385c797715d75ac5628829d60e4a34f1cd190f85021360301a20081825820ca91ad1721ca4f58f32de66256ba8f56c7dffdd4f869ab83e95448101c48e1fc5840fa0c91bce3c2f19104c1621885ebf902878c5f3438298de1a75844848f8f14f6c521132a936ab4eec64a3a8c7c73c7ed84fcb22f904859379af48bec6c7fb6020583840000d87a80821a0007a1201a0bebc200840001d87a80821a0007a1201a0bebc20084030080821a001b77401a1dcd6500f5f6";

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

createMToNOrders();
