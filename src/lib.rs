pub mod config;
pub mod error;
pub mod openmensa;
pub mod server;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

use crate::config::{Settings, user_agent};
use crate::openmensa::OpenMensaClient;
use crate::server::CanteenServer;

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .expect("the HTTP client has a valid configuration")
}

pub fn build_server(settings: &Settings, http: reqwest::Client) -> CanteenServer {
    let openmensa = OpenMensaClient::new(http, settings.openmensa_url.clone());
    CanteenServer::new(openmensa, settings.timezone)
}

pub fn app(settings: &Settings, server: CanteenServer) -> Router {
    let mcp = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        transport_config(settings),
    );
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .nest_service("/mcp", mcp)
}

fn transport_config(settings: &Settings) -> StreamableHttpServerConfig {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true);
    if settings.public_host.is_empty() {
        return config.with_allowed_hosts(Vec::<String>::new());
    }
    let host = &settings.public_host;
    config
        .with_allowed_hosts([host.clone(), "localhost".into(), "127.0.0.1".into()])
        .with_allowed_origins([format!("https://{host}"), "https://claude.ai".into()])
}
