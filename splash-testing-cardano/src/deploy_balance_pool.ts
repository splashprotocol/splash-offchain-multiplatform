import { getConfig } from "./config.ts";
import { getLucid } from "./lucid.ts";
import { Asset, BuiltValidators } from "./types.ts";
import { setupWallet } from "./wallet.ts";

export type CreationResponse = {
    result: String
}

export type TokenInfo = {
    cs: String,
    script: String
}

export type PoolDatum = {
    assetX: Asset,
    assetY: Asset,
    weightX: number
}

const TokenA = "Ada"
const TokenB = "testC"

async function main() {
    const lucid = await getLucid();
    await setupWallet(lucid);
    console.log("hello world");

    const conf = await getConfig<BuiltValidators>();

    const balancePoolValidator = conf.validators!.balancePool;
    
    const orderAddress = lucid.utils.credentialToAddress(
        { hash: balancePoolValidator.hash, type: 'Script' },
      );

    const utxos = (await lucid.wallet.getUtxos())[0];

    console.log(`utxos ${utxos.txHash}`);

    console.log(`pool addr ${orderAddress}`);
    
    const creationRes = await getMintingTokenInfo<CreationResponse>(new URL("http://88.99.59.114:8081/getData/"), "d2e7c4c0bde5fd8e4bb964e36c6ba3296adff6ba8d73833f43c4477466ca8c27", 2, "4f534f43494554595f4144415f4e4654", "1");
    console.log(`creationRes ${creationRes}`);

    const tokenInfoRes = await getMintingTokenInfo<TokenInfo>(new URL("http://88.99.59.114:3490/getData/"), "d2e7c4c0bde5fd8e4bb964e36c6ba3296adff6ba8d73833f43c4477466ca8c27", 2, "4f534f43494554595f4144415f4e4654", "1");
    console.log(`tokenInfoRes ${tokenInfoRes}`);
}

async function getMintingTokenInfo<T>(url: URL, txId: String, outIdx: number, tn: String, qty: String): Promise<String> {
    return fetch(url, {
        method: 'POST',
        body: `{"txRef":"${txId}","outId":${outIdx},"tnName":"${tn}","qty":"${qty}"}`,
        headers: {'Content-Type': 'application/json; charset=UTF-8'} }).then(response => {
            if (!response.ok) {
              throw new Error(response.statusText)
            }
            return response.json() as Promise<{ data: T }>
          })
          .then(data => {
              const test = JSON.stringify( data );
              return JSON.stringify( data )
          })
}
  
main();