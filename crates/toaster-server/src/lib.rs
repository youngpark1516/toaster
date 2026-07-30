pub mod routes;

use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::watch;

pub async fn serve(host: &str, port: u16) -> Result<()> {
    serve_with_publisher(host, port, FramePublisher::new()).await
}

pub async fn serve_with_publisher(host: &str, port: u16, publisher: FramePublisher) -> Result<()> {
    let listener = bind(host, port).await?;
    serve_listener(listener, publisher).await
}

pub async fn bind(host: &str, port: u16) -> Result<TcpListener> {
    TcpListener::bind((host, port))
        .await
        .with_context(|| format!("failed to bind preview server to {host}:{port}"))
}

pub async fn serve_listener(listener: TcpListener, publisher: FramePublisher) -> Result<()> {
    println!(
        "Preview server listening on http://{}",
        listener.local_addr()?
    );

    axum::serve(listener, routes::router(publisher))
        .await
        .context("preview server stopped unexpectedly")
}

#[derive(Clone)]
pub struct FramePublisher {
    sender: watch::Sender<Option<Arc<[u8]>>>,
}

impl FramePublisher {
    pub fn new() -> Self {
        let (sender, _) = watch::channel(None);
        Self { sender }
    }

    pub fn publish_jpeg(&self, jpeg: Arc<[u8]>) {
        self.sender.send_replace(Some(jpeg));
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<Option<Arc<[u8]>>> {
        self.sender.subscribe()
    }
}

impl Default for FramePublisher {
    fn default() -> Self {
        Self::new()
    }
}
