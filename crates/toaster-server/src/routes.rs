use crate::FramePublisher;
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{header, HeaderValue},
    response::{Html, Response},
    routing::get,
    Router,
};
use std::{convert::Infallible, sync::Arc};
use tokio_stream::{wrappers::WatchStream, StreamExt};

const MJPEG_BOUNDARY: &str = "frame";
const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Toaster Live Preview</title>
</head>
<body>
  <img src="/stream" alt="Toaster live preview">
</body>
</html>
"#;

pub fn router(publisher: FramePublisher) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/stream", get(stream))
        .route("/healthz", get(healthz))
        .with_state(publisher)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn healthz() -> &'static str {
    "ok\n"
}

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
