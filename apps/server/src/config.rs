use std::env;
use std::time::Duration;

use crate::models::DbKind;

#[derive(Clone)]
pub struct AppConfig {
    pub server_addr: String,
    pub symbol: String,
    pub timeframe: String,
    pub fetch_limit: u16,
    pub worker_poll_interval: Duration,
    pub bootstrap_lookback_minutes: i64,
    pub db_password: String,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let db_password = env::var("DB_PASSWORD").unwrap_or_else(|_| "postgres1".to_string());
        Self {
            server_addr: env::var("SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:7878".to_string()),
            symbol: env::var("REALTIME_SYMBOL").unwrap_or_else(|_| "BTCUSDT".to_string()),
            timeframe: env::var("REALTIME_TIMEFRAME").unwrap_or_else(|_| "1m".to_string()),
            fetch_limit: env::var("REALTIME_FETCH_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(500),
            worker_poll_interval: Duration::from_secs(
                env::var("REALTIME_POLL_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(15),
            ),
            bootstrap_lookback_minutes: env::var("REALTIME_BOOTSTRAP_LOOKBACK_MINUTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(24 * 60),
            db_password,
        }
    }

    pub fn connection_url(&self, db: DbKind) -> String {
        let fallback = match db {
            DbKind::Postgres => format!(
                "postgres://postgres:{}@127.0.0.1:6432/postgres",
                self.db_password
            ),
            DbKind::Duckdb => format!(
                "postgres://postgres:{}@127.0.0.1:6132/postgres",
                self.db_password
            ),
            DbKind::Timescale => format!(
                "postgres://postgres:{}@127.0.0.1:6332/postgres",
                self.db_password
            ),
            DbKind::Clickhouse => format!(
                "postgres://postgres:{}@127.0.0.1:6232/postgres",
                self.db_password
            ),
        };

        match db {
            DbKind::Postgres => env::var("POSTGRES_URL").unwrap_or(fallback),
            DbKind::Duckdb => env::var("DUCKDB_URL").unwrap_or(fallback),
            DbKind::Timescale => env::var("TIMESCALE_URL").unwrap_or(fallback),
            DbKind::Clickhouse => env::var("CLICKHOUSE_URL").unwrap_or(fallback),
        }
    }
}
