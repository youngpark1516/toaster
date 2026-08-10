//! Browser page, health, status, and multipart MJPEG routes.

use crate::{FramePublisher, PreviewStatus};
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{header, HeaderValue},
    response::{Html, Response},
    routing::get,
    Json, Router,
};
use std::{convert::Infallible, sync::Arc};
use tokio_stream::{wrappers::WatchStream, StreamExt};
use tower_http::trace::TraceLayer;

const MJPEG_BOUNDARY: &str = "frame";
const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Toaster Live Preview</title>
  <style>
    body { margin: 1rem; font-family: monospace; background: #111; color: #eee; }
    img { display: block; max-width: 100%; max-height: 80vh; }
  </style>
</head>
<body>
  <img src="/stream" alt="Toaster live preview">
  <pre id="status">Waiting for the first frame...</pre>
  <script>
    const statusElement = document.getElementById("status");
    async function refreshStatus() {
      try {
        const status = await fetch("/status", { cache: "no-store" }).then(response => response.json());
        if (!status) {
          statusElement.textContent = "Waiting for the first frame...";
          return;
        }
        const samples = status.progressive
          ? `${status.samples} spp batch | ${status.accumulated_samples}/${status.target_samples} spp accumulated${status.progress_complete ? " (complete)" : ` → ${status.next_samples}`}`
          : `${status.samples} spp${status.adaptive_sampling ? ` → ${status.next_samples} (${status.sample_adjustment})` : ""}`;
        statusElement.textContent = `frame ${status.frame_index} | animation ${status.animation_time_seconds.toFixed(2)}s | ${status.width}x${status.height} | ${samples} | ${status.max_bounces} bounces | render ${status.render_time_ms.toFixed(1)}ms | ${status.effective_fps.toFixed(2)}/${status.target_fps} fps`;
      } catch (error) {
        statusElement.textContent = `Status unavailable: ${error}`;
      }
    }
    refreshStatus();
    setInterval(refreshStatus, 1000);
  </script>
</body>
</html>
"#;

/// Builds the preview router with shared publisher state and HTTP tracing.
pub fn router(publisher: FramePublisher) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/stream", get(stream))
        .route("/status", get(status))
        .route("/healthz", get(healthz))
        .with_state(publisher)
        .layer(TraceLayer::new_for_http())
}

/// Returns the self-contained browser preview page.
async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// Returns the lightweight health-check body.
async fn healthz() -> &'static str {
    "ok\n"
}

/// Returns the latest status or JSON `null` before the first frame.
async fn status(State(publisher): State<FramePublisher>) -> Json<Option<PreviewStatus>> {
    Json(publisher.latest_status())
}

/// Streams complete latest-value JPEG parts until the client disconnects.
async fn stream(State(publisher): State<FramePublisher>) -> Response<Body> {
    let frames = WatchStream::new(publisher.subscribe())
        .filter_map(|frame| frame.map(|jpeg| Ok::<Bytes, Infallible>(mjpeg_part(&jpeg))));

    let mut response = Response::new(Body::from_stream(frames));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("multipart/x-mixed-replace; boundary=frame"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, no-cache, must-revalidate"),
    );
    response
}

/// Wraps one JPEG in a complete multipart boundary, headers, and trailing CRLF.
fn mjpeg_part(jpeg: &Arc<[u8]>) -> Bytes {
    let header = format!(
        "--{MJPEG_BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        jpeg.len()
    );
    let mut part = Vec::with_capacity(header.len() + jpeg.len() + 2);
    part.extend_from_slice(header.as_bytes());
    part.extend_from_slice(jpeg);
    part.extend_from_slice(b"\r\n");
    Bytes::from(part)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::StreamExt;

    #[test]
    fn builds_router() {
        let _router = router(FramePublisher::new());
    }

    #[test]
    fn index_contains_mjpeg_image() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let page = runtime.block_on(index());

        assert!(page.0.contains(r#"<img src="/stream""#));
        assert!(page.0.contains(r#"fetch("/status""#));
        assert!(page.0.contains("status.accumulated_samples"));
    }

    #[test]
    fn health_check_is_ok() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(runtime.block_on(healthz()), "ok\n");
    }

    #[test]
    fn formats_mjpeg_part() {
        let jpeg: Arc<[u8]> = vec![0xff, 0xd8, 0xff, 0xd9].into();
        let part = mjpeg_part(&jpeg);

        assert_eq!(
            part.as_ref(),
            b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 4\r\n\r\n\
              \xff\xd8\xff\xd9\r\n"
        );
    }

    #[test]
    fn subscribers_receive_the_latest_complete_frame() {
        let publisher = FramePublisher::new();
        publisher.publish_jpeg(vec![1, 2, 3].into());
        publisher.publish_jpeg(vec![4, 5].into());
        let mut frames = WatchStream::new(publisher.subscribe());

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let latest = runtime.block_on(frames.next()).unwrap().unwrap();

        assert_eq!(latest.as_ref(), &[4, 5]);
    }

    #[test]
    fn status_returns_latest_render_metrics() {
        let publisher = FramePublisher::new();
        let expected = PreviewStatus {
            frame_index: 7,
            animation_time_seconds: 0.5,
            render_time_ms: 42.0,
            effective_fps: 11.5,
            target_fps: 12,
            width: 800,
            height: 600,
            samples: 16,
            progressive: true,
            accumulated_samples: 64,
            target_samples: Some(256),
            progress_complete: false,
            next_samples: 12,
            max_bounces: 6,
            adaptive_sampling: true,
            sample_adjustment: "reducing: over frame budget".to_owned(),
            loop_counters: [("hits".to_owned(), 2)].into(),
            session_counters: [("hits".to_owned(), 7)].into(),
        };
        publisher.publish_frame(vec![1, 2, 3].into(), expected.clone());

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let Json(actual) = runtime.block_on(status(State(publisher)));

        assert_eq!(actual, Some(expected));
        let json = serde_json::to_value(actual.unwrap()).unwrap();
        assert_eq!(json["loop_counters"]["hits"], 2);
        assert_eq!(json["session_counters"]["hits"], 7);
    }

    #[test]
    fn completed_progressive_status_and_frame_remain_latest() {
        let publisher = FramePublisher::new();
        let expected = PreviewStatus {
            frame_index: 3,
            animation_time_seconds: 0.0,
            render_time_ms: 5.0,
            effective_fps: 12.0,
            target_fps: 12,
            width: 2,
            height: 2,
            samples: 1,
            progressive: true,
            accumulated_samples: 5,
            target_samples: Some(5),
            progress_complete: true,
            next_samples: 0,
            max_bounces: 2,
            adaptive_sampling: false,
            sample_adjustment: "complete".to_owned(),
            loop_counters: Default::default(),
            session_counters: Default::default(),
        };
        publisher.publish_frame(vec![9, 8, 7].into(), expected.clone());

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let Json(actual) = runtime.block_on(status(State(publisher.clone())));
        let mut frames = WatchStream::new(publisher.subscribe());
        let latest = runtime.block_on(frames.next()).unwrap().unwrap();

        assert_eq!(actual, Some(expected));
        assert_eq!(latest.as_ref(), &[9, 8, 7]);
    }

    #[test]
    fn stream_uses_mjpeg_content_type() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let response = runtime.block_on(stream(State(FramePublisher::new())));

        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "multipart/x-mixed-replace; boundary=frame"
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store, no-cache, must-revalidate"
        );
    }
}
