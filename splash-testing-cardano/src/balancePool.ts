import { getConfig } from "./config.ts";
import { getLucid } from "./lucid.ts";
import { Asset, BuiltValidators, PubKeyHash } from "./types.ts";
import { setupWallet } from "./wallet.ts";
import { type Unit, MintingPolicy, UTxO, Script, PlutusVersion, Assets, Data } from "https://deno.land/x/lucid@0.10.7/mod.ts";
import { encoder } from 'npm:js-encoding-utils';

export type CreationResponse = {
    result: string
}

export type TokenInfo = {
    policyId: string,
    script: string
}

export type PoolDatum = {
    poolNft: Asset,
    poolX: Asset,
    weightX: number,
    poolY: Asset,
    weightY: number,
    poolLq: Asset,
    feeNum: number,
    treasuryFee: number,
    treasuryX: number,
    treasuryY: number,
    DAOPolicy: PubKeyHash[],
    treasuryAddress: PubKeyHash,
    invariant: number
}

export const stringToHex = (str: string): string =>
  encoder.arrayBufferToHexString(encoder.stringToArrayBuffer(str));

const stringifyBigIntReviewer = (_: any, value: any) =>
  typeof value === 'bigint'
    ? { value: value.toString(), _bigint: true }
    : value;

const TokenA = "Ada"
const TokenB = "testC"

const encodedTestB = stringToHex(TokenB);

async function main() {
    const lucid = await getLucid();
    await setupWallet(lucid);

    const conf = await getConfig<BuiltValidators>();
    
    const utxos = (await lucid.wallet.getUtxos());

    const boxWithAdaAndToken = await getUtxoWithToken(utxos, encodedTestB);

    const nftInfo = await getCSAndSсript(boxWithAdaAndToken.txHash, boxWithAdaAndToken.outputIndex, "6e6674", "1");
    const lqInfo  = await getCSAndSсript(boxWithAdaAndToken.txHash, boxWithAdaAndToken.outputIndex, "6c71", "9223372036854775807");

    console.log(`nft info: ${nftInfo}`);

    //console.log(`lqInfo info: ${lqInfo}`);

    const poolAddress = lucid.utils.credentialToAddress(
        { hash: conf.validators!.balancePool.hash, type: 'Script' },
      );

    const nftMintingPolicy: MintingPolicy = 
        {
            type: "PlutusV2",
            script: nftInfo.script
        }

    const lqMintingPolicy: MintingPolicy = 
        {
            type: "PlutusV2",
            script: lqInfo.script
        }

    const lqUnit: Unit = `${lqInfo.policyId.concat(`6c71`)}`;
    const nftUnit: Unit = `${nftInfo.policyId.concat(`6e6674`)}`;

    console.log(`lq: ${lqUnit}`);
    console.log(`nftUnit: ${nftUnit}`);

    const mintingLqAssets: Record<Unit | "lovelace", bigint> = 
        {
            [lqUnit]: 9223372036854775807n
            //nftUnit: 1n
        }

    const mintingNftAssets: Record<Unit | "lovelace", bigint> = 
        {
            [nftUnit]: 9223372036854775807n
            //nftUnit: 1n
        }
    
    const tx = await lucid.newTx().collectFrom([boxWithAdaAndToken!])
        .attachMintingPolicy(nftMintingPolicy)
        .mintAssets(mintingNftAssets)
        .attachMintingPolicy(lqMintingPolicy)
        .mintAssets(mintingLqAssets)
        .complete();
        //.payToContract(poolAddress, { inline: buildLimitOrderDatum(lucid, conf, beacon) }, depositedValue);

    console.log(`tx: ${tx}`)
}

async function getUtxoWithToken(utxos: UTxO[], token2find: string) {
    return utxos.find( utxo =>
            {
                console.log(`utxo.txHash: ${utxo.txHash}`);
                console.log(`utxo.assets: ${Object.keys(utxo.assets)}`);
                return (Object.keys(utxo.assets) as Array<String>).find(key => key.includes(token2find)) !== undefined
            }
        )
}

async function getCSAndSсript(txId: string, outIdx: number, tn: string, qty: string): Promise<TokenInfo> {

    const res = await getMintingTokenInfo<CreationResponse>(new URL("http://88.99.59.114:8081/getData/"), txId, outIdx, tn, qty);

    console.log(`res: ${res}`);

    const anotherRes = await getMintingTokenInfo<TokenInfo>(new URL("http://88.99.59.114:3490/getData/"), txId, outIdx, tn, qty);

    console.log(`anotherRes: ${anotherRes}`);

    return anotherRes
}

async function getMintingTokenInfo<T>(url: URL, txId: string, outIdx: number, tn: String, qty: string): Promise<T> {
    return fetch(url, {
        method: 'POST',
        body: `{"txRef":"${txId}","outId":${outIdx},"tnName":"${tn}","qty":"${qty}"}`,
        headers: {'Content-Type': 'application/json; charset=UTF-8'} }).then(response => {
            if (!response.ok) {
              throw new Error(response.statusText)
            }
            return response.json() as Promise<T>
          })
          .then(data => {
              console.log(`data: ${JSON.stringify(data)}`);
              return data
          })
}
  
main();