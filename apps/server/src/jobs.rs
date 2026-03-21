use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::time::{interval, MissedTickBehavior};

use crate::config::AppConfig;
use crate::db::CandleRepository;
use crate::exchange::BinanceExchange;
use crate::models::{Candle, DbKind};

pub fn spawn_realtime_workers(
    config: Arc<AppConfig>,
    exchange: Arc<BinanceExchange>,
    repos: Arc<HashMap<DbKind, CandleRepository>>,
) {
    for db in DbKind::ALL {
        if let Some(repo) = repos.get(&db).cloned() {
            let config = config.clone();
            let exchange = exchange.clone();
            tokio::spawn(async move {
                let mut ticker = interval(config.worker_poll_interval);
                ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

                loop {
                    ticker.tick().await;
                    if let Err(err) = sync_once(&config, &exchange, &repo).await {
                        eprintln!("realtime sync failed for {}: {}", repo.db(), err);
                    }
                }
            });
        }
    }
}

async fn sync_once(
    config: &AppConfig,
    exchange: &BinanceExchange,
    repo: &CandleRepository,
) -> Result<(), String> {
    let latest_ts = repo
        .latest_candle_ts()
        .await
        .map_err(|err| format!("latest_candle_ts failed: {err}"))?;

    let now_secs = now_unix_seconds();
    let fetch_since = latest_ts
        .map(|ts| ts + 60)
        .unwrap_or_else(|| now_secs - config.bootstrap_lookback_minutes * 60);

    let candles = exchange
        .fetch_ohlcv(Some(fetch_since), config.fetch_limit)
        .await?;

    let new_candles: Vec<Candle> = candles
        .into_iter()
        .filter(|candle| latest_ts.map(|ts| candle.timestamp > ts).unwrap_or(true))
        .collect();

    let inserted = repo
        .insert_candles(&new_candles)
        .await
        .map_err(|err| format!("insert_candles failed: {err}"))?;

    println!(
        "realtime sync {} | latest_ts {:?} | fetched {} | inserted {}",
        repo.db(),
        latest_ts,
        new_candles.len(),
        inserted
    );
    Ok(())
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|dur| dur.as_secs() as i64)
        .unwrap_or_default()
}
