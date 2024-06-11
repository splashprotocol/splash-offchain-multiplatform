#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeConfig<'a> {
    pub path: &'a str,
    pub magic: u64,
}
