use crate::message::ExecutionReport;
use std::net::SocketAddr;
use std::str::FromStr;
use bigdecimal::ToPrimitive;
use log::info;
use tokio_postgres::Client;
use pg_bigdecimal::BigDecimal as PgBigDecimal;
use pg_bigdecimal::PgNumeric;

pub(crate) async fn write_report(
    client: &Client,
    reporter: SocketAddr,
    report: ExecutionReport,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let num_executions = report.executions.len();
    for exec in report.executions {
        let (price_num, price_den) = exec.mean_price.unwrap().reduced().into_raw();
        let pg_big_decimal = report.meta.clone().mean_spot_price.map(|x| {
            PgBigDecimal::from(x.as_bigint_and_exponent())
        });
        let pg_numeric = PgNumeric::new(pg_big_decimal);
        client
            .execute(
                INSERT_ST,
                &[
                    &exec.id.to_string(),
                    &exec.version.to_string(),
                    &report.pair.to_string(),
                    &(price_num as i64),
                    &(price_den as i64),
                    &(exec.removed_input as i64),
                    &(exec.added_output as i64),
                    &exec.side.to_string(),
                    &pg_numeric,
                    &reporter.to_string(),
                ],
            )
            .await?;
    }
    info!("Inserted {} executions", num_executions);
    Ok(())
}

const INSERT_ST: &str = "INSERT INTO executions (id, ver, pair, price_num, price_den, removed_input, added_output, side, meta, reporter, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, current_timestamp)";
