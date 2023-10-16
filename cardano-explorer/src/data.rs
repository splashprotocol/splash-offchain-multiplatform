use serde::Deserialize;

pub mod full_tx_out;

pub mod value;

#[derive(Deserialize, Copy, Clone)]
pub struct ExplorerConfig<'a> {
    pub url: &'a str,
}