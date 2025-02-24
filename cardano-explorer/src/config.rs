#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplorerConfig {
    MaestroKeyPath(String),
    BlockfrostKeyPath(String),
}
