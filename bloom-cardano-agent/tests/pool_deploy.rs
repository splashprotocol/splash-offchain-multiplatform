use bloom_offchain::execution_engine::liquidity_book::ExecutionCap;
use derive_more::Display;
use serde::de::DeserializeOwned;
use serde::Serialize;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_offchain_cardano::creds::operator_creds;

const EXECUTION_CAP: ExecutionCap = ExecutionCap {
    soft: 6000000000,
    hard: 10000000000,
};

#[tokio::test]
// #[ignore]
async fn deploy_pool() {
    let deployer_key = "";

    let (operator_sk, _, _) = operator_creds(deployer_key, 1_u64);

    let mut tx_builder = constant_tx_builder();

    get_token_info(
        "d2e7c4c0bde5fd8e4bb964e36c6ba3296adff6ba8d73833f43c4477466ca8c27".to_string(),
        2,
        "4f534f43494554595f4144415f4e4654".to_string(),
        1,
    )
    .await;
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MintRequest {
    tx_ref: String,
    out_id: String,
    tn_name: String,
    qty: String,
}

#[derive(serde::Deserialize, Display)]
#[serde(rename_all = "camelCase")]
pub struct MinPolicyCreationResponse {
    result: String,
}

#[derive(serde::Deserialize, Display)]
#[serde(rename_all = "camelCase")]
pub struct MinPolicyInfoResponse {
    policy_id: String,
    script: String,
}

async fn get_token_info(out_id: String, out_idx: u8, name: String, qty: u128) -> (String, String) {
    let creation_url = "http://88.99.59.114:8081/getData/".to_string();
    let info_url = " http://88.99.59.114:3490/getData/".to_string();

    let request = MintRequest {
        tx_ref: out_id,
        out_id: out_idx.to_string(),
        tn_name: name,
        qty: qty.to_string(),
    };

    let creation_response: Option<MinPolicyCreationResponse> =
        post_request::<MintRequest, MinPolicyCreationResponse>(creation_url, request).await;

    println!("creation_response: {}", creation_response);
}

async fn post_request<B: Serialize, T: DeserializeOwned>(url: String, body: B) -> Option<T> {
    let client = reqwest::Client::new();

    client
        .post(url)
        .body("the exact body that is sent")
        .json(body)
        .send()
        .await
        .ok()
        .unwrap()
        .json::<T>()
        .await
        .ok()
}
