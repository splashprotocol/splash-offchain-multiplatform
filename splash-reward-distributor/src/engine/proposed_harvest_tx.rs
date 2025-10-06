use crate::engine::{
    resolved_tx::{CardanoTxInput, PartiallySignedCardanoTx},
    verifier::AuthorizedExecutors,
};
use cml_crypto::TransactionHash;
use spectrum_cardano_lib::{
    transaction::TransactionOutputExtension, value::ValueExtension, AssetClass, AssetName, NetworkId,
    OutputRef, Token,
};
use spectrum_offchain::{domain::Has, ledger::TryFromLedger};
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::{
    constants::SPLASH_NAME,
    deployment::ProtocolValidator,
    protocol_config::{BufferWalletScript, SplashPolicy},
};
use splash_yf_offchain::{
    entities::{
        buffer_wallet::try_extract_buffer_wallet,
        harvest_order::{try_extract_harvest_order, HarvestOrder},
    },
    settings::MinLovelacePerHarvest,
};

pub struct ProposedHarvestTx<OrderId> {
    pub withdrawals: Vec<Withdrawal<OrderId>>,
    pub buffer_wallet_tx_hash: TransactionHash,
}

#[derive(Debug)]
pub struct Withdrawal<OrderId> {
    // Order that requested harvesting
    pub order: HarvestOrder<OrderId>,
    // Amount withdrawn from buffer wallet
    pub amount: u64,
}

impl<Ctx> TryFromLedger<PartiallySignedCardanoTx, Ctx> for ProposedHarvestTx<OutputRef>
where
    Ctx: Has<MinLovelacePerHarvest>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<NetworkId>
        + Has<AuthorizedExecutors>
        + Has<BufferWalletScript>
        + Has<SplashPolicy>,
{
    fn try_from_ledger(repr: &PartiallySignedCardanoTx, ctx: &Ctx) -> Option<Self> {
        let splash_asset_name = AssetName::from_utf8(SPLASH_NAME.into());
        let splash_policy = ctx.select::<SplashPolicy>().0;
        let splash_asset_class = AssetClass::Token(Token(splash_policy, splash_asset_name));
        let network_id = ctx.select::<NetworkId>();

        let mut buffer_wallet = None;

        let withdrawals: Vec<_> = repr
            .inputs
            .iter()
            .filter_map(
                |CardanoTxInput {
                     output_ref,
                     tx_output,
                     issued_at,
                 }| {
                    let res = issued_at.and_then(|issued_at| {
                        try_extract_harvest_order(tx_output, *output_ref, issued_at, ctx).and_then(|order| {
                            repr.tx.body.outputs.iter().find_map(|tx_output| {
                                if *tx_output.address() == order.reward_receiver.to_address(network_id) {
                                    let amount = tx_output.value().amount_of(splash_asset_class)?;
                                    return Some(Withdrawal {
                                        order: order.clone(),
                                        amount,
                                    });
                                }
                                None
                            })
                        })
                    });

                    if res.is_none() && buffer_wallet.is_none() {
                        buffer_wallet = try_extract_buffer_wallet(tx_output, *output_ref, ctx);
                    }

                    res
                },
            )
            .collect();

        let valid_tx_signature = repr
            .tx
            .body
            .required_signers
            .as_ref()
            .map(|signers| {
                if signers.len() == 1 {
                    let signer_key_hash = signers.first().unwrap();
                    let authorized_signers = ctx.select::<AuthorizedExecutors>().0;
                    authorized_signers.contains(signer_key_hash)
                } else {
                    false
                }
            })
            .unwrap_or(false);

        if let Some(buffer_wallet) = buffer_wallet {
            if !withdrawals.is_empty() && valid_tx_signature {
                Some(ProposedHarvestTx {
                    withdrawals,
                    buffer_wallet_tx_hash: buffer_wallet.state_id.tx_hash(),
                })
            } else {
                None
            }
        } else {
            None
        }
    }
}
