use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter, Write};

use bloom_offchain::execution_engine::funding_effect::FundingIO;
use bloom_offchain::execution_engine::liquidity_book::types::Lovelace;
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{TransactionBuilder, TransactionUnspentOutput};
use cml_chain::builders::withdrawal_builder::SingleWithdrawalBuilder;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::certs::Credential;
use cml_chain::plutus::{PlutusData, RedeemerTag};
use cml_chain::transaction::{TransactionInput, TransactionOutput};
use cml_chain::{RequiredSigners, Value};
use either::Either;
use log::trace;
use spectrum_cardano_lib::funding::OperatorFunding;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, NetworkId, OutputRef};
use spectrum_offchain_cardano::constants::MIN_SAFE_LOVELACE_VALUE;
use spectrum_offchain_cardano::creds::OperatorRewardAddress;
use spectrum_offchain_cardano::deployment::DeployedValidatorErased;
use spectrum_offchain_cardano::script::{
    DelayedRedeemer, ScriptContextPreview, ScriptWitness, TxInputsOrdering,
};

pub struct ScriptInputBlueprint {
    pub reference: OutputRef,
    pub utxo: TransactionOutput,
    pub script: ScriptWitness,
    pub redeemer: DelayedRedeemer,
    pub required_signers: RequiredSigners,
}

pub type ScalingFactor = u64;

/// Blueprint of a DEX transaction.
/// Accumulates information that is later used to create real transaction
/// that executes a batch of DEX operations.
pub struct TxBlueprint {
    pub script_io: Vec<(ScriptInputBlueprint, TransactionOutput)>,
    pub reference_inputs: HashSet<(TransactionInput, TransactionOutput)>,
    pub witness_scripts: HashMap<DeployedValidatorErased, (PlutusData, ScalingFactor)>,
}

impl Display for TxBlueprint {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("TxBlueprint(io: ")?;
        for (ix, (i, o)) in self.script_io.iter().enumerate() {
            f.write_str("[")?;
            f.write_str(
                format!(
                    "Input{}(ref: {}, addr: {}, value: {}), ",
                    ix,
                    i.reference,
                    i.utxo
                        .address()
                        .to_bech32(None)
                        .unwrap_or_else(|_| "_".to_string()),
                    serde_json::to_string(i.utxo.value()).unwrap_or_else(|_| "-".to_string())
                )
                .as_str(),
            )?;
            f.write_str(
                format!(
                    "Output{}(addr: {}, value: {})",
                    ix,
                    o.address().to_bech32(None).unwrap_or_else(|_| "-".to_string()),
                    serde_json::to_string(o.value()).unwrap_or_else(|_| "-".to_string())
                )
                .as_str(),
            )?;
            f.write_str("]")?;
        }
        f.write_str(")")
    }
}

impl TxBlueprint {
    pub fn new() -> Self {
        Self {
            script_io: Vec::new(),
            reference_inputs: HashSet::new(),
            witness_scripts: HashMap::new(),
        }
    }

    pub fn add_io(&mut self, input: ScriptInputBlueprint, output: TransactionOutput) {
        self.script_io.push((input, output));
    }

    pub fn add_ref_input(&mut self, utxo: TransactionUnspentOutput) {
        self.reference_inputs.insert((utxo.input, utxo.output));
    }

    pub fn add_witness(&mut self, wit: DeployedValidatorErased, redeemer: PlutusData) {
        match self.witness_scripts.entry(wit) {
            Entry::Occupied(mut e) => {
                let (_, sf) = e.get_mut();
                *sf += 1;
            }
            Entry::Vacant(e) => {
                e.insert((redeemer, 1));
            }
        }
    }

    pub fn project_onto_builder(
        self,
        mut txb: TransactionBuilder,
        network_id: NetworkId,
        operator_address: OperatorRewardAddress,
        operator_funding: FinalizedTxOut,
        operator_interest: u64,
    ) -> (TransactionBuilder, FundingIO<FinalizedTxOut, TransactionOutput>) {
        let TxBlueprint {
            script_io,
            reference_inputs,
            witness_scripts,
        } = self;
        let mut all_io = script_io.into_iter().map(Either::Left).collect::<Vec<_>>();
        let funding_io = if operator_interest > 0 {
            if operator_interest >= MIN_SAFE_LOVELACE_VALUE {
                let reward_dest_address = operator_address.address();
                let reward_dest_coincides_with_funding = reward_dest_address == *operator_funding.0.address();
                let operator_output =
                    TransactionOutput::new(reward_dest_address, Value::from(operator_interest), None, None);
                all_io.push(Either::Right((None, operator_output.clone())));
                if reward_dest_coincides_with_funding {
                    // If reward dest address coincides with funding address, then it can be used as funding.
                    FundingIO::Added(operator_funding, operator_output)
                } else {
                    FundingIO::NotUsed(operator_funding)
                }
            } else {
                // If funding utxo has to be used it is replaced with `operator_output`.
                let reward_dest_address = operator_funding.0.address().clone();
                let mut value = operator_funding.0.value().clone();
                value.add_unsafe(AssetClass::Native, operator_interest);
                let operator_output = TransactionOutput::new(reward_dest_address, value, None, None);
                all_io.push(Either::Right((
                    Some(operator_funding.clone()),
                    operator_output.clone(),
                )));
                FundingIO::Replaced(operator_funding, operator_output)
            }
        } else {
            FundingIO::NotUsed(operator_funding)
        };
        all_io.sort_by(|lh, rh| match (lh, rh) {
            (Either::Left((lh_in, _)), Either::Left((rh_in, _))) => lh_in.reference.cmp(&rh_in.reference),
            (Either::Left((lh_in, _)), Either::Right((Some(rh_in), _))) => {
                lh_in.reference.cmp(&rh_in.reference())
            }
            (Either::Right((Some(lh_in), _)), Either::Left((rh_in, _))) => {
                lh_in.reference().cmp(&rh_in.reference)
            }
            (_, Either::Right((None, _))) => Ordering::Less,
            _ => Ordering::Greater,
        });
        let enumerated_io = all_io.into_iter().enumerate().collect::<Vec<_>>();
        let inputs_ordering = TxInputsOrdering::new(HashMap::from_iter(enumerated_io.iter().filter_map(
            |(ix, io)| match io {
                Either::Left((i, _)) => Some((i.reference, *ix)),
                Either::Right((Some(i), _)) => Some((i.reference(), *ix)),
                _ => None,
            },
        )));
        for (ref_in, ref_utxo) in reference_inputs {
            txb.add_reference_input(TransactionUnspentOutput::new(ref_in, ref_utxo));
        }
        for (ix, io) in enumerated_io {
            match io {
                Either::Left((
                    ScriptInputBlueprint {
                        reference,
                        utxo,
                        script,
                        redeemer,
                        required_signers,
                    },
                    output,
                )) => {
                    let cml_script = PartialPlutusWitness::new(
                        PlutusScriptWitness::Ref(script.hash),
                        redeemer.compute(&inputs_ordering),
                    );
                    let input = SingleInputBuilder::new(reference.into(), utxo)
                        .plutus_script_inline_datum(cml_script, required_signers)
                        .unwrap();
                    let output = SingleOutputBuilderResult::new(output);
                    txb.add_input(input).expect("add script input ok");
                    trace!(
                        "Adding output: {} -> {}",
                        serde_json::to_string(output.output.value()).unwrap(),
                        output
                            .output
                            .address()
                            .to_bech32(None)
                            .unwrap_or_else(|_| "_".to_string())
                    );
                    txb.add_output(output).expect("add script output ok");
                    let ctx = ScriptContextPreview { self_index: ix };
                    txb.set_exunits(
                        RedeemerWitnessKey::new(RedeemerTag::Spend, ix as u64),
                        script.cost.compute(&ctx).into(),
                    );
                }
                Either::Right((maybe_funding_input, funding_output)) => {
                    if let Some(FinalizedTxOut(utxo, reference)) = maybe_funding_input {
                        let input = SingleInputBuilder::new(reference.into(), utxo)
                            .payment_key()
                            .unwrap();
                        txb.add_input(input).expect("add funding input ok");
                    }
                    let output = SingleOutputBuilderResult::new(funding_output);
                    txb.add_output(output).expect("add funding output ok");
                }
            }
        }
        // Project common witness scripts.
        for (wit, (rdmr, scaling_factor)) in witness_scripts {
            let reward_address =
                cml_chain::address::RewardAddress::new(network_id.into(), Credential::new_script(wit.hash));
            let partial_witness = PartialPlutusWitness::new(PlutusScriptWitness::Ref(wit.hash), rdmr);
            let withdrawal_result = SingleWithdrawalBuilder::new(reward_address, 0)
                .plutus_script(partial_witness, vec![].into())
                .unwrap();
            txb.add_reference_input(wit.reference_utxo);
            txb.add_withdrawal(withdrawal_result);
            trace!(
                "Witness: Base cost: {:?}, marginal cost {:?}, scaling factor: {}",
                wit.ex_budget,
                wit.marginal_cost,
                scaling_factor
            );
            let ex_units = wit.ex_budget + wit.marginal_cost.scale(scaling_factor);
            txb.set_exunits(RedeemerWitnessKey::new(RedeemerTag::Reward, 0), ex_units.into());
        }
        (txb, funding_io)
    }
}

pub struct ExecutionState {
    pub tx_blueprint: TxBlueprint,
    pub reserved_tx_fee: Lovelace,
    pub operator_interest: Lovelace,
}

impl ExecutionState {
    pub fn new() -> Self {
        Self {
            tx_blueprint: TxBlueprint::new(),
            reserved_tx_fee: 0,
            operator_interest: 0,
        }
    }

    pub fn add_tx_fee(&mut self, amount: Lovelace) {
        self.reserved_tx_fee += amount;
    }

    pub fn add_operator_interest(&mut self, amount: Lovelace) {
        self.operator_interest += amount;
    }
}

#[cfg(test)]
mod test {
    use cml_chain::plutus::PlutusV2Script;
    use cml_core::serialization::Deserialize;

    #[test]
    fn hash_script_cml() {
        let script = PlutusV2Script::from_cbor_bytes(&*hex::decode(SCRIPT).unwrap()).unwrap();
        let sh = script.hash();
        println!("{}", sh)
    }

    const SCRIPT: &str = "59041459041101000033232323232323232322222323253330093232533300b003132323300100100222533301100114a02646464a66602266ebc0380045288998028028011808801180a80118098009bab301030113011301130113011301130090011323232533300e3370e900118068008991919299980899b8748000c0400044c8c8c8c8c94ccc0594ccc05802c400852808008a503375e601860260046034603660366036603660366036603660366036602602266ebcc020c048c020c048008c020c048004c060dd6180c180c980c9808804980b80098078008b19191980080080111299980b0008a60103d87a80001323253330153375e6018602600400c266e952000330190024bd70099802002000980d001180c0009bac3007300e0063014001300c001163001300b0072301230130013322323300100100322533301200114a026464a66602266e3c008014528899802002000980b0011bae3014001375860206022602260226022602260226022602260120026eb8c040c044c044c044c044c044c044c044c044c044c044c02401cc004c0200108c03c004526136563370e900118049baa003323232533300a3370e90000008991919191919191919191919191919191919191919191919299981298140010991919191924c646600200200c44a6660560022930991980180198178011bae302d0013253330263370e9000000899191919299981698180010991924c64a66605866e1d20000011323253330313034002132498c94ccc0bccdc3a400000226464a666068606e0042649318150008b181a80098168010a99981799b87480080044c8c8c8c8c8c94ccc0e0c0ec00852616375a607200260720046eb4c0dc004c0dc008dd6981a80098168010b18168008b181900098150018a99981619b874800800454ccc0bcc0a800c5261616302a002302300316302e001302e002302c00130240091630240083253330253370e9000000899191919299981618178010a4c2c6eb4c0b4004c0b4008dd6981580098118060b1811805980d806180d0098b1bac30260013026002375c60480026048004604400260440046eb4c080004c080008c078004c078008c070004c070008dd6980d000980d0011bad30180013018002375a602c002602c004602800260280046eb8c048004c048008dd7180800098040030b1804002919299980519b87480000044c8c8c8c94ccc044c05000852616375c602400260240046eb8c040004c02000858c0200048c94ccc024cdc3a400000226464a66601c60220042930b1bae300f0013007002153330093370e900100089919299980718088010a4c2c6eb8c03c004c01c00858c01c0048c014dd5000918019baa0015734aae7555cf2ab9f5740ae855d126126d8799fd87a9f581ce7feddaece029040c973d5bf806fa9497314c0a63dfdc47fc47ac557ffff0001";
}
