use cml_chain::address::Address;
use cml_chain::certs::{Credential, StakeCredential};
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, ScriptRef, TransactionOutput};
use cml_chain::Value;
use cml_crypto::ScriptHash;
use cml_multi_era::babbage::{BabbageFormatTxOut, BabbageScriptRef, BabbageTransactionOutput};

use crate::address::AddressExtension;
use crate::AssetClass;

pub trait TransactionOutputExtension {
    fn address(&self) -> &Address;
    fn value(&self) -> &Value;
    fn value_mut(&mut self) -> &mut Value;
    fn datum(&self) -> Option<DatumOption>;
    fn data_mut(&mut self) -> Option<&mut PlutusData>;
    fn null_datum(&mut self);
    fn into_datum(self) -> Option<DatumOption>;
    fn script_hash(&self) -> Option<ScriptHash>;
    fn update_payment_cred(&mut self, cred: StakeCredential);
    fn update_address(&mut self, addr: Address);
    fn update_value(&mut self, value: Value);
    fn script_ref(&self) -> Option<&BabbageScriptRef>;
    fn sub_asset(&mut self, asset: AssetClass, amount: u64) {
        let updated_value = self.value().checked_sub(&asset.into_value(amount)).unwrap();
        *self.value_mut() = updated_value;
    }
    fn add_asset(&mut self, asset: AssetClass, amount: u64) {
        let updated_value = self.value().checked_add(&asset.into_value(amount)).unwrap();
        *self.value_mut() = updated_value;
    }
}

impl TransactionOutputExtension for BabbageTransactionOutput {
    fn address(&self) -> &Address {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => &tx_out.address,
            Self::BabbageFormatTxOut(tx_out) => &tx_out.address,
        }
    }
    fn value(&self) -> &Value {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => &tx_out.amount,
            Self::BabbageFormatTxOut(tx_out) => &tx_out.amount,
        }
    }
    fn value_mut(&mut self) -> &mut Value {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => &mut tx_out.amount,
            Self::BabbageFormatTxOut(tx_out) => &mut tx_out.amount,
        }
    }
    fn datum(&self) -> Option<DatumOption> {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => tx_out.datum_hash.map(DatumOption::new_hash).clone(),
            Self::BabbageFormatTxOut(tx_out) => tx_out.datum_option.clone(),
        }
    }
    fn data_mut(&mut self) -> Option<&mut PlutusData> {
        match self {
            Self::BabbageFormatTxOut(BabbageFormatTxOut {
                datum_option: Some(DatumOption::Datum { datum, .. }),
                ..
            }) => Some(datum),
            _ => None,
        }
    }
    fn null_datum(&mut self) {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => {
                tx_out.datum_hash.take();
            }
            Self::BabbageFormatTxOut(tx_out) => {
                tx_out.datum_option.take();
            }
        }
    }
    fn into_datum(self) -> Option<DatumOption> {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => tx_out.datum_hash.map(DatumOption::new_hash),
            Self::BabbageFormatTxOut(tx_out) => tx_out.datum_option,
        }
    }
    fn script_hash(&self) -> Option<ScriptHash> {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => tx_out.address.script_hash(),
            Self::BabbageFormatTxOut(tx_out) => tx_out.address.script_hash(),
        }
    }
    fn update_payment_cred(&mut self, cred: StakeCredential) {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => tx_out.address.update_payment_cred(cred),
            Self::BabbageFormatTxOut(tx_out) => tx_out.address.update_payment_cred(cred),
        }
    }
    fn update_address(&mut self, addr: Address) {
        match self {
            BabbageTransactionOutput::AlonzoFormatTxOut(tx_out) => tx_out.address = addr,
            BabbageTransactionOutput::BabbageFormatTxOut(tx_out) => tx_out.address = addr,
        }
    }
    fn update_value(&mut self, value: Value) {
        match self {
            Self::AlonzoFormatTxOut(ref mut out) => {
                out.amount = value;
            }
            Self::BabbageFormatTxOut(ref mut out) => {
                out.amount = value;
            }
        }
    }
    fn script_ref(&self) -> Option<&BabbageScriptRef> {
        match self {
            Self::AlonzoFormatTxOut(_) => None,
            Self::BabbageFormatTxOut(tx_out) => tx_out.script_reference.as_ref(),
        }
    }
}

impl TransactionOutputExtension for TransactionOutput {
    fn address(&self) -> &Address {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => &tx_out.address,
            Self::ConwayFormatTxOut(tx_out) => &tx_out.address,
        }
    }
    fn value(&self) -> &Value {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => &tx_out.amount,
            Self::ConwayFormatTxOut(tx_out) => &tx_out.amount,
        }
    }
    fn value_mut(&mut self) -> &mut Value {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => &mut tx_out.amount,
            Self::ConwayFormatTxOut(tx_out) => &mut tx_out.amount,
        }
    }
    fn datum(&self) -> Option<DatumOption> {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => tx_out.datum_hash.map(DatumOption::new_hash).clone(),
            Self::ConwayFormatTxOut(tx_out) => tx_out.datum_option.clone(),
        }
    }
    fn data_mut(&mut self) -> Option<&mut PlutusData> {
        match self {
            Self::ConwayFormatTxOut(ConwayFormatTxOut {
                datum_option: Some(DatumOption::Datum { datum, .. }),
                ..
            }) => Some(datum),
            _ => None,
        }
    }
    fn null_datum(&mut self) {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => {
                tx_out.datum_hash.take();
            }
            Self::ConwayFormatTxOut(tx_out) => {
                tx_out.datum_option.take();
            }
        }
    }
    fn into_datum(self) -> Option<DatumOption> {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => tx_out.datum_hash.map(DatumOption::new_hash),
            Self::ConwayFormatTxOut(tx_out) => tx_out.datum_option,
        }
    }
    fn script_hash(&self) -> Option<ScriptHash> {
        match self.address().payment_cred()? {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(*hash),
        }
    }
    fn update_payment_cred(&mut self, cred: Credential) {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => tx_out.address.update_payment_cred(cred),
            Self::ConwayFormatTxOut(tx_out) => tx_out.address.update_payment_cred(cred),
        }
    }
    fn update_address(&mut self, addr: Address) {
        match self {
            Self::AlonzoFormatTxOut(tx_out) => tx_out.address = addr,
            Self::ConwayFormatTxOut(tx_out) => tx_out.address = addr,
        }
    }
    fn update_value(&mut self, value: Value) {
        match self {
            TransactionOutput::AlonzoFormatTxOut(ref mut out) => {
                out.amount = value;
            }
            TransactionOutput::ConwayFormatTxOut(ref mut out) => {
                out.amount = value;
            }
        }
    }
    fn script_ref(&self) -> Option<&BabbageScriptRef> {
        None
    }
}

pub trait BabbageScriptRefExtension {
    fn upcast(self) -> ScriptRef;
}

impl BabbageScriptRefExtension for BabbageScriptRef {
    fn upcast(self) -> ScriptRef {
        match self {
            BabbageScriptRef::Native {
                script,
                len_encoding,
                tag_encoding,
            } => ScriptRef::Native {
                script,
                len_encoding,
                tag_encoding,
            },
            BabbageScriptRef::PlutusV1 {
                script,
                len_encoding,
                tag_encoding,
            } => ScriptRef::PlutusV1 {
                script,
                len_encoding,
                tag_encoding,
            },
            BabbageScriptRef::PlutusV2 {
                script,
                len_encoding,
                tag_encoding,
            } => ScriptRef::PlutusV2 {
                script,
                len_encoding,
                tag_encoding,
            },
        }
    }
}

pub trait BabbageTransactionOutputExtension {
    fn upcast(self) -> TransactionOutput;
}

impl BabbageTransactionOutputExtension for BabbageTransactionOutput {
    fn upcast(self) -> TransactionOutput {
        match self {
            BabbageTransactionOutput::AlonzoFormatTxOut(alonzo_out) => {
                TransactionOutput::AlonzoFormatTxOut(alonzo_out)
            }
            BabbageTransactionOutput::BabbageFormatTxOut(babbage_out) => {
                TransactionOutput::ConwayFormatTxOut(ConwayFormatTxOut {
                    address: babbage_out.address,
                    amount: babbage_out.amount,
                    datum_option: babbage_out.datum_option,
                    script_reference: babbage_out.script_reference.map(|script_ref| script_ref.upcast()),
                    encodings: None,
                })
            }
        }
    }
}
