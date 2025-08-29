use crate::message::ExecutionReport;
use log::info;
use std::net::SocketAddr;
use tokio_postgres::Client;

pub(crate) async fn write_report(
    client: &Client,
    reporter: SocketAddr,
    report: ExecutionReport,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let num_executions = report.executions.len();
    for exec in report.executions {
        let (price_num, price_den) = exec.avg_price.unwrap().reduced().into_raw();
        client
            .execute(
                INSERT_REPORT_ST,
                &[
                    &exec.id.to_string(),
                    &exec.version.to_string(),
                    &report.pair.to_string(),
                    &(price_num as i64),
                    &(price_den as i64),
                    &(exec.removed_input as i64),
                    &(exec.added_output as i64),
                    &(exec.fee as i64),
                    &exec.side.to_string(),
                    &reporter.to_string(),
                ],
            )
            .await?;
    }
    info!("Inserted {} executions", num_executions);
    Ok(())
}

const INSERT_REPORT_ST: &str = "INSERT INTO reports (id, ver, pair, price_num, price_den, removed_input, added_output, fee, side, reporter, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, current_timestamp)";
