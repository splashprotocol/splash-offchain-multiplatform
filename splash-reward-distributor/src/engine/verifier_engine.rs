use std::marker::PhantomData;

use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use cml_chain::transaction::Transaction;
use futures::Stream;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::{
    deployment::ProtocolValidator,
    protocol_config::{BufferWalletScript, SplashPolicy},
};
use splash_yf_offchain::{events::OnChainEvent, settings::MinLovelacePerHarvest};
use tokio::sync::oneshot;
use tokio_stream::StreamExt;

use crate::engine::{
    resolved_tx::PartiallySignedCardanoTx,
    verifier::{AuthorizedExecutors, LocalVerifier, TxCosignRequest, VerifierHandleLedgerEvent},
};

pub struct VerifierEngine<U, R, Verifier, Ctx> {
    ledger_event_stream: U,
    cosign_request_stream: R,
    local_verifier: Verifier,
    pd: PhantomData<Ctx>,
}

impl<U, R, Verifier, Ctx> VerifierEngine<U, R, Verifier, Ctx> {
    pub fn new(ledger_event_stream: U, cosign_request_stream: R, local_verifier: Verifier) -> Self {
        Self {
            ledger_event_stream,
            cosign_request_stream,
            local_verifier,
            pd: PhantomData,
        }
    }
}

impl<GaugeId, StateId, Bearer, U, R, Verifier, Ctx> VerifierEngine<U, R, Verifier, Ctx>
where
    GaugeId: Copy + Unpin + Send + 'static,
    StateId: Copy + Unpin + Send + 'static,
    Bearer: Unpin + Send + 'static,
    U: Stream<
            Item = (
                BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
                TransactionHandle,
            ),
        > + Unpin,
    R: Stream<Item = (TxCosignRequest, oneshot::Sender<Option<Transaction>>)> + Unpin,
    Verifier:
        VerifierHandleLedgerEvent + LocalVerifier<TxCosignRequest, Transaction, Ctx> + Unpin + Send + 'static,
    Ctx: Has<MinLovelacePerHarvest>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<NetworkId>
        + Has<SplashPolicy>
        + Has<AuthorizedExecutors>
        + Has<BufferWalletScript>
        + Sync,
{
    pub async fn run(mut self, ctx: Ctx) {
        loop {
            tokio::select! {
                Some(event) = self.ledger_event_stream.next() => {
                    process_ledger_event(event, &mut self.local_verifier).await;
                }
                Some((tx, sender)) = self.cosign_request_stream.next() => {
                    sender.send(self.local_verifier.try_approve(&tx, &ctx).await).unwrap();
                }
            }
        }
    }
}

async fn process_ledger_event<GaugeId, StateId, Bearer, Verifier>(
    event: (
        BlockEvents<OnChainEvent<GaugeId, StateId, Bearer>>,
        TransactionHandle,
    ),
    verifier: &mut Verifier,
) where
    Verifier: VerifierHandleLedgerEvent,
{
    let (events, tx) = event;
    match events {
        BlockEvents::RollForward {
            events, block_slot, ..
        } => {
            verifier.confirm_block_slot(block_slot);
            for event in events {
                match event {
                    OnChainEvent::BotHarvestingAction { payouts, tx_hash, .. } => {
                        let confirmed_user_harvests = payouts
                            .iter()
                            .map(|(harvest_order, _)| harvest_order.account_key)
                            .collect::<Vec<_>>();
                        verifier.confirm_harvest_tx(tx_hash, &confirmed_user_harvests);
                    }
                    OnChainEvent::BotGaugeBufferingAction { tx_hash, .. } => {
                        verifier.confirm_gauge_buffering_tx(tx_hash);
                    }
                    _ => (),
                }
            }
        }
        BlockEvents::RollBackward {
            events, block_slot, ..
        } => {
            verifier.rollback_block_slot(block_slot);
            for event in events {
                match event {
                    OnChainEvent::BotHarvestingAction { payouts, tx_hash, .. } => {
                        let user_harvests_to_remove = payouts
                            .iter()
                            .map(|(harvest_order, _)| harvest_order.account_key)
                            .collect::<Vec<_>>();
                        verifier.rollback_harvest_tx(tx_hash, user_harvests_to_remove);
                    }
                    OnChainEvent::BotGaugeBufferingAction { tx_hash, .. } => {
                        verifier.rollback_gauge_buffering_tx(tx_hash);
                    }
                    _ => (),
                }
            }
        }
    }
    tx.commit();
}
