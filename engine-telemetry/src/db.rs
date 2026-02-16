use crate::message::ExecutionReport;
use bloom_offchain::execution_engine::liquidity_book::core::ExecutionEvent;
use log::info;
use std::net::SocketAddr;
use tokio_postgres::Client;

pub(crate) async fn write_report(
    client: &Client,
    reporter: SocketAddr,
    report: ExecutionReport,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Prepare data for the reports table.
    let reporter_str = reporter.to_string();
    let pair_json_str = serde_json::to_string(&report.pair)?;
    let (asset_a, asset_b) = report.pair.assets();
    let asset_a_str = asset_a.to_string();
    let asset_b_str = asset_b.to_string();
    let tx_hash_str: Option<String> = report.tx_hash.map(|h| h.to_hex());

    // Insert the report and get its id.
    let row = client
        .query_one(
            "INSERT INTO reports (reporter, asset_a, asset_b, pair_json, tx_hash) VALUES ($1, $2, $3, $4::jsonb, $5) RETURNING id",
            &[&reporter_str, &asset_a_str, &asset_b_str, &pair_json_str, &tx_hash_str],
        )
        .await?;
    let report_id: i64 = row.get(0);

    // Insert executions preserving order.
    if !report.executions.is_empty() {
        let stmt_exec = client
            .prepare(
                "INSERT INTO executions \
                 (report_id, ord_index, order_id, version, avg_price, removed_input, added_output, fee, side) \
                 VALUES ($1, $2, $3, $4, ($5::numeric / $6::numeric), $7, $8, $9, $10)",
            )
            .await?;
        for (ix, ex) in report.executions.iter().enumerate() {
            let order_id = ex.id.to_string();
            let version = ex.version.to_string();
            let avg_num = ex.avg_price.numer().to_string();
            let avg_den = ex.avg_price.denom().to_string();
            let removed_input = ex.removed_input.to_string();
            let added_output = ex.added_output.to_string();
            let fee = ex.fee.to_string();
            let side = ex.side.to_string();
            client
                .execute(
                    &stmt_exec,
                    &[
                        &report_id,
                        &(ix as i32),
                        &order_id,
                        &version,
                        &avg_num,
                        &avg_den,
                        &removed_input,
                        &added_output,
                        &fee,
                        &side,
                    ],
                )
                .await?;
        }
    }

    // Insert events preserving order.
    if !report.events.is_empty() {
        let stmt_event = client
            .prepare("INSERT INTO events (report_id, ev_index, event_type, event_json) VALUES ($1, $2, $3, $4::jsonb)")
            .await?;
        for (ix, ev) in report.events.iter().enumerate() {
            let ev_type: &str = match ev {
                ExecutionEvent::SpotPrice(_) => "SpotPrice",
                ExecutionEvent::SpotPriceNotAvailable => "SpotPriceNotAvailable",
                ExecutionEvent::LiquidityBookSizePreAttempt(_) => "LiquidityBookSizePreAttempt",
                ExecutionEvent::LiquidityBookSizePostAttempt(_) => "LiquidityBookSizePostAttempt",
            };
            let ev_json_str = serde_json::to_string(ev)?;
            client
                .execute(&stmt_event, &[&report_id, &(ix as i32), &ev_type, &ev_json_str])
                .await?;
        }
    }

    info!(
        "Persisted report #{}, executions={}, events={}",
        report_id,
        report.executions.len(),
        report.events.len()
    );

    Ok(())
}
