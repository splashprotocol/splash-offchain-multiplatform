use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub pg: Pg,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pg {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub db_name: String,
}
