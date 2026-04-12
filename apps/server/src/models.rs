use serde::Serialize;
use sqlx::FromRow;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Candle {
    pub timestamp: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandleUpdateEvent {
    pub db: String,
    pub latest_timestamp: i64,
    pub inserted: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarketPair {
    BtcUsd,
    EthUsd,
    SolUsd,
}

impl MarketPair {
    pub const ALL: [MarketPair; 3] = [MarketPair::BtcUsd, MarketPair::EthUsd, MarketPair::SolUsd];

    pub fn from_base_quote(base: &str, quote: &str) -> Result<Self, &'static str> {
        match (
            base.trim().to_ascii_lowercase().as_str(),
            quote.trim().to_ascii_lowercase().as_str(),
        ) {
            ("btc", "usd") => Ok(Self::BtcUsd),
            ("eth", "usd") => Ok(Self::EthUsd),
            ("sol", "usd") => Ok(Self::SolUsd),
            _ => Err("pair must be one of: btc/usd, eth/usd, sol/usd"),
        }
    }

    pub fn from_symbol(symbol: &str) -> Result<Self, &'static str> {
        match symbol.trim().to_ascii_uppercase().as_str() {
            "BTCUSDT" => Ok(Self::BtcUsd),
            "ETHUSDT" => Ok(Self::EthUsd),
            "SOLUSDT" => Ok(Self::SolUsd),
            _ => Err("REALTIME_SYMBOL must be one of: BTCUSDT, ETHUSDT, SOLUSDT"),
        }
    }

    pub fn table_name(self) -> &'static str {
        match self {
            Self::BtcUsd => "btc_usd",
            Self::EthUsd => "eth_usd",
            Self::SolUsd => "sol_usd",
        }
    }

    pub fn binance_symbol(self) -> &'static str {
        match self {
            Self::BtcUsd => "BTCUSDT",
            Self::EthUsd => "ETHUSDT",
            Self::SolUsd => "SOLUSDT",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Unit {
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, Copy)]
pub struct Period {
    pub size: i64,
    pub unit: Unit,
}

impl Period {
    pub fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim().to_ascii_lowercase();
        let digits_len = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();

        if digits_len == 0 || digits_len == trimmed.len() {
            return Err(
                "period must look like 1min/1mins, 1hour/1hours, 1day/1days, 1week/1weeks, 1month/1months"
                    .to_string(),
            );
        }

        let size = trimmed[..digits_len]
            .parse::<i64>()
            .map_err(|_| "period size is invalid".to_string())?;

        if size <= 0 {
            return Err("period size must be positive".to_string());
        }

        let unit = match &trimmed[digits_len..] {
            "min" | "mins" | "minute" | "minutes" => Unit::Minute,
            "hour" | "hours" => Unit::Hour,
            "day" | "days" => Unit::Day,
            "week" | "weeks" => Unit::Week,
            "month" | "months" => Unit::Month,
            _ => {
                return Err(
                    "period unit must be min/mins/hour/hours/day/days/week/weeks/month/months"
                        .to_string(),
                )
            }
        };

        Ok(Self { size, unit })
    }

    pub fn as_seconds(self) -> Option<i64> {
        let per_unit = match self.unit {
            Unit::Minute => 60,
            Unit::Hour => 60 * 60,
            Unit::Day => 60 * 60 * 24,
            Unit::Week => 60 * 60 * 24 * 7,
            Unit::Month => return None,
        };
        Some(per_unit * self.size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DbKind {
    Postgres,
    Duckdb,
    Timescale,
    Clickhouse,
}

impl DbKind {
    pub const ALL: [DbKind; 4] = [
        DbKind::Postgres,
        DbKind::Duckdb,
        DbKind::Timescale,
        DbKind::Clickhouse,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DbKind::Postgres => "postgres",
            DbKind::Duckdb => "duckdb",
            DbKind::Timescale => "timescale",
            DbKind::Clickhouse => "clickhouse",
        }
    }
}

impl fmt::Display for DbKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DbKind {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "postgres" => Ok(DbKind::Postgres),
            "duckdb" => Ok(DbKind::Duckdb),
            "timescale" => Ok(DbKind::Timescale),
            "clickhouse" => Ok(DbKind::Clickhouse),
            _ => Err("database must be one of: postgres, duckdb, timescale, clickhouse"),
        }
    }
}
