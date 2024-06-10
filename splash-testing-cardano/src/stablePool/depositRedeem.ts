import { getConfig } from "../config.ts";
import { getLucid } from "../lucid.ts";
import { BuiltValidators } from "../types.ts";
import { setupWallet } from "../wallet.ts";
import { deposit } from "../balance/depositRedeem.ts";

async function main() {

    const lucid = await getLucid();
    await setupWallet(lucid);
    const conf = await getConfig<BuiltValidators>();

    const myAddr = await lucid.wallet.address();
    console.log("My address: ", lucid.utils.getAddressDetails(myAddr));

    let depositTx = await deposit(
        lucid,
        conf.validators!.DepositT2t2tStableDepositT2t,
        {
            poolnft: {
                policy: "875bd843b8adf6f2c494cad655552b1ed81c91f030e2106afc5560bf",
                name: "6e6674",
            },
            x: [{
                policy: "",
                name: "",
            }, 10_000_000n],
            y: [{
                policy: "4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26",
                name: "7465737443",
            }, 10_000_000n],
            lq: {
                policy: "5a6e35e842d82500f37cab442501408fd9df10f9ba65596a079d2318",
                name: "6c71",
            },
            exFee: 1_500_000n,
            rewardPkh: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
            stakePkh: lucid.utils.getAddressDetails(myAddr).stakeCredential!.hash,
            collateralAda: 1_500_000n
        }
    )
    const txId = await (await depositTx.sign().complete()).submit();
    console.log(`tx: ${txId}`)
}

main();