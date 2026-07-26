use crate::FramePublisher;
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{header, HeaderValue},
    response::Response,
    routing::get,
    Router,
};
use std::{convert::Infallible, sync::Arc};
use tokio_stream::{wrappers::WatchStream, StreamExt};

const MJPEG_BOUNDARY: &str = "frame";

pub fn router(publisher: FramePublisher) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/stream", get(stream))
        .with_state(publisher)
}

async fn index() -> &'static str {
    "toaster preview server\n"
}

async fn stream(State(publisher): State<FramePublisher>) -> Response<Body> {
    let frames = WatchStream::new(publisher.subscribe())
        .filter_map(|frame| frame.map(|jpeg| Ok::<Bytes, Infallible>(mjpeg_part(&jpeg))));

    let mut response = Response::new(Body::from_stream(frames));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("multipart/x-mixed-replace; boundary=frame"),
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
    }
}
