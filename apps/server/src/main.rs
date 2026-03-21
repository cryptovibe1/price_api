mod config;
mod db;
mod exchange;
mod jobs;
mod models;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use config::AppConfig;
use db::{connect_repositories, CandleRepository};
use exchange::BinanceExchange;
use jobs::spawn_realtime_workers;
use models::{DbKind, ErrorResponse, Period};
use salvo::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
};
use salvo::http::{HeaderValue, Method, StatusCode};
use salvo::prelude::*;

static APP_STATE: OnceLock<AppState> = OnceLock::new();

#[derive(Clone)]
struct AppState {
    repos: Arc<HashMap<DbKind, CandleRepository>>,
}

impl AppState {
    fn repo(&self, db: DbKind) -> Option<&CandleRepository> {
        self.repos.get(&db)
    }
}

fn app_state() -> &'static AppState {
    APP_STATE.get().expect("app state must be initialized")
}

fn make_error(res: &mut Response, status: StatusCode, message: impl Into<String>) {
    set_cors_headers(res);
    res.status_code(status);
    res.render(Json(ErrorResponse {
        error: message.into(),
    }));
}

fn set_cors_headers(res: &mut Response) {
    let headers = res.headers_mut();
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET,OPTIONS"),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type"),
    );
}

#[handler]
async fn get_candles(req: &mut Request, res: &mut Response) {
    set_cors_headers(res);

    if req.method() == Method::OPTIONS {
        res.status_code(StatusCode::NO_CONTENT);
        return;
    }

    let db = match req.param::<String>("db") {
        Some(db) => match db.parse::<DbKind>() {
            Ok(db) => db,
            Err(err) => {
                make_error(res, StatusCode::BAD_REQUEST, err);
                return;
            }
        },
        None => {
            make_error(res, StatusCode::BAD_REQUEST, "missing database name");
            return;
        }
    };

    let base = req
        .param::<String>("base")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let quote = req
        .param::<String>("quote")
        .unwrap_or_default()
        .to_ascii_lowercase();

    if base != "btc" || quote != "usd" {
        make_error(
            res,
            StatusCode::NOT_FOUND,
            "only btc/usd pair is available right now",
        );
        return;
    }

    let period_raw = match req.query::<String>("period") {
        Some(period) => period,
        None => {
            make_error(res, StatusCode::BAD_REQUEST, "missing query param: period");
            return;
        }
    };

    let ts_start = match req.query::<i64>("ts_start") {
        Some(v) => v,
        None => {
            make_error(
                res,
                StatusCode::BAD_REQUEST,
                "missing query param: ts_start",
            );
            return;
        }
    };

    let ts_end = match req.query::<i64>("ts_end") {
        Some(v) => v,
        None => {
            make_error(res, StatusCode::BAD_REQUEST, "missing query param: ts_end");
            return;
        }
    };

    if ts_end < ts_start {
        make_error(res, StatusCode::BAD_REQUEST, "ts_end must be >= ts_start");
        return;
    }

    let period = match Period::parse(&period_raw) {
        Ok(period) => period,
        Err(err) => {
            make_error(res, StatusCode::BAD_REQUEST, err);
            return;
        }
    };

    let Some(repo) = app_state().repo(db) else {
        make_error(
            res,
            StatusCode::BAD_GATEWAY,
            format!("database {} is unavailable", db),
        );
        return;
    };

    match repo.query_aggregated(period, ts_start, ts_end).await {
        Ok(candles) => res.render(Json(candles)),
        Err(err) => make_error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("query failed: {err}"),
        ),
    }
}

#[tokio::main]
async fn main() {
    let config = Arc::new(AppConfig::from_env());
    let exchange = Arc::new(BinanceExchange::new(
        config.symbol.clone(),
        config.timeframe.clone(),
    ));
    let repos = Arc::new(connect_repositories(&config).await);

    if APP_STATE
        .set(AppState {
            repos: repos.clone(),
        })
        .is_err()
    {
        panic!("app state must be set once");
    }

    spawn_realtime_workers(config.clone(), exchange, repos);

    let router = Router::new().push(
        Router::with_path("candles/<db>/<base>/<quote>")
            .get(get_candles)
            .options(get_candles),
    );

    let acceptor = TcpListener::new(config.server_addr.as_str()).bind().await;
    println!("price api listening at http://{}", config.server_addr);
    Server::new(acceptor).serve(router).await;
}
