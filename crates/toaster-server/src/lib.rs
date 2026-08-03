//! Axum MJPEG preview server backed by latest-value watch channels.

/// Browser routes and multipart MJPEG response formatting.
pub mod routes;

use anyhow::{Context, Result};
use serde::Serialize;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::watch;

/// Binds and serves an empty preview publisher until the server stops.
pub async fn serve(host: &str, port: u16) -> Result<()> {
    serve_with_publisher(host, port, FramePublisher::new()).await
}

/// Binds and serves a caller-provided latest-frame publisher.
pub async fn serve_with_publisher(host: &str, port: u16, publisher: FramePublisher) -> Result<()> {
    let listener = bind(host, port).await?;
    serve_listener(listener, publisher).await
}

/// Binds the TCP listener separately so callers can fail before GPU setup.
pub async fn bind(host: &str, port: u16) -> Result<TcpListener> {
    TcpListener::bind((host, port))
        .await
        .with_context(|| format!("failed to bind preview server to {host}:{port}"))
}

/// Serves preview routes on an already-bound listener until shutdown or failure.
pub async fn serve_listener(listener: TcpListener, publisher: FramePublisher) -> Result<()> {
    let address = listener.local_addr()?;
    tracing::info!(%address, "preview server listening");

    axum::serve(listener, routes::router(publisher))
        .await
        .context("preview server stopped unexpectedly")
}

#[derive(Clone, Debug, PartialEq, Serialize)]
/// JSON-serializable status for the latest published render update.
pub struct PreviewStatus {
    /// Monotonic render-frame index.
    pub frame_index: u32,
    /// Evaluated animation time.
    pub animation_time_seconds: f32,
    /// Renderer time through RGBA conversion.
    pub render_time_ms: f64,
    /// Observed publication rate.
    pub effective_fps: f64,
    /// Requested preview rate.
    pub target_fps: u32,
    /// Frame width.
    pub width: u32,
    /// Frame height.
    pub height: u32,
    /// Samples rendered by the most recent update.
    pub samples: u32,
    /// Whether batches are accumulating into a static progressive image.
    pub progressive: bool,
    /// Total samples represented by the current image.
    pub accumulated_samples: u32,
    /// Progressive target, absent for independent frames.
    pub target_samples: Option<u32>,
    /// Whether progressive rendering reached its exact target.
    pub progress_complete: bool,
    /// Sample count planned for the next update.
    pub next_samples: u32,
    /// Maximum path depth.
    pub max_bounces: u32,
    /// Whether sample count adapts to the frame budget.
    pub adaptive_sampling: bool,
    /// Human-readable reason for the last adaptive decision.
    pub sample_adjustment: String,
}

#[derive(Clone)]
/// Cloneable latest-frame and latest-status broadcaster.
///
/// `watch` replacement semantics ensure slow clients never backpressure or
/// accumulate a queue of obsolete frames.
pub struct FramePublisher {
    /// Latest complete JPEG frame.
    frame_sender: watch::Sender<Option<Arc<[u8]>>>,
    /// Latest metadata corresponding to a publication.
    status_sender: watch::Sender<Option<PreviewStatus>>,
}

impl FramePublisher {
    /// Creates empty frame and status channels.
    pub fn new() -> Self {
        let (frame_sender, _) = watch::channel(None);
        let (status_sender, _) = watch::channel(None);
        Self {
            frame_sender,
            status_sender,
        }
    }

    /// Replaces the latest JPEG without changing status.
    pub fn publish_jpeg(&self, jpeg: Arc<[u8]>) {
        self.frame_sender.send_replace(Some(jpeg));
    }

    /// Replaces status and then publishes the corresponding complete JPEG.
    pub fn publish_frame(&self, jpeg: Arc<[u8]>, status: PreviewStatus) {
        self.status_sender.send_replace(Some(status));
        self.publish_jpeg(jpeg);
    }

    /// Subscribes a stream client to the latest JPEG value.
    pub(crate) fn subscribe(&self) -> watch::Receiver<Option<Arc<[u8]>>> {
        self.frame_sender.subscribe()
    }

    /// Clones the most recently published status.
    pub(crate) fn latest_status(&self) -> Option<PreviewStatus> {
        self.status_sender.borrow().clone()
    }
}

impl Default for FramePublisher {
    /// Creates an empty latest-frame publisher.
    fn default() -> Self {
        Self::new()
    }
}
