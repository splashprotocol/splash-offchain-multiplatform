//! Handle funding boxes for the bot. Note that we're using elements from `splash_dao_offchain` such
//! as `FundingBox`. The reason is we want to use the `FundingRepo` trait for persistence, which is
//! already in use for the DAO-bot.
use spectrum_cardano_lib::{
    tx_view::{TimedOutput, TxViewPartiallyResolved},
    OutputRef,
};
use spectrum_offchain::{domain::Has, ledger::TryFromLedger};
use splash_dao_offchain::{
    entities::onchain::funding_box::{FundingBox, FundingBoxId, FundingBoxSnapshot},
    protocol_config::OperatorCreds,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmedFundingBoxChanges {
    pub consumed: Vec<FundingBoxId>,
    pub created: Vec<FundingBox>,
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for ConfirmedFundingBoxChanges
where
    Cx: Has<OperatorCreds>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let mut consumed = vec![];
        // Scan for consumed and created funding boxes
        for (input, output) in &repr.inputs {
            if let Some(TimedOutput { output, .. }) = output {
                let output_ref = OutputRef::from(input.clone());
                let ctx = FundingBoxCtx {
                    operator_creds: ctx.select::<OperatorCreds>(),
                    output_ref,
                };
                if FundingBoxSnapshot::try_from_ledger(output, &ctx).is_some() {
                    consumed.push(FundingBoxId::from(output_ref));
                }
            }
        }

        let mut created = vec![];
        for (ix, tx_output) in repr.outputs.iter().enumerate() {
            let output_ref = OutputRef::new(repr.hash, ix as u64);
            let ctx = FundingBoxCtx {
                operator_creds: ctx.select::<OperatorCreds>(),
                output_ref,
            };
            if let Some(snapshot) = FundingBoxSnapshot::try_from_ledger(tx_output, &ctx) {
                created.push(snapshot.get().clone());
            }
        }
        Some(Self { consumed, created })
    }
}

/// Need this struct so we can use `FundingBoxSnapshot::try_from_ledger(...)` above.
struct FundingBoxCtx {
    operator_creds: OperatorCreds,
    output_ref: OutputRef,
}

impl Has<OperatorCreds> for FundingBoxCtx {
    fn select<U: type_equalities::IsEqual<OperatorCreds>>(&self) -> OperatorCreds {
        self.operator_creds.clone()
    }
}

impl Has<OutputRef> for FundingBoxCtx {
    fn select<U: type_equalities::IsEqual<OutputRef>>(&self) -> OutputRef {
        self.output_ref
    }
}
