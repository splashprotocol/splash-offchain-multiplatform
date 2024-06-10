import { getLucid } from "../lucid.ts";
import { setupWallet } from "../wallet.ts";
import { getConfig } from "../config.ts";
import { BuiltValidators } from "../types.ts";
import {createLimitOrder} from "../limitOrder.ts"

const lucid = await getLucid();
await setupWallet(lucid);

const conf = await getConfig<BuiltValidators>();

async function main() {

    const myAddr = await lucid.wallet.address();
    console.log("My address: ", lucid.utils.getAddressDetails(myAddr));
    const txBid = await createLimitOrder(lucid, conf.validators!.limitOrder, {
        input: {
            policy: "4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26",
            name: "7465737443",
        },
        output: {
            policy: "", //"",
            name: "",
        },
        tradableInput: 100_000n,
        minMarginalOutput: 10000n,
        costPerExStep: 500_000n,
        basePrice: {
            num: 1n,
            denom: 10n,
        },
        fee: 500_000n,
        redeemerAddr: myAddr,
        cancellationPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
        permittedExecutors: [],
    });
    const tx = await txBid.sign().complete()
    console.log(`tx bytes: ${tx.toString()}`)
    const txBidId = await tx.submit();
    console.log(txBidId);
    // await sleep(120);
    // const txAsk = await createLimitOrder(lucid, conf.validators!.limitOrder, {
    //     input: {
    //         policy: "",
    //         name: "",
    //     },
    //     output: {
    //         policy: "fd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3a",
    //         name: "74657374746f6b656e",
    //     },
    //     tradableInput: 100_000_000n,
    //     minMarginalOutput: 1_000n,
    //     costPerExStep: 500_000n,
    //     basePrice: {
    //         num: 1n,
    //         denom: 1000n,
    //     },
    //     fee: 500_000n,
    //     redeemerAddr: myAddr,
    //     cancellationPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
    //     permittedExecutors: [],
    // });
    // const txAskId = await (await txAsk.sign().complete()).submit();
    //console.log(txAskId);
}

async function createMToNOrders() {
    // const myAddr = await lucid.wallet.address();
    // console.log("My address: ", lucid.utils.getAddressDetails(myAddr));
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
    //         policy: "fd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3a",
    //         name: "74657374746f6b656e",
    //     },
    //     output: {
    //         policy: "",
    //         name: "",
    //     },
    //     tradableInput: 60_000n,
    //     minMarginalOutput: 1_000n,
    //     costPerExStep: 600_000n,
    //     basePrice: {
    //         num: 1000n,
    //         denom: 1n,
    //     },
    //     fee: 500_000n,
    //     redeemerAddr: myAddr,
    //     cancellationPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
    //     permittedExecutors: [],
    // });
    // const txBidId2 = await (await txBid2.sign().complete()).submit();
    // console.log("Bid #1: " + txBidId2);
    // await sleep(60);
    // const txAsk = await createLimitOrder(lucid, conf.validators!.limitOrder, {
    //     input: {
    //         policy: "",
    //         name: "",
    //     },
    //     output: {
    //         policy: "fd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3a",
    //         name: "74657374746f6b656e",
    //     },
    //     tradableInput: 100_000_000n,
    //     minMarginalOutput: 1_000n,
    //     costPerExStep: 500_000n,
    //     basePrice: {
    //         num: 1n,
    //         denom: 1000n,
    //     },
    //     fee: 500_000n,
    //     redeemerAddr: myAddr,
    //     cancellationPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
    //     permittedExecutors: [],
    // });
    // const txAskId = await (await txAsk.sign().complete()).submit();
    // console.log("Ask #0: " + txAskId);
}

//main();
