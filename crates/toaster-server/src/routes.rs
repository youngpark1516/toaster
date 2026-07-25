use axum::{routing::get, Router};

pub fn router() -> Router {
    Router::new().route("/", get(index))
}

async fn index() -> &'static str {
    "toaster preview server\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_router() {
        let _router = router();
    }
}
