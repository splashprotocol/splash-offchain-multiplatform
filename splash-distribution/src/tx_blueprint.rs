use std::collections::{HashMap, HashSet};
use cml_chain::plutus::PlutusData;
use cml_chain::RequiredSigners;
use cml_chain::transaction::{TransactionInput, TransactionOutput};
use bloom_offchain_cardano::execution_engine::execution_state::{ScalingFactor, ScriptInputBlueprint};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain_cardano::deployment::DeployedValidatorErased;
use spectrum_offchain_cardano::script::{DelayedRedeemer, ScriptWitness};

pub struct InputBlueprint {
    pub reference: OutputRef,
    pub utxo: TransactionOutput,
    pub required_signers: RequiredSigners,
}

pub struct DistributionTxBlueprint {
    pub script_io: Vec<(ScriptInputBlueprint, TransactionOutput)>,
    pub buffered_wallets: Vec<(InputBlueprint, Option<TransactionOutput>)>,
    pub reference_inputs: HashSet<(TransactionInput, TransactionOutput)>,
    pub witness_scripts: HashMap<DeployedValidatorErased, (PlutusData, ScalingFactor)>,
}
