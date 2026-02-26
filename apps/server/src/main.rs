use std::env;

use salvo::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
};
use salvo::http::{HeaderValue, Method, StatusCode};
use salvo::prelude::*;
use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Serialize, FromRow)]
struct Candle {
    timestamp: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Clone, Copy)]
enum Unit {
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, Copy)]
struct Period {
    size: i64,
    unit: Unit,
}

impl Period {
    fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim().to_ascii_lowercase();
        let digits_len = trimmed
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .count();

        if digits_len == 0 || digits_len == trimmed.len() {
            return Err("period must look like 1min, 1hour, 1day, 1week, 1month".to_string());
        }

        let size = trimmed[..digits_len]
            .parse::<i64>()
            .map_err(|_| "period size is invalid".to_string())?;

        if size <= 0 {
            return Err("period size must be positive".to_string());
        }

        let unit = match &trimmed[digits_len..] {
            "min" | "minute" | "minutes" => Unit::Minute,
            "hour" | "hours" => Unit::Hour,
            "day" | "days" => Unit::Day,
            "week" | "weeks" => Unit::Week,
            "month" | "months" => Unit::Month,
            _ => {
                return Err("period unit must be min/hour/day/week/month".to_string());
            }
        };

        Ok(Self { size, unit })
    }

    fn as_seconds(self) -> Option<i64> {
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

fn make_error(res: &mut Response, status: StatusCode, message: impl Into<String>) {
    set_cors_headers(res);
    res.status_code(status);
    res.render(Json(ErrorResponse {
        error: message.into(),
    }));
}

fn set_cors_headers(res: &mut Response) {
    let headers = res.headers_mut();
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET,OPTIONS"),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type"),
    );
}

fn connection_url(db: &str) -> Option<String> {
    let password = env::var("DB_PASSWORD").unwrap_or_else(|_| "postgres1".to_string());

    match db {
        "postgres" => Some(
            env::var("POSTGRES_URL")
                .unwrap_or_else(|_| format!("postgres://postgres:{password}@127.0.0.1:6432/postgres")),
        ),
        "duckdb" => Some(
            env::var("DUCKDB_URL")
                .unwrap_or_else(|_| format!("postgres://postgres:{password}@127.0.0.1:6132/postgres")),
        ),
        "timescale" => Some(
            env::var("TIMESCALE_URL")
                .unwrap_or_else(|_| format!("postgres://postgres:{password}@127.0.0.1:6332/postgres")),
        ),
        "clickhouse" => Some(
            env::var("CLICKHOUSE_URL")
                .unwrap_or_else(|_| format!("postgres://postgres:{password}@127.0.0.1:6232/postgres")),
        ),
        _ => None,
    }
}

fn aggregation_sql(period: Period) -> String {
    if let Some(bucket_seconds) = period.as_seconds() {
        return format!(
            "
            SELECT
                ((timestamp / {bucket_seconds}) * {bucket_seconds})::BIGINT AS timestamp,
                (ARRAY_AGG(open ORDER BY timestamp ASC))[1]::DOUBLE PRECISION AS open,
                MAX(high)::DOUBLE PRECISION AS high,
                MIN(low)::DOUBLE PRECISION AS low,
                (ARRAY_AGG(close ORDER BY timestamp DESC))[1]::DOUBLE PRECISION AS close,
                SUM(volume)::DOUBLE PRECISION AS volume
            FROM btc_usd
            WHERE timestamp BETWEEN $1 AND $2
            GROUP BY 1
            ORDER BY 1
            "
        );
    }

    let month_size = period.size;
    format!(
        "
        WITH buckets AS (
            SELECT
                timestamp,
                open,
                high,
                low,
                close,
                volume,
                (
                    (
                        (EXTRACT(YEAR FROM TO_TIMESTAMP(timestamp))::INT * 12)
                        + EXTRACT(MONTH FROM TO_TIMESTAMP(timestamp))::INT
                        - 1
                    ) / {month_size}
                ) * {month_size} AS month_bucket
            FROM btc_usd
            WHERE timestamp BETWEEN $1 AND $2
        )
        SELECT
            EXTRACT(EPOCH FROM MAKE_TIMESTAMP((month_bucket / 12), ((month_bucket % 12) + 1), 1, 0, 0, 0))::BIGINT AS timestamp,
            (ARRAY_AGG(open ORDER BY timestamp ASC))[1]::DOUBLE PRECISION AS open,
            MAX(high)::DOUBLE PRECISION AS high,
            MIN(low)::DOUBLE PRECISION AS low,
            (ARRAY_AGG(close ORDER BY timestamp DESC))[1]::DOUBLE PRECISION AS close,
            SUM(volume)::DOUBLE PRECISION AS volume
        FROM buckets
        GROUP BY month_bucket
        ORDER BY timestamp
        "
    )
}

#[handler]
async fn get_candles(req: &mut Request, res: &mut Response) {
    set_cors_headers(res);

    if req.method() == Method::OPTIONS {
        res.status_code(StatusCode::NO_CONTENT);
        return;
    }

    let db = match req.param::<String>("db") {
        Some(db) => db.to_ascii_lowercase(),
        None => {
            make_error(res, StatusCode::BAD_REQUEST, "missing database name");
            return;
        }
    };

    let base = req
        .param::<String>("base")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let quote = req
        .param::<String>("quote")
        .unwrap_or_default()
        .to_ascii_lowercase();

    if base != "btc" || quote != "usd" {
        make_error(
            res,
            StatusCode::NOT_FOUND,
            "only btc/usd pair is available right now",
        );
        return;
    }

    let period_raw = match req.query::<String>("period") {
        Some(period) => period,
        None => {
            make_error(res, StatusCode::BAD_REQUEST, "missing query param: period");
            return;
        }
    };

    let ts_start = match req.query::<i64>("ts_start") {
        Some(v) => v,
        None => {
            make_error(res, StatusCode::BAD_REQUEST, "missing query param: ts_start");
            return;
        }
    };

    let ts_end = match req.query::<i64>("ts_end") {
        Some(v) => v,
        None => {
            make_error(res, StatusCode::BAD_REQUEST, "missing query param: ts_end");
            return;
        }
    };

    if ts_end < ts_start {
        make_error(res, StatusCode::BAD_REQUEST, "ts_end must be >= ts_start");
        return;
    }

    let period = match Period::parse(&period_raw) {
        Ok(period) => period,
        Err(err) => {
            make_error(res, StatusCode::BAD_REQUEST, err);
            return;
        }
    };

    let db_url = match connection_url(&db) {
        Some(url) => url,
        None => {
            make_error(
                res,
                StatusCode::BAD_REQUEST,
                "database must be one of: postgres, duckdb, timescale, clickhouse",
            );
            return;
        }
    };

    let sql = aggregation_sql(period);
    let pool = match sqlx::PgPool::connect(&db_url).await {
        Ok(pool) => pool,
        Err(err) => {
            make_error(
                res,
                StatusCode::BAD_GATEWAY,
                format!("failed connecting to database: {err}"),
            );
            return;
        }
    };

    let result = sqlx::query_as::<_, Candle>(&sql)
        .bind(ts_start)
        .bind(ts_end)
        .fetch_all(&pool)
        .await;

    match result {
        Ok(candles) => {
            res.render(Json(candles));
        }
        Err(err) => {
            make_error(
                res,
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("query failed: {err}"),
            );
        }
    }
}

#[tokio::main]
async fn main() {
    let router = Router::new()
        .push(Router::with_path("candles/<db>/<base>/<quote>").get(get_candles).options(get_candles));

    let acceptor = TcpListener::new("0.0.0.0:7878").bind().await;
    println!("price api listening at http://0.0.0.0:7878");
    Server::new(acceptor).serve(router).await;
}
