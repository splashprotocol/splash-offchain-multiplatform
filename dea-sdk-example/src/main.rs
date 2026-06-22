use anyhow::{Context, Result};
use dea_sdk::{config::DeaConfig, domain::ReplayInput, replay};
use std::{env, fs};

fn main() -> Result<()> {
    let path = env::args()
        .nth(1)
        .context("usage: dea-sdk-example <fixture-path>")?;
    let raw = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
    let input: ReplayInput =
        serde_json::from_str(&raw).with_context(|| format!("failed to parse {path}"))?;
    let report = replay::assess(&DeaConfig::default(), &input);
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

