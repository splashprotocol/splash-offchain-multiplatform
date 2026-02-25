use crate::index::{CredentialKind, Txo, TxoQuery, UtxoResolver};
use actix_cors::Cors;
use actix_web::dev::{AppService, HttpServiceFactory, Server};
use actix_web::web::Data;
use actix_web::{guard, web, App, HttpResponse, HttpServer, Responder};
use async_primitives::beacon::Beacon;
use cml_chain::address::Address;
use cml_chain::certs::Credential;
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding, TransactionHash};
use log::trace;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use std::fmt::Display;
use std::io;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, serde::Deserialize, serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GetTxOsRequest {
    pkh: Ed25519KeyHash,
    query: TxoQuery,
    offset: usize,
    limit: usize,
}

impl Display for GetTxOsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "GetTxOsRequest {{ pkh: {}, query: {}, offset: {}, limit: {} }}",
            self.pkh, self.query, self.offset, self.limit
        )
    }
}

#[derive(Clone, serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UTxO {
    pub transaction_hash: TransactionHash,
    pub index: usize,
    pub address: Address,
    pub value: Vec<Asset>,
    pub settled_at: Option<u64>,
    pub spent: bool,
}

impl From<Txo> for UTxO {
    fn from(txo: Txo) -> Self {
        Self {
            transaction_hash: txo.oref.tx_hash(),
            index: txo.oref.index() as usize,
            address: txo.output.address().clone(),
            value: vec![Asset {
                policy_id: "".to_string(),
                base16_name: "".to_string(),
                amount: txo.output.value().coin.to_string(),
            }]
            .into_iter()
            .chain(txo.output.value().multiasset.iter().flat_map(|(pol, assets)| {
                assets.iter().map(|(name, amt)| Asset {
                    policy_id: pol.to_string(),
                    base16_name: name.to_raw_hex(),
                    amount: amt.to_string(),
                })
            }))
            .collect(),
            settled_at: txo.settled_at,
            spent: txo.spent,
        }
    }
}

#[derive(Clone, serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub policy_id: String,
    pub base16_name: String,
    pub amount: String,
}

pub struct Service<R>(PhantomData<R>);

async fn get_utxos<R>(
    req: web::Json<GetTxOsRequest>,
    db: Data<R>,
    startup_complete: Data<Arc<AtomicBool>>,
) -> impl Responder
where
    R: UtxoResolver + 'static,
{
    if !startup_complete.load(Ordering::Acquire) {
        return HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "error": "Service is starting up",
            "message": "Service is initializing - please retry in a moment"
        }));
    }
    trace!("Received request: {:?}", req);
    let scope = (Credential::new_pub_key(req.pkh), CredentialKind::Payment);
    let utxos = db.get_utxos(Some(scope), req.query, req.offset, req.limit).await;
    let result = utxos.into_iter().map(UTxO::from).collect::<Vec<_>>();
    trace!("Responding with: {} TXOs", result.len());
    HttpResponse::Ok().json(result)
}

fn get_utxos_service<R: UtxoResolver + 'static>() -> actix_web::Resource {
    web::resource("/getUtxos").route(
        web::route()
            .guard(guard::Post())
            .guard(guard::Header("content-type", "application/json"))
            .to(get_utxos::<R>),
    )
}

async fn healthcheck(state_synced: Data<Beacon>, startup_complete: Data<Arc<AtomicBool>>) -> impl Responder {
    if !startup_complete.load(Ordering::Acquire) {
        return HttpResponse::ServiceUnavailable().finish();
    }
    if state_synced.read() {
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::ServiceUnavailable().finish()
    }
}

fn healthcheck_service() -> actix_web::Resource {
    web::resource("/health").route(web::route().guard(guard::Get()).to(healthcheck))
}

pub async fn build_api_server<R>(
    db: R,
    state_synced: Beacon,
    startup_complete: Arc<AtomicBool>,
    bind_addr: SocketAddr,
) -> Result<Server, io::Error>
where
    R: UtxoResolver + Send + Clone + 'static,
{
    Ok(HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();

        App::new()
            .wrap(cors)
            .app_data(Data::new(db.clone()))
            .app_data(Data::new(state_synced.clone()))
            .app_data(Data::new(startup_complete.clone()))
            .service(healthcheck_service())
            .service(get_utxos_service::<R>())
    })
    .bind(bind_addr)?
    .workers(8)
    .disable_signals()
    .run())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn request_samples() {
        let sample_request_all = GetTxOsRequest {
            pkh: Ed25519KeyHash::from([0u8; 28]),
            query: TxoQuery::All(Some(1)),
            offset: 0,
            limit: 10,
        };

        let json = serde_json::to_string_pretty(&sample_request_all).unwrap();
        assert!(!json.is_empty());
        println!("{}", json);

        let sample_request_unspent = GetTxOsRequest {
            pkh: Ed25519KeyHash::from([0u8; 28]),
            query: TxoQuery::Unspent,
            offset: 0,
            limit: 10,
        };

        let json = serde_json::to_string_pretty(&sample_request_unspent).unwrap();
        assert!(!json.is_empty());
        println!("{}", json);
    }
}
