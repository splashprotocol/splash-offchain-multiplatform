use cml_chain::{
    auxdata::{AuxiliaryData, ConwayFormatAuxData},
    certs::Certificate,
    transaction::{
        cbor_encodings::{ConwayFormatTxOutEncoding, TransactionBodyEncoding, TransactionEncoding},
        ConwayFormatTxOut, Transaction, TransactionBody, TransactionOutput, TransactionWitnessSet,
    },
    Script,
};
use cml_multi_era::{
    allegra::AllegraCertificate,
    babbage::{
        cbor_encodings::{
            BabbageFormatAuxDataEncoding, BabbageFormatTxOutEncoding, BabbageTransactionBodyEncoding,
            BabbageTransactionEncoding,
        },
        BabbageAuxiliaryData, BabbageFormatAuxData, BabbageFormatTxOut, BabbageScript, BabbageTransaction,
        BabbageTransactionBody, BabbageTransactionOutput, BabbageTransactionWitnessSet,
    },
};

/// Convert [Transaction] into [BabbageTransaction]
pub fn to_babbage_transaction(tx: Transaction) -> BabbageTransaction {
    BabbageTransaction {
        body: to_babbage_transaction_body(tx.body),
        witness_set: to_babbage_witness_set(tx.witness_set),
        is_valid: tx.is_valid,
        auxiliary_data: tx.auxiliary_data.map(to_babbage_auxiliary_data),
        encodings: tx.encodings.map(to_babbage_transaction_encoding),
    }
}

fn to_babbage_witness_set(ws: TransactionWitnessSet) -> BabbageTransactionWitnessSet {
    BabbageTransactionWitnessSet {
        vkeywitnesses: ws.vkeywitnesses,
        native_scripts: ws.native_scripts,
        bootstrap_witnesses: ws.bootstrap_witnesses,
        plutus_v1_scripts: ws.plutus_v1_scripts,
        plutus_datums: ws.plutus_datums,
        redeemers: ws.redeemers,
        plutus_v2_scripts: ws.plutus_v2_scripts,
        encodings: None,
    }
}

fn to_babbage_transaction_output(output: TransactionOutput) -> BabbageTransactionOutput {
    match output {
        TransactionOutput::AlonzoFormatTxOut(tx_out) => BabbageTransactionOutput::AlonzoFormatTxOut(tx_out),
        TransactionOutput::ConwayFormatTxOut(ConwayFormatTxOut {
            address,
            amount,
            datum_option,
            script_reference,
            encodings,
        }) => BabbageTransactionOutput::BabbageFormatTxOut(BabbageFormatTxOut {
            address,
            amount,
            datum_option,
            script_reference: script_reference.map(to_babbage_script),
            encodings: encodings.map(to_babbage_encoding),
        }),
    }
}

fn to_babbage_script(script: Script) -> BabbageScript {
    match script {
        Script::Native {
            script,
            len_encoding,
            tag_encoding,
        } => BabbageScript::Native {
            script,
            len_encoding,
            tag_encoding,
        },
        Script::PlutusV1 {
            script,
            len_encoding,
            tag_encoding,
        } => BabbageScript::PlutusV1 {
            script,
            len_encoding,
            tag_encoding,
        },
        Script::PlutusV2 {
            script,
            len_encoding,
            tag_encoding,
        } => BabbageScript::PlutusV2 {
            script,
            len_encoding,
            tag_encoding,
        },
        Script::PlutusV3 {
            script: _,
            len_encoding: _,
            tag_encoding: _,
        } => unreachable!(),
    }
}

fn to_babbage_encoding(enc: ConwayFormatTxOutEncoding) -> BabbageFormatTxOutEncoding {
    let ConwayFormatTxOutEncoding {
        len_encoding,
        orig_deser_order,
        address_key_encoding,
        amount_key_encoding,
        datum_option_key_encoding,
        script_reference_tag_encoding,
        script_reference_bytes_encoding,
        script_reference_key_encoding,
    } = enc;

    BabbageFormatTxOutEncoding {
        len_encoding,
        orig_deser_order,
        address_key_encoding,
        amount_key_encoding,
        datum_option_key_encoding,
        script_reference_tag_encoding,
        script_reference_bytes_encoding,
        script_reference_key_encoding,
    }
}

fn to_babbage_transaction_body(body: TransactionBody) -> BabbageTransactionBody {
    let outputs = body
        .outputs
        .into_iter()
        .map(to_babbage_transaction_output)
        .collect();
    let certs = body.certs.map(|c| c.into_iter().map(to_allegra_cert).collect());
    BabbageTransactionBody {
        inputs: body.inputs,
        outputs,
        fee: body.fee,
        ttl: body.ttl,
        certs,
        withdrawals: body.withdrawals,
        update: None,
        auxiliary_data_hash: body.auxiliary_data_hash,
        validity_interval_start: body.validity_interval_start,
        mint: body.mint,
        script_data_hash: body.script_data_hash,
        collateral_inputs: body.collateral_inputs,
        required_signers: body.required_signers,
        network_id: body.network_id,
        collateral_return: body.collateral_return.map(to_babbage_transaction_output),
        total_collateral: body.total_collateral,
        reference_inputs: body.reference_inputs,
        encodings: body.encodings.map(to_babbage_transaction_body_encoding),
    }
}

fn to_allegra_cert(cert: Certificate) -> AllegraCertificate {
    match cert {
        Certificate::StakeRegistration(s) => AllegraCertificate::StakeRegistration(s),
        Certificate::StakeDeregistration(s) => AllegraCertificate::StakeDeregistration(s),
        Certificate::StakeDelegation(s) => AllegraCertificate::StakeDelegation(s),
        Certificate::PoolRegistration(p) => AllegraCertificate::PoolRegistration(p),
        Certificate::PoolRetirement(p) => AllegraCertificate::PoolRetirement(p),
        _ => panic!("Variants not supported"),
    }
}

fn to_babbage_transaction_body_encoding(enc: TransactionBodyEncoding) -> BabbageTransactionBodyEncoding {
    BabbageTransactionBodyEncoding {
        len_encoding: enc.len_encoding,
        orig_deser_order: enc.orig_deser_order,
        inputs_encoding: enc.inputs_encoding,
        inputs_key_encoding: enc.inputs_key_encoding,
        outputs_encoding: enc.outputs_encoding,
        outputs_key_encoding: enc.outputs_key_encoding,
        fee_encoding: enc.fee_encoding,
        fee_key_encoding: enc.fee_key_encoding,
        ttl_encoding: enc.ttl_encoding,
        ttl_key_encoding: enc.ttl_key_encoding,
        certs_encoding: enc.certs_encoding,
        certs_key_encoding: enc.certs_key_encoding,
        withdrawals_encoding: enc.withdrawals_encoding,
        withdrawals_value_encodings: enc.withdrawals_value_encodings,
        withdrawals_key_encoding: enc.withdrawals_key_encoding,
        update_key_encoding: None,
        auxiliary_data_hash_encoding: enc.auxiliary_data_hash_encoding,
        auxiliary_data_hash_key_encoding: enc.auxiliary_data_hash_key_encoding,
        validity_interval_start_encoding: enc.validity_interval_start_encoding,
        validity_interval_start_key_encoding: enc.validity_interval_start_key_encoding,
        mint_encoding: enc.mint_encoding,
        mint_key_encodings: enc.mint_key_encodings,
        mint_value_encodings: enc.mint_value_encodings,
        mint_key_encoding: enc.mint_key_encoding,
        script_data_hash_encoding: enc.script_data_hash_encoding,
        script_data_hash_key_encoding: enc.script_data_hash_key_encoding,
        collateral_inputs_encoding: enc.collateral_inputs_encoding,
        collateral_inputs_key_encoding: enc.collateral_inputs_key_encoding,
        required_signers_encoding: enc.required_signers_encoding,
        required_signers_elem_encodings: enc.required_signers_elem_encodings,
        required_signers_key_encoding: enc.required_signers_key_encoding,
        network_id_key_encoding: enc.network_id_key_encoding,
        collateral_return_key_encoding: enc.collateral_return_key_encoding,
        total_collateral_encoding: enc.total_collateral_encoding,
        total_collateral_key_encoding: enc.total_collateral_key_encoding,
        reference_inputs_encoding: enc.reference_inputs_encoding,
        reference_inputs_key_encoding: enc.reference_inputs_key_encoding,
    }
}

fn to_babbage_auxiliary_data(d: AuxiliaryData) -> BabbageAuxiliaryData {
    match d {
        AuxiliaryData::Shelley(s) => BabbageAuxiliaryData::Shelley(s),
        AuxiliaryData::ShelleyMA(s) => BabbageAuxiliaryData::ShelleyMA(s),
        AuxiliaryData::Conway(ConwayFormatAuxData {
            metadata,
            native_scripts,
            plutus_v1_scripts,
            plutus_v2_scripts,
            plutus_v3_scripts: _,
            encodings,
        }) => {
            let encodings = encodings.map(|c| BabbageFormatAuxDataEncoding {
                len_encoding: c.len_encoding,
                tag_encoding: c.tag_encoding,
                orig_deser_order: c.orig_deser_order,
                metadata_key_encoding: c.metadata_key_encoding,
                native_scripts_encoding: c.native_scripts_encoding,
                native_scripts_key_encoding: c.native_scripts_key_encoding,
                plutus_v1_scripts_encoding: c.plutus_v1_scripts_encoding,
                plutus_v1_scripts_key_encoding: c.plutus_v1_scripts_key_encoding,
                plutus_v2_scripts_encoding: c.plutus_v2_scripts_encoding,
                plutus_v2_scripts_key_encoding: c.plutus_v2_scripts_key_encoding,
            });

            BabbageAuxiliaryData::Babbage(BabbageFormatAuxData {
                metadata,
                native_scripts,
                plutus_v1_scripts,
                plutus_v2_scripts,
                encodings,
            })
        }
    }
}

fn to_babbage_transaction_encoding(enc: TransactionEncoding) -> BabbageTransactionEncoding {
    BabbageTransactionEncoding {
        len_encoding: enc.len_encoding,
    }
}
