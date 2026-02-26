CREATE TABLE btc_usd (
    timestamp   BIGINT NOT NULL,
    open    DECIMAL(18, 8) NOT NULL,
    high    DECIMAL(18, 8) NOT NULL,
    low     DECIMAL(18, 8) NOT NULL,
    close   DECIMAL(18, 8) NOT NULL,
    volume  DECIMAL(24, 10) NOT NULL,

    PRIMARY KEY (timestamp)
);

CREATE INDEX idx_btc_usd_time ON btc_usd (timestamp DESC);
