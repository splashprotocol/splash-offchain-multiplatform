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
                policy: "89bcd86d86a82b7f8056ae9d413294742dfdeb6bd6c2a156ebb507a5",
                name: "6e6674",
            },
            x: [{
                policy: "",
                name: "",
            }, 1000_000_000n],
            y: [{
                policy: "4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26",
                name: "7465737443",
            }, 1000_000_000n],
            lq: {
                policy: "27dfc16384cb64ca3fd0340f5bfdfcf4f77591d59bfca01945b51099",
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