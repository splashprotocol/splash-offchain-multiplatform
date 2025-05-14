use crate::http_clients::lp_indexer_client::{LpIndexerAccount, LpIndexerClient};
use async_trait::async_trait;
use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::crypto::Vkeywitness;
use cml_chain::governance::Vote::No;
use cml_chain::transaction::Transaction;
use cml_core::serialization::{FromBytes, Serialize as CMLSerialize};
use futures::FutureExt;
use reqwest::RequestBuilder;
use serde::Serialize;

pub trait ValidatorClient {
    async fn validate_tx(&self, signed_tx: SignedTxBuilder) -> Option<Transaction>;
}

#[derive(Clone)]
pub struct HttpClient {
    http_client: reqwest::Client,
    base_url: String,
}

impl HttpClient {
    pub fn new() -> Self {
        let client = reqwest::Client::new();
        Self {
            http_client: client,
            base_url: "http://127.0.0.1:8080".to_string(),
        }
    }

    pub async fn get(&self, url: &str) -> Result<String, Box<dyn std::error::Error>> {
        let req = self.http_client.get(format!("{}{}", &self.base_url, url));
        Ok(req.send().await?.text().await?)
    }

    pub async fn post<T: Serialize>(&self, url: &str, body: T) -> Result<String, Box<dyn std::error::Error>> {
        let json_body = serde_json::to_string(&body)?;

        let req: RequestBuilder = self
            .http_client
            .post(format!("{}{}", &self.base_url, url))
            .header("Content-Type", "application/json")
            .body(json_body);

        let mut resp = req.send().await?;
        let response = resp.text().await?;

        Ok(response)
    }
}

impl ValidatorClient for HttpClient {
    async fn validate_tx(&self, signed_tx: SignedTxBuilder) -> Option<Transaction> {
        let tx_body_raw = hex::encode(signed_tx.body().clone().to_cbor_bytes());
        if let (Ok(res)) = self.post("/validate", tx_body_raw).await {
            let tx_signature = hex::decode(res).unwrap();
            let vkey_witness = Vkeywitness::from_bytes(tx_signature).unwrap();
            let mut signature_to_return = signed_tx.clone();
            signature_to_return.clone().add_vkey(vkey_witness);
            Some(signature_to_return.build_unchecked())
        } else {
            None
        }
    }
}

#[async_trait]
impl LpIndexerClient for HttpClient {
    async fn lock_user(&self, account: String) -> Option<LpIndexerAccount> {
        if let Ok(result) = self.get(format!("/accounts/{}/lock", account).as_str()).await {
            let result = serde_json::from_str(result.as_str()).unwrap();
            return Some(result);
        } else {
            None
        }
    }

    async fn get_user_info(&self, account: String) -> Result<Option<LpIndexerAccount>, String> {
        todo!()
    }
}
