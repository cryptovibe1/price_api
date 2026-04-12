use std::collections::HashMap;

use sqlx::{postgres::PgPoolOptions, postgres::PgRow, PgPool, Postgres, QueryBuilder, Row};

use crate::config::AppConfig;
use crate::models::{Candle, DbKind, MarketPair, Period};

#[derive(Clone)]
pub struct CandleRepository {
    db: DbKind,
    pool: PgPool,
}

impl CandleRepository {
    pub fn new(db: DbKind, pool: PgPool) -> Self {
        Self { db, pool }
    }

    pub fn db(&self) -> DbKind {
        self.db
    }

    pub async fn latest_candle_ts(&self, pair: MarketPair) -> Result<Option<i64>, sqlx::Error> {
        let sql = format!("SELECT MAX(timestamp) AS max_ts FROM {}", pair.table_name());
        let row: PgRow = sqlx::query(&sql).fetch_one(&self.pool).await?;
        row.try_get::<Option<i64>, _>("max_ts")
    }

    pub async fn insert_candles(
        &self,
        pair: MarketPair,
        candles: &[Candle],
    ) -> Result<u64, sqlx::Error> {
        if candles.is_empty() {
            return Ok(0);
        }

        let mut builder = QueryBuilder::<Postgres>::new(&format!(
            "INSERT INTO {} (timestamp, open, high, low, close, volume) ",
            pair.table_name()
        ));
        builder.push_values(candles, |mut b, candle| {
            b.push_bind(candle.timestamp)
                .push_bind(candle.open)
                .push_bind(candle.high)
                .push_bind(candle.low)
                .push_bind(candle.close)
                .push_bind(candle.volume);
        });
        builder.push(" ON CONFLICT (timestamp) DO NOTHING");

        let result = builder.build().execute(&self.pool).await?;
        Ok(result.rows_affected())
    }

    pub async fn query_aggregated(
        &self,
        pair: MarketPair,
        period: Period,
        ts_start: i64,
        ts_end: i64,
    ) -> Result<Vec<Candle>, sqlx::Error> {
        let sql = aggregation_sql(pair, period);
        sqlx::query_as::<_, Candle>(&sql)
            .bind(ts_start)
            .bind(ts_end)
            .fetch_all(&self.pool)
            .await
    }
}

pub async fn connect_repositories(config: &AppConfig) -> HashMap<DbKind, CandleRepository> {
    let mut repos = HashMap::new();
    for db in DbKind::ALL {
        let url = config.connection_url(db);
        match PgPoolOptions::new().max_connections(5).connect(&url).await {
            Ok(pool) => {
                println!("connected db pool for {}", db);
                repos.insert(db, CandleRepository::new(db, pool));
            }
            Err(err) => {
                eprintln!("failed connecting {}: {}", db, err);
            }
        }
    }
    repos
}

fn aggregation_sql(pair: MarketPair, period: Period) -> String {
    let table = pair.table_name();
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
            FROM {table}
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
            FROM {table}
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
