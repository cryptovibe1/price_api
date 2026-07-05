CREATE TABLE sol_btc (
    timestamp   BIGINT NOT NULL,
    open    DECIMAL(18, 8) NOT NULL,
    high    DECIMAL(18, 8) NOT NULL,
    low     DECIMAL(18, 8) NOT NULL,
    close   DECIMAL(18, 8) NOT NULL,
    volume  DECIMAL(24, 10) NOT NULL,

    PRIMARY KEY (timestamp)
);

CREATE INDEX idx_sol_btc_time ON sol_btc (timestamp DESC);
