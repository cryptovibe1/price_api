# Real-Time Candles Plan

## Goal

Build a real-time BTC/USDT candle ingestion flow that:

- pulls fresh `1m` candles from Binance via `ccxt-rust`
- runs background workers with `flashq`
- continues from the latest stored candle in each database
- writes candles into each benchmark database independently
- keeps ingestion isolated per database so one failure does not block the others

## Target Scope

The new work should fit the current repo shape:

- API server: `apps/server`
- Web UI: `apps/ui_web`
- existing benchmark databases:
  - postgres
  - timescale
  - clickhouse via pg wrapper
  - duckdb via pg wrapper

## Assumptions

- historical schema from `init_btc_usdt.sql` remains the base table contract
- ingestion starts from the max stored candle timestamp per database
- source symbol is `BTC/USDT`
- source interval is `1m`
- exchange source is Binance
- workers run inside the server process first; standalone worker binary can be added later if needed

## Deliverables

1. exchange client module for fetching Binance OHLCV
2. worker/job layer for periodic real-time pulls
3. per-database repository layer to read last timestamp and insert new candles
4. dedup-safe insert/upsert behavior
5. configuration for poll interval, batch size, symbol, timeframe
6. logging and failure visibility for each database pipeline
7. short runbook in markdown for starting and verifying ingestion

## Implementation Plan

### Phase 1. Server structure

- add modules in `apps/server`:
  - `config`
  - `exchange`
  - `models`
  - `db`
  - `jobs`
  - `app_state`
- define a canonical candle model:
  - `timestamp`
  - `open`
  - `high`
  - `low`
  - `close`
  - `volume`
- keep timestamp normalized to Unix seconds in app code

### Phase 2. Exchange fetcher

- integrate `ccxt-rust`
- create Binance client wrapper with one responsibility:
  - fetch `BTC/USDT` `1m` candles
- support parameters:
  - `since`
  - `limit`
- convert exchange OHLCV payload into the internal candle model
- normalize Binance millisecond timestamps into seconds

### Phase 3. Database repository layer

- add one repository path per database backend
- implement shared operations:
  - `latest_candle_ts()`
  - `insert_candles(&[Candle])`
- keep SQL/backend differences behind the repository layer
- use backend-safe dedup behavior:
  - ignore existing timestamps
  - or upsert on `timestamp`

### Phase 4. Worker orchestration

- add `flashq` workers/jobs
- create separate jobs for:
  - postgres
  - timescale
  - clickhouse
  - duckdb
- each job loop:
  - read latest timestamp from db
  - fetch candles from Binance starting after that timestamp
  - filter already-seen candles defensively
  - persist new candles
- keep jobs independent so one database can fail without stopping the others

### Phase 5. Polling rules

- default polling interval: every 10-30 seconds
- fetch a small overlap window to protect against boundary misses
- only insert candles strictly newer than the stored max timestamp
- if no new candle is available, log at debug/info level and continue

### Phase 6. Config and startup

- add env/config values for:
  - exchange symbol
  - timeframe
  - poll interval
  - fetch limit
  - per-database connection strings
- start workers during server boot
- log worker startup clearly:
  - database target
  - symbol
  - timeframe
  - poll interval

### Phase 7. Observability

- add structured logs for:
  - job started
  - latest local timestamp
  - candles fetched
  - candles inserted
  - job failure
- include database name in every ingestion log line
- add a simple health/status endpoint later if needed

### Phase 8. Validation

- verify each database receives new rows
- verify duplicate runs do not create duplicate candles
- verify a database outage does not stop the other workers
- verify API endpoints read the newly inserted candles correctly

## Suggested File Layout

```text
apps/server/src/
  main.rs
  config.rs
  app_state.rs
  models.rs
  exchange/
    mod.rs
    binance.rs
  db/
    mod.rs
    postgres.rs
    timescale.rs
    clickhouse.rs
    duckdb.rs
  jobs/
    mod.rs
    realtime.rs
```

## Execution Order

1. Add candle model and config.
2. Add Binance fetch wrapper with a local smoke test path.
3. Add repository methods for `latest_candle_ts` and inserts.
4. Implement one working worker for postgres first.
5. Reuse the same worker flow for timescale, clickhouse, and duckdb.
6. Start all workers from server startup.
7. Add verification notes and operating commands.

## Risks

- `ccxt-rust` API surface may differ from the example and may need adaptation
- exchange rate limits or transient network failures need retry/backoff
- timestamp precision mismatch between exchange and DB can create duplicates if not normalized
- pg-wrapper backends may differ in upsert semantics and constraint support

## Open Decisions

- whether workers should live inside `apps/server` or a dedicated binary
- whether failed jobs should use retry with backoff or fixed-interval retry only
- whether inserts should be batched per fetch or per candle
- whether to add a separate raw ingestion table before writing into benchmark tables

## Minimal First Milestone

Deliver one server boot path that:

- starts a postgres real-time worker
- fetches Binance `BTC/USDT` `1m` candles
- resumes from the latest stored timestamp
- inserts only new candles
- logs every successful polling cycle

After that, extend the same pattern to timescale, clickhouse, and duckdb.
