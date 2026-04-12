use std::future::pending;
use std::sync::Arc;

use price_api_server::config::AppConfig;
use price_api_server::db::connect_repositories;
use price_api_server::exchange::BinanceExchange;
use price_api_server::jobs::spawn_realtime_workers;
use price_api_server::models::MarketPair;

#[tokio::main]
async fn main() {
    let requested_symbol = std::env::var("REALTIME_SYMBOL").ok();
    let pairs: Vec<MarketPair> = match requested_symbol {
        Some(symbol) => {
            vec![MarketPair::from_symbol(&symbol).unwrap_or_else(|err| panic!("{err}"))]
        }
        None => MarketPair::ALL.into_iter().collect(),
    };

    let base_config = Arc::new(AppConfig::from_env());
    let repos = Arc::new(connect_repositories(&base_config).await);

    for pair in pairs {
        let config = Arc::new(AppConfig::from_env_for_pair(pair));
        let exchange = Arc::new(BinanceExchange::new(
            config.symbol.clone(),
            config.timeframe.clone(),
        ));
        spawn_realtime_workers(config.clone(), exchange, repos.clone());
        println!(
            "price worker running for {} {}",
            config.symbol, config.timeframe
        );
    }

    pending::<()>().await;
}
