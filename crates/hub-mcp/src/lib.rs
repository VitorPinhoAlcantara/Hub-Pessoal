mod tools;

use std::hash::{BuildHasher, Hasher};
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use hub_core::HubContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::sync::oneshot;

pub use tools::HubTools;

pub const SERVER_NAME: &str = "hub";
pub const MCP_PATH: &str = "/mcp";

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("não foi possível abrir a porta {port}: {source}")]
    Bind { port: u16, source: std::io::Error },
    #[error("erro de arquivo: {0}")]
    Io(#[from] std::io::Error),
    #[error("falha ao gravar a configuração: {0}")]
    Config(#[from] hub_core::ConfigError),
}

pub struct McpServer {
    pub addr: SocketAddr,
    pub token: String,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl McpServer {
    pub fn url(&self) -> String {
        format!("http://{}{MCP_PATH}", self.addr)
    }

    pub fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn generate_token() -> String {
    let state = std::collections::hash_map::RandomState::new();
    (0..4)
        .map(|i| {
            let mut hasher = state.build_hasher();
            hasher.write_u64(i);
            hasher.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default(),
            );
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

pub fn ensure_token(ctx: &HubContext) -> Result<String, McpError> {
    let mut config = ctx.config();
    if !config.mcp.token.trim().is_empty() {
        return Ok(config.mcp.token);
    }
    config.mcp.token = generate_token();
    config.save(&ctx.paths.config_file())?;
    let token = config.mcp.token.clone();
    *ctx.config.lock().unwrap_or_else(|e| e.into_inner()) = config;
    Ok(token)
}

pub fn client_config_path(ctx: &HubContext) -> PathBuf {
    ctx.paths.data_dir.join("mcp.json")
}

pub fn client_config(url: &str, token: &str) -> serde_json::Value {
    serde_json::json!({
        "mcpServers": {
            SERVER_NAME: {
                "type": "http",
                "url": url,
                "headers": { "Authorization": format!("Bearer {token}") }
            }
        }
    })
}

pub fn write_client_config(ctx: &HubContext, server: &McpServer) -> Result<PathBuf, McpError> {
    let path = client_config_path(ctx);
    let text = serde_json::to_string_pretty(&client_config(&server.url(), &server.token))
        .unwrap_or_default();
    std::fs::write(&path, text)?;
    Ok(path)
}

async fn require_token(
    State(expected): State<String>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let authorized = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == format!("Bearer {expected}"));
    if authorized {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub fn start(ctx: HubContext, port: u16, token: String) -> Result<McpServer, McpError> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .map_err(|source| McpError::Bind { port, source })?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let tools_ctx = ctx.clone();
    let auth_token = token.clone();
    let thread = std::thread::Builder::new()
        .name("hub-mcp".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::error!(%error, "falha ao criar o runtime do MCP");
                    return;
                }
            };
            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(listener) => listener,
                    Err(error) => {
                        tracing::error!(%error, "falha ao abrir o socket do MCP");
                        return;
                    }
                };
                let service: StreamableHttpService<HubTools, LocalSessionManager> =
                    StreamableHttpService::new(
                        move || Ok(HubTools::new(tools_ctx.clone())),
                        Default::default(),
                        StreamableHttpServerConfig::default(),
                    );
                let router = axum::Router::new()
                    .nest_service(MCP_PATH, service)
                    .layer(middleware::from_fn_with_state(auth_token, require_token));
                let result = axum::serve(listener, router)
                    .with_graceful_shutdown(async move {
                        let _ = shutdown_rx.await;
                    })
                    .await;
                if let Err(error) = result {
                    tracing::error!(%error, "servidor MCP parou");
                }
            });
        })?;
    tracing::info!(%addr, "servidor MCP iniciado");
    Ok(McpServer {
        addr,
        token,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    })
}

#[cfg(test)]
mod tests;
