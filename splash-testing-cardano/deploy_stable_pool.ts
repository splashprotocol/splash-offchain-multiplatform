import { PoolValidatePool } from "./plutus.ts";
import { getConfig } from "./src/config.ts";
import { getLucid } from "./src/lucid.ts";
import { Asset, BuiltValidators, PubKeyHash, ScriptHash, asUnit } from "./types.ts";
import { setupWallet } from "./src/wallet.ts";
import { Unit, Datum, MintingPolicy, UTxO, Script, PlutusVersion, Assets, Data, Lucid, fromText, List } from 'https://deno.land/x/lucid@0.10.7/mod.ts';
import { encoder } from 'npm:js-encoding-utils';

export type PoolDatum = {
    // Identifier of the pool | Immutable:
    pool_nft: Asset,
    // Number of tradable assets in the pool | Immutable:
    n: bigint,
    // Identifiers of the tradable assets | Immutable:
    tradable_assets: List<Asset>,
    // Precision multipliers for calculations, i.e. precision / decimals.
    // Precision must be fixed as maximum value of tradable tokens decimals | Immutable:
    tradable_tokens_multipliers: List<bigint>,
    // Identifier of the liquidity token, representing user's share in the pool | Immutable:
    lp_token: Asset,
    // Flag if liquidity provider fee is editable | Immutable:
    lp_fee_is_editable: boolean,
    // Invariant's amplification coefficient | Mutable:
    ampl_coeff: bigint,
    // Numerator of the liquidity provider fee | Mutable:
    lp_fee_num: bigint,
    // Numerator of the protocol fee share | Mutable:
    protocol_fee_num: bigint,
    // Information about the DAO script, which audits the correctness of the "DAO-actions" with stable pool | Mutable:
    dao_stabe_proxy_witness: List<ScriptHash>,
    // Treasury address | Mutable:
    treasury_address: ScriptHash,
    // Collected (and currently available) protocol fees in the tradable assets native units | Mutable:
    protocol_fees: List<bigint>,
    // Actual value of the pool's invariant | Mutable:
    inv: bigint,
    }
export const stringToHex = (str: string): string =>
  encoder.arrayBufferToHexString(encoder.stringToArrayBuffer(str));

// Allowed for editing
const TokenB   = "TOADA"
const TokenBCS = "8a72cdb17e0fd67332e158296bd9ab17bf441841bdf6e5192cec19c3"

const startLovelaceValue = 100000000
const startTokenB        = 100

const lqEmission = 340282366920938463463374607431768211455n;

// do not touch
const feeDen = 100000
const TokenA = "Ada"
const nftTNBase16 = `4f534f43494554595f4144415f4e4654`;
const lqTNBase16 = `4f534f43494554595f4144415f4c4f`;
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
    const stablePoolScript = new PoolValidatePool(`544553545f44414f5f564f54494e475f57`);
    const stablePoolScriptHash = lucid.utils.validatorToScriptHash(stablePoolScript)
    const poolAddress = lucid.utils.credentialToAddress(
        { hash: stablePoolScriptHash, type: 'Script' },
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
    const ampl_coeff = 1000n;
    const lp_fee_num = 500n;
    const protocol_fee_num = 30n;
    const PoolDatum = await buildPoolConfig(lucid, [startLovelaceValue, startTokenB], [1n, 1000000n], true, ampl_coeff, lp_fee_num, protocol_fee_num, nftInfo.policyId, lqInfo.policyId);

    const deposited_lp = BigInt(lqEmission) - BigInt(PoolDatum.inv)
    const depositedValue = { 
        lovelace: BigInt(startLovelaceValue),
        [asUnit(PoolDatum.tradable_assets[1])]: BigInt(startTokenB),
        [asUnit(PoolDatum.pool_nft)]: BigInt(nftEmission),
        [asUnit(PoolDatum.lp_token)]: BigInt(deposited_lp)
    }

    console.log(`depositedValue: ${JSON.stringify(depositedValue, stringifyBigIntReviewer)}`)

    console.log(`token box: ${JSON.stringify(boxWithToken, stringifyBigIntReviewer)}`);
    console.log(`ada box: ${JSON.stringify(boxWithAda, stringifyBigIntReviewer)}`);
    console.log(`poolAddress: ${JSON.stringify(poolAddress, stringifyBigIntReviewer)}`);
    const poolD = buildPoolDatum(lucid, PoolDatum)
    console.log(`PoolDatum: ${JSON.stringify(PoolDatum, stringifyBigIntReviewer)}`)
    console.log(`depositedValue: ${JSON.stringify(depositedValue, stringifyBigIntReviewer)}`)
    console.log(`mintingNftAssets: ${JSON.stringify(mintingNftAssets, stringifyBigIntReviewer)}`)
    console.log(`mintingLqAssets: ${JSON.stringify(mintingLqAssets, stringifyBigIntReviewer)}`)

    const tx = await lucid.newTx().collectFrom([boxWithToken!, boxWithAda!])
        .attachMintingPolicy(nftMintingPolicy)
        .mintAssets(mintingNftAssets, Data.to(0n))
        .attachMintingPolicy(lqMintingPolicy)
        .mintAssets(mintingLqAssets, Data.to(0n))
        .payToContract(poolAddress, { inline: poolD }, depositedValue)
        .complete();

    const txId = await (await tx.sign().complete()).submit();

    console.log(`tx: ${txId}`)
}


async function buildPoolConfig(lucid: Lucid, initial_reserves: List<bigint>, tradable_tokens_multipliers: List<bigint>, lp_fee_is_editable: boolean, ampl_coeff: bigint, lp_fee_num: bigint, protocol_fee_num: bigint, nftCS: string, lqCS: string): Promise<PoolDatum> {

    const myAddr = await lucid.wallet.address();

    const inv = BigInt(initial_reserves[0]) + BigInt(initial_reserves[1]) * 1000000n;
    console.log(`inv: ${JSON.stringify(inv, stringifyBigIntReviewer)}`);
    const dao = await getDAOPolicy(nftCS)

    return {
        pool_nft: {
            policy: nftCS,
            name: nftTNBase16,
        },
        n: 2n,
        // for tests pool x is always ada?
        tradable_assets: [
            {policy: "",
            name: "",},
            {policy: TokenBCS,
            name: encodedTestB,}],
        tradable_tokens_multipliers: tradable_tokens_multipliers,
        lp_token: {
            policy: lqCS,
            name: lqTNBase16,
        },
        lp_fee_is_editable: lp_fee_is_editable,
        ampl_coeff:  ampl_coeff,
        lp_fee_num: lp_fee_num,
        protocol_fee_num: protocol_fee_num,
        dao_stabe_proxy_witness: [dao.curSymbol],
        protocol_fees: [0n, 0n],
        // incorrect treasury address. change to script
        treasury_address: lucid.utils.getAddressDetails(myAddr).paymentCredential!.hash,
        inv: inv,
    }
}
function buildPoolDatum(lucid: Lucid, conf: PoolDatum): Datum {
    return Data.to({
        poolNft: conf.pool_nft,
        n: BigInt(conf.n),
        tradableAssets: conf.tradable_assets,
        tradableTokensMultipliers: [BigInt(conf.tradable_tokens_multipliers[0]), BigInt(conf.tradable_tokens_multipliers[1])],
        lpToken: conf.lp_token,
        lpFeeIsEditable: Boolean(conf.lp_fee_is_editable),
        amplCoeff: BigInt(conf.ampl_coeff),
        lpFeeNum: BigInt(conf.lp_fee_num),
        protocolFeeNum: BigInt(conf.protocol_fee_num),
        daoStabeProxyWitness: conf.dao_stabe_proxy_witness,
        treasuryAddress: conf.treasury_address,
        protocolFees: [BigInt(conf.protocol_fees[0]), BigInt(conf.protocol_fees[1])],
        inv: BigInt(conf.inv),
    }, PoolValidatePool.inputDatum)
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