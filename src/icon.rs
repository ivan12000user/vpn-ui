use axum::{
    Router,
    http::{HeaderValue, header},
    response::Response,
    routing::get,
};

const ICON_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
  <rect width="64" height="64" rx="14" fill="#101418"/>
  <path d="M32 8 52 16v15c0 13-8.7 21.2-20 25C20.7 52.2 12 44 12 31V16L32 8Z" fill="#68a8ff"/>
  <path d="M22 24h8v7h4v-7h8v16h-8v-7h-4v7h-8V24Z" fill="#07111d"/>
</svg>
"#;

pub fn router() -> Router {
    Router::new()
        .route("/favicon.svg", get(icon))
        .route("/favicon.ico", get(icon))
}

async fn icon() -> Response<String> {
    let mut response = Response::new(ICON_SVG.to_string());
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    response
}
