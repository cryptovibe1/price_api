use reqwest::Client;
use serde_json::Value;

use crate::models::Candle;

#[derive(Clone)]
pub struct BinanceExchange {
    client: Client,
    symbol: String,
    timeframe: String,
}

impl BinanceExchange {
    pub fn new(symbol: String, timeframe: String) -> Self {
        Self {
            client: Client::new(),
            symbol,
            timeframe,
        }
    }

    pub async fn fetch_ohlcv(
        &self,
        since_ts_seconds: Option<i64>,
        limit: u16,
    ) -> Result<Vec<Candle>, String> {
        let mut request = self
            .client
            .get("https://api.binance.com/api/v3/klines")
            .query(&[
                ("symbol", self.symbol.as_str()),
                ("interval", self.timeframe.as_str()),
            ])
            .query(&[("limit", limit)]);

        if let Some(since) = since_ts_seconds {
            let start_time_ms = since.saturating_mul(1000);
            request = request.query(&[("startTime", start_time_ms)]);
        }

        let response = request
            .send()
            .await
            .map_err(|err| format!("binance request failed: {err}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<body unavailable>".to_string());
            return Err(format!("binance error {status}: {body}"));
        }

        let rows: Vec<Vec<Value>> = response
            .json()
            .await
            .map_err(|err| format!("binance json decode failed: {err}"))?;

        rows.into_iter()
            .map(|row| {
                if row.len() < 6 {
                    return Err("binance returned malformed kline row".to_string());
                }

                let ts_ms = row[0]
                    .as_i64()
                    .ok_or_else(|| "binance timestamp is invalid".to_string())?;

                Ok(Candle {
                    timestamp: ts_ms / 1000,
                    open: parse_f64(&row[1])?,
                    high: parse_f64(&row[2])?,
                    low: parse_f64(&row[3])?,
                    close: parse_f64(&row[4])?,
                    volume: parse_f64(&row[5])?,
                })
            })
            .collect()
    }
}

fn parse_f64(value: &Value) -> Result<f64, String> {
    match value {
        Value::String(v) => v
            .parse::<f64>()
            .map_err(|_| format!("invalid float value: {v}")),
        Value::Number(v) => v
            .as_f64()
            .ok_or_else(|| "invalid numeric value".to_string()),
        _ => Err("unsupported float json value".to_string()),
    }
}
