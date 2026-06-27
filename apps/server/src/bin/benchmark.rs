/// Run: cargo run -p price-api-server --bin benchmark
///
/// Env vars:
///   BENCH_ITERS=10       iterations per (db × scenario), default 10
///   BENCH_CSV=1          also emit results as CSV to benchmark_results.csv
///   DB_PASSWORD=...      shared across all DBs

use std::collections::HashMap;
use std::fs::File;
use std::io::Write as IoWrite;
use std::time::{Duration, Instant};

use price_api_server::config::AppConfig;
use price_api_server::db::{aggregation_sql, connect_repositories, virtual_pair_aggregation_sql};
use price_api_server::models::{DbKind, MarketPair, Period};
use sqlx::PgPool;

const DAY: i64 = 86_400;
const WEEK: i64 = 7 * DAY;
const MONTH: i64 = 30 * DAY;
const YEAR: i64 = 365 * DAY;

// ── scenario types ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
enum QueryKind {
    Single(MarketPair),
    Virtual { num: MarketPair, den: MarketPair },
}

#[derive(Clone, Debug)]
struct Scenario {
    pair_label: &'static str,
    timeframe_label: &'static str,
    period_label: &'static str,
    kind: QueryKind,
    lookback_secs: Option<i64>, // None → all available data
    period: Period,
}

impl Scenario {
    fn sql(&self) -> String {
        match self.kind {
            QueryKind::Single(pair) => aggregation_sql(pair, self.period),
            QueryKind::Virtual { num, den } => virtual_pair_aggregation_sql(num, den, self.period),
        }
    }

    /// The pair used to look up ts_max in this DB.
    fn anchor_pair(&self) -> MarketPair {
        match self.kind {
            QueryKind::Single(p) => p,
            QueryKind::Virtual { num, .. } => num,
        }
    }
}

// ── result ────────────────────────────────────────────────────────────────────

struct BenchResult {
    scenario_idx: usize,
    db: DbKind,
    rows: usize,
    samples: Vec<Duration>,
    error: Option<String>,
}

impl BenchResult {
    fn mean(&self) -> Duration {
        if self.samples.is_empty() {
            return Duration::ZERO;
        }
        self.samples.iter().sum::<Duration>() / self.samples.len() as u32
    }

    fn percentile(&self, p: f64) -> Duration {
        if self.samples.is_empty() {
            return Duration::ZERO;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
        sorted[idx]
    }
}

// ── scenario list ─────────────────────────────────────────────────────────────

fn build_scenarios() -> Vec<Scenario> {
    let mut out = Vec::new();

    // (timeframe_label, lookback_secs, period_str)
    static SINGLE_CONFIGS: &[(&str, Option<i64>, &str)] = &[
        ("1d", Some(DAY), "1min"),
        ("1d", Some(DAY), "5min"),
        ("1d", Some(DAY), "15min"),
        ("1d", Some(DAY), "1hour"),
        ("1w", Some(WEEK), "5min"),
        ("1w", Some(WEEK), "15min"),
        ("1w", Some(WEEK), "1hour"),
        ("1w", Some(WEEK), "4hour"),
        ("1mo", Some(MONTH), "1hour"),
        ("1mo", Some(MONTH), "4hour"),
        ("1mo", Some(MONTH), "1day"),
        ("3mo", Some(3 * MONTH), "4hour"),
        ("3mo", Some(3 * MONTH), "1day"),
        ("3mo", Some(3 * MONTH), "1week"),
        ("1y", Some(YEAR), "1day"),
        ("1y", Some(YEAR), "1week"),
        ("1y", Some(YEAR), "1month"),
        ("alltime", None, "1day"),
        ("alltime", None, "1week"),
        ("alltime", None, "1month"),
    ];

    static PAIRS: &[(MarketPair, &str)] = &[
        (MarketPair::BtcUsd, "btc/usd"),
        (MarketPair::EthUsd, "eth/usd"),
        (MarketPair::SolUsd, "sol/usd"),
        (MarketPair::XauUsd, "xau/usd"),
    ];

    for &(pair, pair_label) in PAIRS {
        for &(tf_label, lookback, period_str) in SINGLE_CONFIGS {
            out.push(Scenario {
                pair_label,
                timeframe_label: tf_label,
                period_label: period_str,
                kind: QueryKind::Single(pair),
                lookback_secs: lookback,
                period: Period::parse(period_str).unwrap(),
            });
        }
    }

    // Virtual (cross-rate) pairs: numerator / denominator, both in USD
    static VIRTUAL_PAIRS: &[(MarketPair, MarketPair, &str)] = &[
        (MarketPair::EthUsd, MarketPair::BtcUsd, "eth/btc"),
        (MarketPair::SolUsd, MarketPair::BtcUsd, "sol/btc"),
        (MarketPair::SolUsd, MarketPair::EthUsd, "sol/eth"),
        (MarketPair::XauUsd, MarketPair::BtcUsd, "xau/btc"),
    ];

    static VIRTUAL_CONFIGS: &[(&str, Option<i64>, &str)] = &[
        ("1d", Some(DAY), "1min"),
        ("1d", Some(DAY), "1hour"),
        ("1w", Some(WEEK), "1hour"),
        ("1w", Some(WEEK), "4hour"),
        ("1mo", Some(MONTH), "4hour"),
        ("1mo", Some(MONTH), "1day"),
        ("1y", Some(YEAR), "1day"),
        ("1y", Some(YEAR), "1week"),
        ("alltime", None, "1week"),
        ("alltime", None, "1month"),
    ];

    for &(num, den, pair_label) in VIRTUAL_PAIRS {
        for &(tf_label, lookback, period_str) in VIRTUAL_CONFIGS {
            out.push(Scenario {
                pair_label,
                timeframe_label: tf_label,
                period_label: period_str,
                kind: QueryKind::Virtual { num, den },
                lookback_secs: lookback,
                period: Period::parse(period_str).unwrap(),
            });
        }
    }

    out
}

// ── helpers ───────────────────────────────────────────────────────────────────

async fn ts_max_for(pool: &PgPool, pair: MarketPair) -> Option<i64> {
    let sql = format!("SELECT MAX(timestamp) FROM {}", pair.table_name());
    sqlx::query_scalar::<_, Option<i64>>(&sql)
        .fetch_one(pool)
        .await
        .ok()
        .flatten()
}

async fn ts_min_for(pool: &PgPool, pair: MarketPair) -> Option<i64> {
    let sql = format!("SELECT MIN(timestamp) FROM {}", pair.table_name());
    sqlx::query_scalar::<_, Option<i64>>(&sql)
        .fetch_one(pool)
        .await
        .ok()
        .flatten()
}

fn fmt_dur(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms >= 1000.0 {
        format!("{:.1}s ", ms / 1000.0)
    } else {
        format!("{:.1}ms", ms)
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let iters: usize = std::env::var("BENCH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let emit_csv = std::env::var("BENCH_CSV").unwrap_or_default() == "1";

    println!("=== Price API Database Benchmarks ===");
    println!("Iterations per scenario: {iters}");
    println!();

    // Connect to all DBs
    let config = AppConfig::from_env();
    let repos = connect_repositories(&config).await;

    if repos.is_empty() {
        eprintln!("No databases connected — aborting.");
        std::process::exit(1);
    }

    println!(
        "Connected: {}",
        repos
            .keys()
            .map(|d| d.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!();

    let scenarios = build_scenarios();
    let total = scenarios.len() * repos.len();
    println!("Scenarios: {}  ×  DBs: {}  =  {} benchmark runs", scenarios.len(), repos.len(), total);
    println!();

    // Pre-compute ts_max and ts_min per (db, pair) so we don't pay for it during timing
    println!("Probing data ranges...");
    let mut ts_max_cache: HashMap<(DbKind, MarketPair), i64> = HashMap::new();
    let mut ts_min_cache: HashMap<(DbKind, MarketPair), i64> = HashMap::new();
    for (&db, repo) in &repos {
        for pair in MarketPair::ALL {
            if let Some(max) = ts_max_for(repo.pool(), pair).await {
                ts_max_cache.insert((db, pair), max);
            }
            if let Some(min) = ts_min_for(repo.pool(), pair).await {
                ts_min_cache.insert((db, pair), min);
            }
        }
    }

    // Run benchmarks
    let mut results: Vec<BenchResult> = Vec::new();
    let mut completed = 0usize;

    for (scenario_idx, scenario) in scenarios.iter().enumerate() {
        let anchor = scenario.anchor_pair();

        for (&db, repo) in &repos {
            let Some(&ts_max) = ts_max_cache.get(&(db, anchor)) else {
                completed += 1;
                continue;
            };

            let ts_end = ts_max;
            let ts_start = match scenario.lookback_secs {
                Some(secs) => ts_end - secs,
                None => ts_min_cache.get(&(db, anchor)).copied().unwrap_or(0),
            };

            // For virtual pairs, also ensure denominator has data
            if let QueryKind::Virtual { den, .. } = scenario.kind {
                if ts_max_cache.get(&(db, den)).is_none() {
                    completed += 1;
                    continue;
                }
            }

            let sql = scenario.sql();
            let mut samples: Vec<Duration> = Vec::with_capacity(iters);
            let mut rows = 0usize;
            let mut error: Option<String> = None;

            for i in 0..iters {
                let t0 = Instant::now();
                match sqlx::query(&sql)
                    .bind(ts_start)
                    .bind(ts_end)
                    .fetch_all(repo.pool())
                    .await
                {
                    Ok(rs) => {
                        let elapsed = t0.elapsed();
                        if i == 0 {
                            rows = rs.len();
                        }
                        samples.push(elapsed);
                    }
                    Err(e) => {
                        error = Some(e.to_string());
                        break;
                    }
                }
            }

            completed += 1;
            print!(
                "\r  [{:>4}/{total}] {:<12} {:<8} {:<6} {} {:<10} … {:<9}",
                completed,
                scenario.pair_label,
                scenario.timeframe_label,
                scenario.period_label,
                if error.is_some() { "ERR" } else { "   " },
                db.as_str(),
                if samples.is_empty() {
                    "failed   ".to_string()
                } else {
                    let mean = samples.iter().sum::<Duration>() / samples.len() as u32;
                    fmt_dur(mean)
                }
            );
            let _ = std::io::stdout().flush();

            results.push(BenchResult {
                scenario_idx,
                db,
                rows,
                samples,
                error,
            });
        }
    }

    println!("\r{:<80}", ""); // clear progress line

    // ── Print results table ───────────────────────────────────────────────────

    let col_pair = 13usize;
    let col_tf = 8usize;
    let col_per = 7usize;
    let col_db = 11usize;
    let col_rows = 6usize;
    let col_num = 9usize;

    let header = format!(
        "{:<col_pair$} {:<col_tf$} {:<col_per$} {:<col_db$} {:>col_rows$}  {:>col_num$} {:>col_num$} {:>col_num$} {:>col_num$}",
        "Pair", "Timeframe", "Period", "DB", "Rows", "Mean", "P50", "P95", "P99"
    );
    let sep = "-".repeat(header.len());

    println!("{header}");
    println!("{sep}");

    let mut last_group = ("", "", "");

    for (scenario_idx, scenario) in scenarios.iter().enumerate() {
        let group = (scenario.pair_label, scenario.timeframe_label, scenario.period_label);
        if group != last_group {
            if last_group != ("", "", "") {
                println!();
            }
            last_group = group;
        }

        // Collect DB results for this scenario, sorted for consistent order
        let mut db_results: Vec<&BenchResult> = results
            .iter()
            .filter(|r| r.scenario_idx == scenario_idx)
            .collect();
        db_results.sort_by_key(|r| r.db.as_str());

        for r in &db_results {
            if let Some(err) = &r.error {
                println!(
                    "{:<col_pair$} {:<col_tf$} {:<col_per$} {:<col_db$} {:>col_rows$}  {:<9}",
                    scenario.pair_label,
                    scenario.timeframe_label,
                    scenario.period_label,
                    r.db.as_str(),
                    "-",
                    format!("ERR: {}", &err[..err.len().min(40)])
                );
                continue;
            }
            if r.samples.is_empty() {
                continue;
            }
            println!(
                "{:<col_pair$} {:<col_tf$} {:<col_per$} {:<col_db$} {:>col_rows$}  {:>col_num$} {:>col_num$} {:>col_num$} {:>col_num$}",
                scenario.pair_label,
                scenario.timeframe_label,
                scenario.period_label,
                r.db.as_str(),
                r.rows,
                fmt_dur(r.mean()),
                fmt_dur(r.percentile(50.0)),
                fmt_dur(r.percentile(95.0)),
                fmt_dur(r.percentile(99.0)),
            );
        }
    }

    println!();
    println!("{sep}");

    // ── Summary: wins per DB ──────────────────────────────────────────────────

    println!();
    println!("=== Summary: fastest DB per scenario ===");
    println!();

    let mut wins: HashMap<DbKind, usize> = HashMap::new();
    let mut total_mean: HashMap<DbKind, Vec<Duration>> = HashMap::new();
    let mut scenario_count = 0usize;

    for scenario_idx in 0..scenarios.len() {
        let scenario = &scenarios[scenario_idx];
        let db_results: Vec<&BenchResult> = results
            .iter()
            .filter(|r| r.scenario_idx == scenario_idx && !r.samples.is_empty() && r.error.is_none())
            .collect();

        if db_results.is_empty() {
            continue;
        }
        scenario_count += 1;

        let winner = db_results
            .iter()
            .min_by_key(|r| r.mean())
            .unwrap();

        *wins.entry(winner.db).or_insert(0) += 1;

        for r in &db_results {
            total_mean.entry(r.db).or_default().push(r.mean());
        }

        let winner_mean = fmt_dur(winner.mean());
        println!(
            "  {:<13} {:<8} {:<6}  →  {} ({})",
            scenario.pair_label,
            scenario.timeframe_label,
            scenario.period_label,
            winner.db.as_str(),
            winner_mean
        );
    }

    println!();
    println!("=== Overall performance ===");
    println!();

    let col_w = 12usize;
    let col_n = 8usize;
    println!(
        "  {:<col_w$} {:>7} {:>col_n$} {:>col_n$}",
        "DB", "Wins", "AvgMean", "P50Mean"
    );
    println!("  {}", "-".repeat(col_w + 7 + col_n * 2 + 6));

    let mut db_summary: Vec<(DbKind, usize, Duration, Duration)> = DbKind::ALL
        .iter()
        .filter_map(|&db| {
            let means = total_mean.get(&db)?;
            if means.is_empty() {
                return None;
            }
            let avg = means.iter().sum::<Duration>() / means.len() as u32;
            let mut sorted = means.clone();
            sorted.sort_unstable();
            let p50 = sorted[sorted.len() / 2];
            Some((db, *wins.get(&db).unwrap_or(&0), avg, p50))
        })
        .collect();
    db_summary.sort_by_key(|&(_, _, avg, _)| avg);

    for (db, w, avg, p50) in &db_summary {
        let pct = if scenario_count > 0 {
            (*w as f64 / scenario_count as f64) * 100.0
        } else {
            0.0
        };
        println!(
            "  {:<col_w$} {:>4} ({:4.1}%)  {:>col_n$} {:>col_n$}",
            db.as_str(),
            w,
            pct,
            fmt_dur(*avg),
            fmt_dur(*p50)
        );
    }

    println!();

    // ── Optional CSV export ───────────────────────────────────────────────────

    if emit_csv {
        let path = "benchmark_results.csv";
        match File::create(path) {
            Ok(mut f) => {
                writeln!(f, "pair,timeframe,period,db,rows,mean_ms,p50_ms,p95_ms,p99_ms,error")
                    .ok();
                for r in &results {
                    let s = &scenarios[r.scenario_idx];
                    if let Some(err) = &r.error {
                        writeln!(
                            f,
                            "{},{},{},{},0,,,,, \"{}\"",
                            s.pair_label, s.timeframe_label, s.period_label, r.db, err
                        )
                        .ok();
                    } else if !r.samples.is_empty() {
                        writeln!(
                            f,
                            "{},{},{},{},{},{:.3},{:.3},{:.3},{:.3},",
                            s.pair_label,
                            s.timeframe_label,
                            s.period_label,
                            r.db,
                            r.rows,
                            r.mean().as_secs_f64() * 1000.0,
                            r.percentile(50.0).as_secs_f64() * 1000.0,
                            r.percentile(95.0).as_secs_f64() * 1000.0,
                            r.percentile(99.0).as_secs_f64() * 1000.0,
                        )
                        .ok();
                    }
                }
                println!("CSV written to {path}");
            }
            Err(e) => eprintln!("Failed to write CSV: {e}"),
        }
    }
}
