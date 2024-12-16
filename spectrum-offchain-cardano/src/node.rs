#[derive(Clone, serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NodeConfig {
    pub path: String,
    pub magic: u64,
}
