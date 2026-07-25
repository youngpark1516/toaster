pub mod routes;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

pub async fn serve(host: &str, port: u16) -> Result<()> {
    let listener = TcpListener::bind((host, port))
        .await
        .with_context(|| format!("failed to bind preview server to {host}:{port}"))?;

    println!("Preview server listening on http://{host}:{port}");

    axum::serve(listener, routes::router())
        .await
        .context("preview server stopped unexpectedly")
}
