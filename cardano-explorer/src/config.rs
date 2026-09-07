#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplorerConfig {
    BlockfrostKeyPath(String),
}
