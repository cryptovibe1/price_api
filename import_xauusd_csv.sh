#!/usr/bin/env bash
set -euo pipefail

CSV_FILE="${CSV_FILE:-XAU_1m_data.csv}"
PG_USER="${PG_USER:-postgres}"
PG_DB="${PG_DB:-postgres}"
TABLE="${TABLE:-xau_usd}"

if [[ -n "${PG_CONTAINER:-}" ]]; then
  CONTAINERS=("$PG_CONTAINER")
else
  CONTAINERS=(
    docker-postgres_18-1
    docker-timescaledb-1
    docker-pg_clickhouse-1
    docker-pg_duckdb-1
  )
fi

if [[ ! -f "$CSV_FILE" ]]; then
  echo "missing CSV file: $CSV_FILE" >&2
  exit 1
fi

for container in "${CONTAINERS[@]}"; do
  echo "importing $CSV_FILE into $container:$TABLE" >&2

  MAX_TS="$({
    docker exec "$container" psql -U "$PG_USER" -d "$PG_DB" -Atc \
      "SELECT COALESCE(MAX(timestamp),0) FROM $TABLE"
  })"

  python3 - "$CSV_FILE" "$MAX_TS" <<'PY' | docker exec -i "$container" \
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
    reader = csv.DictReader(fh, delimiter=";")
    for row in reader:
        ts = int(dt.datetime.strptime(row["Date"], "%Y.%m.%d %H:%M").replace(
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
done
