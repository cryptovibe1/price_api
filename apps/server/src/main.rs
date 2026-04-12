use std::sync::Arc;

use price_api_server::api::run_server;
use price_api_server::config::AppConfig;

#[tokio::main]
async fn main() {
    let config = Arc::new(AppConfig::from_env());
    run_server(config).await;
}
