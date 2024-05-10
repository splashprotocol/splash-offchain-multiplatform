import {
  M,
  PolicyId,
  Script,
  UTxO,
} from "https://deno.land/x/lucid@0.10.7/mod.ts";
import {
  IGovProxyGovProxy,
  IInflationInflation,
  ISmartFarmFarmFactory,
  IVeFactoryVeFactory,
  IVotingEscrowVotingEscrow,
  IWeightingPollWpFactory,
} from "../plutus.ts";

export type BuiltValidator = {
  script: Script;
  hash: string;
  exBudget: {
    mem: bigint;
    steps: bigint;
  };
};

export type BuiltPolicy = {
  script: Script;
  policyId: PolicyId;
  assetName: string;
  quantity: bigint;
};

export type ScriptNames =
  | "inflation"
  | "votingEscrow"
  | "farmFactory"
  | "wpFactory"
  | "veFactory"
  | "govProxy"
  | "permManager"
  | "mintWPAuthToken"
  | "weightingPower"
  | "smartFarm";

export type NFTNames =
  | "factory_auth"
  | "gt"
  | "ve_factory_auth"
  | "perm_auth"
  | "proposal_auth"
  | "edao_msig";

export type DeployedValidator = BuiltValidator & {
  referenceUtxo: UTxO;
};

export type BuiltValidators = Record<ScriptNames, BuiltValidator>;
export type DeployedValidators = Record<ScriptNames, DeployedValidator>;
export type NFTDetails = Record<NFTNames, BuiltPolicy>;

// Datums for singleton entities

// farm_factory datum
export type FarmState = {
  last_farm_id: number;
  farm_seed_data: string;
};

export type WPFactoryState = {
  /// Epoch of the last WP.
  last_poll_epoch: number;
  /// Active farms.
  active_farms: string[];
};

export type DaoInput = {
  inflation: IInflationInflation["epoch"];
  votingEscrow: IVotingEscrowVotingEscrow["state"];
  farmFactory: ISmartFarmFarmFactory["state"];
  wpFactory: IWeightingPollWpFactory["state"];
  veFactory: IVeFactoryVeFactory["conf"];
};
