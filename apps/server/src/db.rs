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

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn latest_candle_ts(&self, pair: MarketPair) -> Result<Option<i64>, sqlx::Error> {
        let sql = format!("SELECT MAX(timestamp) AS max_ts FROM {}", pair.table_name());
        let row: PgRow = sqlx::query(&sql).fetch_one(&self.pool).await?;
        row.try_get::<Option<i64>, _>("max_ts")
    }

    pub async fn latest_candle(&self, pair: MarketPair) -> Result<Option<Candle>, sqlx::Error> {
        let sql = format!(
            "SELECT \
                timestamp, \
                open::DOUBLE PRECISION AS open, \
                high::DOUBLE PRECISION AS high, \
                low::DOUBLE PRECISION AS low, \
                close::DOUBLE PRECISION AS close, \
                volume::DOUBLE PRECISION AS volume \
             FROM {} ORDER BY timestamp DESC LIMIT 1",
            pair.table_name()
        );
        sqlx::query_as::<_, Candle>(&sql)
            .fetch_optional(&self.pool)
            .await
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

pub fn aggregation_sql(pair: MarketPair, period: Period) -> String {
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

/// Generates SQL for a synthetic cross-rate pair (numerator / denominator).
/// e.g. ETH/BTC = eth_usd prices divided by btc_usd prices, aligned to the same time bucket.
pub fn virtual_pair_aggregation_sql(num: MarketPair, den: MarketPair, period: Period) -> String {
    let num_t = num.table_name();
    let den_t = den.table_name();

    if let Some(bucket) = period.as_seconds() {
        return format!(
            "
            WITH
            num AS (
                SELECT
                    ((timestamp / {bucket}) * {bucket}) AS ts_b,
                    (ARRAY_AGG(open  ORDER BY timestamp ASC ))[1] AS open,
                    MAX(high) AS high,
                    MIN(low)  AS low,
                    (ARRAY_AGG(close ORDER BY timestamp DESC))[1] AS close
                FROM {num_t}
                WHERE timestamp BETWEEN $1 AND $2
                GROUP BY 1
            ),
            den AS (
                SELECT
                    ((timestamp / {bucket}) * {bucket}) AS ts_b,
                    (ARRAY_AGG(open  ORDER BY timestamp ASC ))[1] AS open,
                    MAX(high) AS high,
                    MIN(low)  AS low,
                    (ARRAY_AGG(close ORDER BY timestamp DESC))[1] AS close
                FROM {den_t}
                WHERE timestamp BETWEEN $1 AND $2
                GROUP BY 1
            )
            SELECT
                num.ts_b::BIGINT                                        AS timestamp,
                (num.open  / NULLIF(den.open,  0))::DOUBLE PRECISION    AS open,
                (num.high  / NULLIF(den.low,   0))::DOUBLE PRECISION    AS high,
                (num.low   / NULLIF(den.high,  0))::DOUBLE PRECISION    AS low,
                (num.close / NULLIF(den.close, 0))::DOUBLE PRECISION    AS close,
                0.0::DOUBLE PRECISION                                   AS volume
            FROM num JOIN den ON num.ts_b = den.ts_b
            ORDER BY 1
            "
        );
    }

    let month_size = period.size;
    format!(
        "
        WITH
        num_b AS (
            SELECT timestamp, open, high, low, close,
                (
                    ((EXTRACT(YEAR  FROM TO_TIMESTAMP(timestamp))::INT * 12)
                     + EXTRACT(MONTH FROM TO_TIMESTAMP(timestamp))::INT - 1)
                    / {month_size}
                ) * {month_size} AS mb
            FROM {num_t} WHERE timestamp BETWEEN $1 AND $2
        ),
        den_b AS (
            SELECT timestamp, open, high, low, close,
                (
                    ((EXTRACT(YEAR  FROM TO_TIMESTAMP(timestamp))::INT * 12)
                     + EXTRACT(MONTH FROM TO_TIMESTAMP(timestamp))::INT - 1)
                    / {month_size}
                ) * {month_size} AS mb
            FROM {den_t} WHERE timestamp BETWEEN $1 AND $2
        ),
        num_agg AS (
            SELECT mb,
                (ARRAY_AGG(open  ORDER BY timestamp ASC ))[1] AS open,
                MAX(high) AS high, MIN(low) AS low,
                (ARRAY_AGG(close ORDER BY timestamp DESC))[1] AS close
            FROM num_b GROUP BY mb
        ),
        den_agg AS (
            SELECT mb,
                (ARRAY_AGG(open  ORDER BY timestamp ASC ))[1] AS open,
                MAX(high) AS high, MIN(low) AS low,
                (ARRAY_AGG(close ORDER BY timestamp DESC))[1] AS close
            FROM den_b GROUP BY mb
        )
        SELECT
            EXTRACT(EPOCH FROM MAKE_TIMESTAMP(
                (num_agg.mb / 12), ((num_agg.mb % 12) + 1), 1, 0, 0, 0
            ))::BIGINT                                                   AS timestamp,
            (num_agg.open  / NULLIF(den_agg.open,  0))::DOUBLE PRECISION AS open,
            (num_agg.high  / NULLIF(den_agg.low,   0))::DOUBLE PRECISION AS high,
            (num_agg.low   / NULLIF(den_agg.high,  0))::DOUBLE PRECISION AS low,
            (num_agg.close / NULLIF(den_agg.close, 0))::DOUBLE PRECISION AS close,
            0.0::DOUBLE PRECISION                                        AS volume
        FROM num_agg JOIN den_agg ON num_agg.mb = den_agg.mb
        ORDER BY timestamp
        "
    )
}
