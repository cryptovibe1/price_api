### price_api

#### tech stack
```
salvo rs as web server
web ui use plotters-rs plotters
```

### Plan
```
Create salvo rest ap endpoints regarding databases 
/candles/postgres/btc/usd
/candles/duckdb/btc/usd
/candles/timescale/btc/usd
/candles/clickhouse/btc/usd
query params:
period: '1min', '1hour', '1day', '1week', '1month' // where 1 can be changed by n
ts_start: {timestamp}
ts_end: {timestamp}

returns:
    aggregated candels
    {
        timestamp
        open
        high
        low
        close
        volume
    }
```

### db servers already launched
```
connections takes from docker/pg.yaml
```

### dirs
```
apps/server - rust api server
apps/ui_web - web ui receive data from server
```
