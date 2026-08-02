pub mod routes;

use anyhow::{Context, Result};
use serde::Serialize;
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PreviewStatus {
    pub frame_index: u32,
    pub animation_time_seconds: f32,
    pub render_time_ms: f64,
    pub effective_fps: f64,
    pub target_fps: u32,
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub next_samples: u32,
    pub max_bounces: u32,
    pub adaptive_sampling: bool,
    pub sample_adjustment: String,
}

#[derive(Clone)]
pub struct FramePublisher {
    frame_sender: watch::Sender<Option<Arc<[u8]>>>,
    status_sender: watch::Sender<Option<PreviewStatus>>,
}

impl FramePublisher {
    pub fn new() -> Self {
        let (frame_sender, _) = watch::channel(None);
        let (status_sender, _) = watch::channel(None);
        Self {
            frame_sender,
            status_sender,
        }
    }

    pub fn publish_jpeg(&self, jpeg: Arc<[u8]>) {
        self.frame_sender.send_replace(Some(jpeg));
    }

    pub fn publish_frame(&self, jpeg: Arc<[u8]>, status: PreviewStatus) {
        self.status_sender.send_replace(Some(status));
        self.publish_jpeg(jpeg);
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<Option<Arc<[u8]>>> {
        self.frame_sender.subscribe()
    }

    pub(crate) fn latest_status(&self) -> Option<PreviewStatus> {
        self.status_sender.borrow().clone()
    }
}

impl Default for FramePublisher {
    fn default() -> Self {
        Self::new()
    }
}
