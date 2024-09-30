import { BalanceContract } from "../plutus.ts";
import { getConfig } from "./config.ts";
import { getLucid } from "./lucid.ts";
import { Asset, BuiltValidators, PubKeyHash, asUnit } from "./types.ts";
import { setupWallet } from "./wallet.ts";
import { Unit, Datum, MintingPolicy, UTxO, Script, PlutusVersion, Assets, Data, Lucid, C, fromHex} from "@lucid-evolution/lucid";
import { encoder } from 'npm:js-encoding-utils';

export const stringToHex = (str: string): string =>
  encoder.arrayBufferToHexString(encoder.stringToArrayBuffer(str));

// Allowed for editing
const TokenB   = "testC"
const TokenBCS = "4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26"

const startLovelaceValue = 100000000
const startTokenB        = 100000000

const lqEmission = 9223372036854775807n;

const lqFee = 99970
const treasuryFee = 10

// token b weight will be equals to `10 - adaWeight`
const adaWeight = 2;

// do not touch
const feeDen = 100000
const weigtDen = 10
const tokenBWeight = 10 - adaWeight;
const TokenA = "Ada"
const nftTNBase16 = `6e6674`;
const lqTNBase16 = `6c71`;
const nftEmission = 1n;
const encodedTestB = stringToHex(TokenB);

export type CreationResponse = {
    result: string
}

export type TokenInfo = {
    policyId: string,
    script: string
}

export type DAOInfo = {
    curSymbol: string,
}

export type PoolConfig = {
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
    // treasuryAddress - is contract
    treasuryAddress: PubKeyHash,
    invariant: number
}

const stringifyBigIntReviewer = (_: any, value: any) =>
  typeof value === 'bigint'
    ? { value: value.toString(), _bigint: true }
    : value;

async function main() {

    const lucid = await getLucid();
    await setupWallet(lucid);

    const conf = await getConfig<BuiltValidators>();
    
    const utxos = (await lucid.wallet.getUtxos());

    const boxWithToken = await getUtxoWithToken(utxos, encodedTestB);
    const boxWithAda   = await getUtxoWithAda(utxos)

    const nftInfo = await getCSAndSсript(boxWithToken.txHash, boxWithToken.outputIndex, nftTNBase16, `${nftEmission}`);
    const lqInfo  = await getCSAndSсript(boxWithToken.txHash, boxWithToken.outputIndex, lqTNBase16, `${lqEmission}`);

    console.log(`nft info: ${nftInfo}`);

    console.log(`address: ${await lucid.wallet.address()}`);

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

    const lqUnit: Unit  = `${lqInfo.policyId.concat(lqTNBase16)}`;
    const nftUnit: Unit = `${nftInfo.policyId.concat(nftTNBase16)}`;

    console.log(`lq: ${lqUnit}`);
    console.log(`nftUnit: ${nftUnit}`);

    const mintingLqAssets: Record<Unit | "lovelace", bigint> = 
        {
            [lqUnit]: lqEmission
            //nftUnit: 1n
        }

    const mintingNftAssets: Record<Unit | "lovelace", bigint> = 
        {
            [nftUnit]: nftEmission
            //nftUnit: 1n
        }
    
    const poolConfig = await buildPoolConfig(lucid, startLovelaceValue, adaWeight, startTokenB, tokenBWeight, nftInfo.policyId, lqInfo.policyId);

    console.log(`poolConfig: ${JSON.stringify(poolConfig)}`)

    const depositedValue = { 
        lovelace: BigInt(startLovelaceValue),
        [asUnit(poolConfig.poolY)]: BigInt(startTokenB),
        [asUnit(poolConfig.poolNft)]: nftEmission,
        [asUnit(poolConfig.poolLq)]: lqEmission
    }

    console.log(`depositedValue: ${JSON.stringify(depositedValue, stringifyBigIntReviewer)}`)

    console.log(`token box: ${JSON.stringify(boxWithToken, stringifyBigIntReviewer)}`);
    console.log(`ada box: ${JSON.stringify(boxWithAda, stringifyBigIntReviewer)}`);

    const tx = await lucid.newTx().collectFrom([boxWithToken!, boxWithAda!])
        .attachMintingPolicy(nftMintingPolicy)
        .mintAssets(mintingNftAssets, Data.to(0n))
        .attachMintingPolicy(lqMintingPolicy)
        .mintAssets(mintingLqAssets, Data.to(0n))
        .payToContract(poolAddress, { inline: buildPoolDatum(lucid, poolConfig) }, depositedValue)
        .complete();

    console.log(`poolConfig: ${JSON.stringify(poolConfig)}`)

    const txId = await (await tx.sign().complete()).submit();

    console.log(`tx: ${txId}`)
}

async function buildPoolConfig(lucid: Lucid, xQty: number, xWeight: number, yQty: number, yWeight: number, nftCS: string, lqCS: string): Promise<PoolConfig> {

    const myAddr = await lucid.wallet.address();

    const invariant = Math.round(Math.pow(xQty, (xWeight / weigtDen)) * Math.pow(yQty, (yWeight / weigtDen)));

    const dao = await getDAOPolicy(nftCS)

    const scriptCred: C.StakeCredential = C.StakeCredential.from_scripthash(
        C.ScriptHash.from_bytes(
          fromHex(dao.curSymbol),
        )
      );

    return {
        poolNft: {
            policy: nftCS,
            name: nftTNBase16,
        },
        // for tests pool x is always ada?
        poolX: {
            policy: "",
            name: "",
        },
        weightX: adaWeight,
        poolY: {
            policy: TokenBCS,
            name: encodedTestB,
        },
        weightY: tokenBWeight,
        poolLq: {
            policy: lqCS,
            name: lqTNBase16,
        },
        feeNum: lqFee,
        treasuryFee: treasuryFee,
        treasuryX: 0,
        treasuryY: 0,
        DAOPolicy: [{
            Inline: [{ ScriptCredential: [dao.curSymbol] }]
        }],
        // incorrect treasury address. change to script
        treasuryAddress: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
        invariant: invariant
    }
}

function buildPoolDatum(lucid: Lucid, conf: PoolConfig): Datum {
    return Data.to({
        poolnft: conf.poolNft,
        poolx: conf.poolX,
        weightX: BigInt(conf.weightX),
        poolY: conf.poolY,
        weightY: BigInt(conf.weightY),
        poolLq: conf.poolLq,
        feenum: BigInt(conf.feeNum),
        treasuryFee: BigInt(conf.treasuryFee),
        treasuryx: BigInt(conf.treasuryX),
        treasuryy: BigInt(conf.treasuryY),
        daoPolicy: conf.DAOPolicy,
        treasuryAddress: conf.treasuryAddress,
        invariant: BigInt(conf.invariant),
    }, BalanceContract.conf)
}

async function getUtxoWithToken(utxos: UTxO[], token2find: string) {
    return utxos.find( utxo =>
            {
                return (Object.keys(utxo.assets) as Array<String>).find(key => key.includes(token2find)) !== undefined
            }
        )
}

async function getUtxoWithAda(utxos: UTxO[]) {
    return utxos.find( utxo =>
            {
                return ((utxo.assets["lovelace"] > startLovelaceValue))
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

async function getDAOPolicy(nftCS: string): Promise<DAOInfo> {
    return getDAO<DAOInfo>(new URL("http://88.99.59.114:8085/dao/"), nftCS, nftTNBase16);
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

async function getDAO<T>(url: URL, nftCS: string, nftTN: string): Promise<T> {
    return fetch(url, {
        method: 'POST',
        body: `{"nftCS":"${nftCS}","nftTN":"${nftTN}"}`,
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