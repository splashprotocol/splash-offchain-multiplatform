import { PolicyId, Script, UTxO, Unit } from "@lucid-evolution/lucid";

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

export type ScriptNames = "limitOrder" | "limitOrderWitness" | "gridOrderNative" | "royaltyPool"; //"limitOrder" | "limitOrderWitness" | "gridOrderNative" | "balancePool" | "balanceDeposit" | "balanceRedeem" | "stablePoolT2T" | "stableDeposit" | "stableRedeem";
export type BuiltValidators = Record<ScriptNames, BuiltValidator>;
export type DeployedValidators = Record<ScriptNames, DeployedValidator>;