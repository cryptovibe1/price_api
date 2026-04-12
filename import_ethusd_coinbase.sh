#!/usr/bin/env bash
set -euo pipefail

CSV_FILE="${CSV_FILE:-ETHUSD_1m_Coinbase.csv}"
PG_CONTAINER="${PG_CONTAINER:-docker-pg_duckdb-1}"
PG_USER="${PG_USER:-postgres}"
PG_DB="${PG_DB:-postgres}"
TABLE="${TABLE:-eth_usd}"

if [[ ! -f "$CSV_FILE" ]]; then
  echo "missing CSV file: $CSV_FILE" >&2
  exit 1
fi

MAX_TS="$({
  docker exec "$PG_CONTAINER" psql -U "$PG_USER" -d "$PG_DB" -Atc \
    "SELECT COALESCE(MAX(timestamp),0) FROM $TABLE"
})"

python3 - "$CSV_FILE" "$MAX_TS" <<'PY' | docker exec -i "$PG_CONTAINER" \
  psql -U "$PG_USER" -d "$PG_DB" -c \
  "COPY $TABLE (timestamp, open, high, low, close, volume) FROM STDIN WITH (FORMAT CSV, HEADER)"
import csv
import datetime as dt
import sys

csv_file = sys.argv[1]
max_ts = int(sys.argv[2])

writer = csv.writer(sys.stdout, lineterminator="\n")
writer.writerow(["timestamp", "open", "high", "low", "close", "volume"])

with open(csv_file, "r", newline="") as fh:
    reader = csv.DictReader(fh)
    for row in reader:
        ts = int(dt.datetime.strptime(row["Open time"], "%Y-%m-%d %H:%M:%S").replace(
            tzinfo=dt.timezone.utc
        ).timestamp())
        if ts <= max_ts:
            continue
        writer.writerow([
            ts,
            row["Open"],
            row["High"],
            row["Low"],
            row["Close"],
            row["Volume"],
        ])
PY
