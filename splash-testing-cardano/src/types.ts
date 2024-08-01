import { PolicyId, Script, UTxO, Unit } from "https://deno.land/x/lucid@0.10.7/mod.ts";

export type PubKeyHash = string

export type Rational = {
    num: bigint,
    denom: bigint,
}

export type Asset = {
    policy: PolicyId,
    name: string,
}

export function asUnit(asset: Asset): Unit {
    return asset.policy + asset.name;
}

export type Market = {
    base: Asset,
    quote: Asset
}

export type BuiltValidator = {
    script: Script;
    hash: string;
};

export type DeployedValidator = BuiltValidator & {
    referenceUtxo: UTxO;
};

export type ScriptNames = "admin" | "pool" | "factory" | "ammPool";
export type BuiltValidators = Record<ScriptNames, BuiltValidator>;
export type DeployedValidators = Record<ScriptNames, DeployedValidator>;


export type MintingInfo = {
    asset: Asset,
    mintingValue: Record<Unit | "lovelace", bigint>,
    mintingRedeemer: string
}

export type DegenPoolInfo = {
    degenDatum: {
        poolNft: { policy: string; name: string };
        assetX: Asset;
        assetY: Asset;
        aNum: bigint;
        bNum: bigint;
        batcherPk: string;
        adaCapThr: bigint;
        adminWitness: string;
        factoryWitness: string;
    },
    degenValue: Assets
    txHash: TxHash;
    outputIndex: number;
    utxo: UTxO
}