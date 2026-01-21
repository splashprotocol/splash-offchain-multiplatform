use std::sync::Arc;

use futures::channel::mpsc::Sender;
use futures::SinkExt;
use log::{debug, error, info, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

use bloom_offchain_cardano::orders::green::{AuthorizedIntention, GreenOrder, Intent};
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::Token;

use crate::account_index::AccountIndex;
use crate::config::IntentReceiverConfig;

/// AuthedIntent structure matching intent-relay encoding.
/// Duplicated here to avoid cross-crate dependency.
#[derive(Debug, Clone)]
pub struct AuthedIntent {
    pub account_id: [u8; 32],
    pub intent: Vec<u8>,
    pub prefix: Vec<u8>,
    pub postfix: Vec<u8>,
    pub signature: Vec<u8>,
    pub credential: [u8; 32],
}

impl AuthedIntent {
    /// Decode from bytes (must match intent-relay encoding).
    pub fn decode(encoded: &[u8]) -> Result<Self, String> {
        let mut cursor = 0;

        // Decode account_id (fixed 32 bytes)
        if cursor + 32 > encoded.len() {
            return Err("Invalid data: incomplete account_id".to_string());
        }
        let mut account_id = [0u8; 32];
        account_id.copy_from_slice(&encoded[cursor..cursor + 32]);
        cursor += 32;

        // Helper to read u32
        let read_u32 = |data: &[u8], cursor: &mut usize| -> Result<u32, String> {
            if *cursor + 4 > data.len() {
                return Err("Invalid data: unexpected end of input".to_string());
            }
            let value = u32::from_be_bytes(data[*cursor..*cursor + 4].try_into().unwrap());
            *cursor += 4;
            Ok(value)
        };

        // Helper to read vec
        let read_vec = |data: &[u8], len: u32, cursor: &mut usize| -> Result<Vec<u8>, String> {
            let len = len as usize;
            if *cursor + len > data.len() {
                return Err("Invalid data: unexpected end of input".to_string());
            }
            let value = data[*cursor..*cursor + len].to_vec();
            *cursor += len;
            Ok(value)
        };

        let intent_len = read_u32(encoded, &mut cursor)?;
        let intent = read_vec(encoded, intent_len, &mut cursor)?;

        let prefix_len = read_u32(encoded, &mut cursor)?;
        let prefix = read_vec(encoded, prefix_len, &mut cursor)?;

        let postfix_len = read_u32(encoded, &mut cursor)?;
        let postfix = read_vec(encoded, postfix_len, &mut cursor)?;

        let signature_len = read_u32(encoded, &mut cursor)?;
        let signature = read_vec(encoded, signature_len, &mut cursor)?;

        if cursor + 32 > encoded.len() {
            return Err("Invalid data: incomplete credential".to_string());
        }
        let mut credential = [0u8; 32];
        credential.copy_from_slice(&encoded[cursor..cursor + 32]);

        Ok(Self {
            account_id,
            intent,
            prefix,
            postfix,
            signature,
            credential,
        })
    }
}

/// TCP Intent Receiver that accepts connections from intent-relay.
pub struct IntentReceiver {
    config: IntentReceiverConfig,
    account_index: Arc<RwLock<AccountIndex>>,
    green_order_sender: Sender<GreenOrder>,
}

impl IntentReceiver {
    pub fn new(
        config: IntentReceiverConfig,
        account_index: Arc<RwLock<AccountIndex>>,
        green_order_sender: Sender<GreenOrder>,
    ) -> Self {
        Self {
            config,
            account_index,
            green_order_sender,
        }
    }

    /// Start the TCP listener and handle incoming connections.
    pub async fn run(self) {
        let listener = match TcpListener::bind(self.config.bind_addr).await {
            Ok(l) => {
                info!("Intent receiver listening on {}", self.config.bind_addr);
                l
            }
            Err(e) => {
                error!("Failed to bind intent receiver to {}: {}", self.config.bind_addr, e);
                return;
            }
        };

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!("Accepted connection from intent-relay at {}", addr);
                    let account_index = Arc::clone(&self.account_index);
                    let sender = self.green_order_sender.clone();
                    let config = self.config.clone();

                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, account_index, sender, config).await {
                            warn!("Connection handler error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
    }
}

/// Handle a single TCP connection from intent-relay.
async fn handle_connection(
    mut stream: TcpStream,
    account_index: Arc<RwLock<AccountIndex>>,
    mut sender: Sender<GreenOrder>,
    config: IntentReceiverConfig,
) -> Result<(), String> {
    loop {
        // Read message length (4 bytes, big-endian)
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                debug!("Connection closed by peer");
                return Ok(());
            }
            Err(e) => {
                return Err(format!("Failed to read message length: {}", e));
            }
        }

        let msg_len = u32::from_be_bytes(len_buf) as usize;
        if msg_len > 1024 * 1024 {
            // Sanity check: max 1MB message
            return Err(format!("Message too large: {} bytes", msg_len));
        }

        // Read message body
        let mut msg_buf = vec![0u8; msg_len];
        stream
            .read_exact(&mut msg_buf)
            .await
            .map_err(|e| format!("Failed to read message body: {}", e))?;

        // Decode AuthedIntent
        let authed_intent = match AuthedIntent::decode(&msg_buf) {
            Ok(ai) => ai,
            Err(e) => {
                warn!("Failed to decode AuthedIntent: {}", e);
                // Send error response (1 byte: 0 = error)
                let _ = stream.write_all(&[0u8]).await;
                continue;
            }
        };

        // Process the intent
        match process_intent(&authed_intent, &account_index, &mut sender, &config).await {
            Ok(()) => {
                debug!(
                    "Successfully processed intent for account {:?}",
                    hex::encode(&authed_intent.account_id[..8])
                );
                // Send success response (1 byte: 1 = success)
                let _ = stream.write_all(&[1u8]).await;
            }
            Err(e) => {
                warn!("Failed to process intent: {}", e);
                // Send error response
                let _ = stream.write_all(&[0u8]).await;
            }
        }
    }
}

/// Process a single AuthedIntent: validate, construct GreenOrder, send to executor.
async fn process_intent(
    authed_intent: &AuthedIntent,
    account_index: &Arc<RwLock<AccountIndex>>,
    sender: &mut Sender<GreenOrder>,
    config: &IntentReceiverConfig,
) -> Result<(), String> {
    // Convert account_id bytes to Token
    // Token is (PolicyId, AssetName) where PolicyId is 28 bytes and AssetName is variable
    // For simplicity, assume first 28 bytes are policy_id, remaining 4 bytes are asset_name
    let account_id = token_from_bytes(&authed_intent.account_id)?;

    // Look up account in index
    let index = account_index.read().await;
    let entry = index
        .get(&account_id)
        .ok_or_else(|| format!("Account not found: {}", account_id))?;

    // Parse intent from bytes
    let intent = parse_intent(&authed_intent.intent)?;

    // Verify signature against account's hot_cred
    // TODO: Implement proper signature verification
    // For now, we trust the signature was verified by intent-relay
    // In production, verify: signature(prefix || intent || postfix) against hot_cred

    // Create authorized intention
    let authorized_intention = AuthorizedIntention::new_signed(
        intent,
        authed_intent.signature.clone(),
        authed_intent.prefix.clone(),
        authed_intent.postfix.clone(),
    );

    // Create MPT snapshot
    let mpt_snapshot = entry.create_snapshot(None);

    // Create GreenOrder
    let green_order = GreenOrder::new(
        account_id,
        authorized_intention,
        entry.utxo.clone(),
        mpt_snapshot,
        config.max_cost_per_ex_step,
        ExUnits { mem: 200_000, steps: 100_000_000 }, // Default marginal cost estimate
        config.min_marginal_output,
    );

    drop(index); // Release lock before sending

    // Send to executor
    sender
        .send(green_order)
        .await
        .map_err(|e| format!("Failed to send GreenOrder to executor: {}", e))?;

    Ok(())
}

/// Convert 32 bytes to Token (PolicyId || AssetName).
fn token_from_bytes(bytes: &[u8; 32]) -> Result<Token, String> {
    use cml_chain::PolicyId;
    use cml_crypto::RawBytesEncoding;
    use spectrum_cardano_lib::AssetName;

    // First 28 bytes: policy_id
    let policy_id = PolicyId::from_raw_bytes(&bytes[0..28])
        .map_err(|e| format!("Invalid policy_id: {:?}", e))?;

    // Remaining 4 bytes: asset_name (padded, trim trailing zeros)
    let asset_name_bytes: Vec<u8> = bytes[28..32]
        .iter()
        .copied()
        .rev()
        .skip_while(|&b| b == 0)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let asset_name = AssetName::try_from(asset_name_bytes)
        .map_err(|e| format!("Invalid asset_name: {:?}", e))?;

    Ok(Token(policy_id, asset_name))
}

/// Parse Intent from CBOR-encoded bytes.
fn parse_intent(bytes: &[u8]) -> Result<Intent, String> {
    use cml_chain::plutus::PlutusData;
    use cml_core::serialization::Deserialize;
    use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
    use spectrum_cardano_lib::plutus_data::PlutusDataExtension;
    use spectrum_cardano_lib::types::TryFromPData;
    use spectrum_cardano_lib::AssetClass;

    // Intent is CBOR-encoded PlutusData
    let pd = PlutusData::from_cbor_bytes(bytes)
        .map_err(|e| format!("Failed to decode intent PlutusData: {:?}", e))?;

    // Parse the constr data using PlutusDataExtension
    let cpd = pd
        .into_constr_pd()
        .ok_or("Intent is not a ConstrPlutusData")?;

    if cpd.alternative != 0 {
        return Err("Invalid intent constructor".to_string());
    }

    let mut fields = cpd.fields.into_iter();

    // Parse target_nonce (index, value)
    let nonce_cpd = fields.next()
        .ok_or("Missing target_nonce")?
        .into_constr_pd()
        .ok_or("target_nonce is not a tuple")?;
    let mut nonce_fields = nonce_cpd.fields.into_iter();
    let nonce_idx = nonce_fields.next()
        .and_then(|pd| pd.into_i128())
        .and_then(|v| i64::try_from(v).ok())
        .ok_or("Invalid nonce index")?;
    let nonce_val = nonce_fields.next()
        .and_then(|pd| pd.into_i128())
        .and_then(|v| i64::try_from(v).ok())
        .ok_or("Invalid nonce value")?;

    // Parse leaving_asset using existing TryFromPData impl
    let leaving_asset = AssetClass::try_from_pd(fields.next().ok_or("Missing leaving_asset")?)
        .ok_or("Invalid leaving_asset")?;

    // Parse leaving_amount
    let leaving_amount = fields.next()
        .and_then(|pd| pd.into_u64())
        .ok_or("Invalid leaving_amount")?;

    // Parse arriving_asset using existing TryFromPData impl
    let arriving_asset = AssetClass::try_from_pd(fields.next().ok_or("Missing arriving_asset")?)
        .ok_or("Invalid arriving_asset")?;

    // Parse expected_arriving_amount
    let expected_arriving_amount = fields.next()
        .and_then(|pd| pd.into_u64())
        .ok_or("Invalid expected_arriving_amount")?;

    // Parse fee_lovelace
    let fee_lovelace = fields.next()
        .and_then(|pd| pd.into_u64())
        .ok_or("Invalid fee_lovelace")?;

    // Parse operator
    let operator_bytes = fields.next()
        .and_then(|pd| pd.into_bytes())
        .ok_or("Invalid operator bytes")?;
    let operator = Ed25519KeyHash::from_raw_bytes(&operator_bytes)
        .map_err(|e| format!("Invalid operator hash: {:?}", e))?;

    Ok(Intent {
        target_nonce: (nonce_idx, nonce_val),
        leaving_asset,
        leaving_amount,
        arriving_asset,
        expected_arriving_amount,
        fee_lovelace,
        operator,
    })
}

/// Create a stream that runs the intent receiver.
pub fn intent_receiver_stream(
    config: IntentReceiverConfig,
    account_index: Arc<RwLock<AccountIndex>>,
    green_order_sender: Sender<GreenOrder>,
) -> impl std::future::Future<Output = ()> {
    let receiver = IntentReceiver::new(config, account_index, green_order_sender);
    receiver.run()
}
