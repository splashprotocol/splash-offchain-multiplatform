#[derive(serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub enum ExplorerConfig {
    MaestroKeyPath(String),
    BlockfrostKeyPath(String),
}
