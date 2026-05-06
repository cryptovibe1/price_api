# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Crypto price API benchmarking platform. Compares four PostgreSQL-compatible database engines (Postgres 18, pg_duckdb, pg_clickhouse, TimescaleDB) serving OHLCV candlestick data for BTC/USD, ETH/USD, and SOL/USD. Includes a Rust REST API server, a Binance realtime ingestion worker, and a WASM-based charting UI.

## Commands

### Infrastructure
```bash
docker-compose -f docker/pg.yaml up -d    # start all 4 database containers
```

### API Server (default: http://0.0.0.0:7878)
```bash
cargo run -p price-api-server --bin price-api-server
```

### Realtime Worker (pulls from Binance, ingests into all connected DBs)
```bash
cargo run -p price-api-server --bin price-api-worker                # all pairs (BTC, ETH, SOL)
REALTIME_SYMBOL=SOLUSDT cargo run -p price-api-server --bin price-api-worker  # single pair
```

### UI (WASM via trunk)
```bash
rustup target add wasm32-unknown-unknown
cargo install trunk
cd apps/ui_web && trunk serve index.html --open
```

### Build check
```bash
cargo check --workspace
cargo build -p price-api-server
```

## Architecture

**Workspace layout:** Two crates in `apps/` — `server` (API + worker binaries) and `ui_web` (WASM chart UI).

### Server (`apps/server`)

Two binaries share the same library:
- **`price-api-server`** (`src/main.rs`) — Salvo HTTP server. Routes: `GET /candles/<db>/<base>/<quote>?period=&ts_start=&ts_end=` and `WS /ws/<db>` for live candle updates. Global `AppState` held in a `OnceLock`, containing `CandleRepository` per `DbKind` and broadcast `Sender` per DB for websocket notifications.
- **`price-api-worker`** (`src/bin/worker.rs`) — Spawns one `spawn_realtime_workers` loop per `MarketPair`. Each loop polls Binance klines API, inserts new candles into all 4 DBs via `CandleRepository::insert_candles` with `ON CONFLICT DO NOTHING`.

Key modules:
- `models.rs` — `Candle`, `DbKind` (4 variants), `MarketPair` (4 pairs), `Period` (time aggregation parsing: `1min`..`1month`).
- `db.rs` — `CandleRepository` wraps `PgPool`. `aggregation_sql()` generates GROUP BY bucket queries (integer division for sub-month, month-bucket CTE for months).
- `exchange.rs` — `BinanceExchange` fetches from `api.binance.com/api/v3/klines`.
- `config.rs` — `AppConfig::from_env()` reads env vars. DB connections use fixed ports mapped in `docker/pg.yaml`: Postgres=6432, DuckDB=6132, TimescaleDB=6332, ClickHouse=6232.

### UI (`apps/ui_web`)

Single-file WASM app (`src/lib.rs`, ~1400 lines) using plotters-rs on HTML canvas. Renders candlestick + RSI charts with moving averages (Fibonacci-based defaults: 13, 21, 34, 55...), Fibonacci retracement tool, pan/zoom, log scale, and measure tool. Fetches from server API, connects via WebSocket for live updates. All state is in `thread_local!` `RefCell`s.

### Database Schema

Each asset pair has its own table (`btc_usd`, `eth_usd`, `sol_usd`, `xau_usd`) with identical schema: `(timestamp BIGINT PK, open, high, low, close, volume)`. All 4 DB engines use the same schema and are accessed via PostgreSQL wire protocol (sqlx).

## Environment Variables

| Variable | Default | Purpose |
|---|---|---|
| `REALTIME_SYMBOL` | `BTCUSDT` | Worker: which Binance pair to pull (`BTCUSDT`, `ETHUSDT`, `SOLUSDT`, `XAUTUSDT`). Omit to pull all. |
| `REALTIME_BOOTSTRAP_START_TS` | auto | Worker: Unix timestamp to start backfill from |
| `REALTIME_POLL_SECS` | `15` | Worker: poll interval seconds |
| `REALTIME_FETCH_LIMIT` | `500` | Worker: Binance klines per request |
| `SERVER_ADDR` | `0.0.0.0:7878` | Server bind address |
| `DB_PASSWORD` | `postgres1` | Password for all DB containers |
| `POSTGRES_URL` / `DUCKDB_URL` / `TIMESCALE_URL` / `CLICKHOUSE_URL` | auto from port map | Override individual DB connection strings |
