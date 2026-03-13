#!/usr/bin/env bash
set -euo pipefail

CSV_URL="https://raw.githubusercontent.com/ff137/bitstamp-btcusd-minute-data/main/data/updates/btcusd_bitstamp_1min_latest.csv"
CSV_FILE="btcusd_bitstamp_1min_latest.csv"
PATCH_FILE="btcusd_bitstamp_1min_latest_patch.csv"
PG_CONTAINER="docker-pg_duckdb-1"
PG_USER="postgres"
TABLE="btc_usd"

curl -fsSL -o "$CSV_FILE" "$CSV_URL"

MAX_TS="$({
  docker exec "$PG_CONTAINER" psql -U "$PG_USER" -Atc "SELECT COALESCE(MAX(timestamp),0) FROM $TABLE"
})"

awk -F, -v max_ts="$MAX_TS" 'NR==1 || $1 > max_ts' "$CSV_FILE" > "$PATCH_FILE"

docker exec -i "$PG_CONTAINER" \
  psql -U "$PG_USER" -c "COPY $TABLE FROM STDIN WITH (FORMAT CSV, HEADER)" < "$PATCH_FILE"
