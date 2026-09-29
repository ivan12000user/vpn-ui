use axum::{
    body::{Body, to_bytes},
    extract::Request,
    http::header,
    middleware::Next,
    response::Response,
};

const OLD_AUTH_BUTTON: &str = r#"    <a
      id="admin-auth-button"
      class="button"
      href="/api/admin/auth"
      target="_blank"
      rel="noopener"
    >🔐 Управление</a>

"#;

const OLD_AUTH_HINT: &str = "Нажмите «🔐 Управление», ";
const NEW_AUTH_HINT: &str = "Обновите страницу и войдите снова, ";
const FAVICON_LINK: &str = r#"<link rel="icon" type="image/svg+xml" href="/favicon.svg">"#;

pub async fn polish_html(request: Request, next: Next) -> Response {
    let response = next.run(request).await;

    let is_html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"));

    if !is_html {
        return response;
    }

    let (mut parts, body) = response.into_parts();

    let bytes = match to_bytes(body, 2 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Response::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("failed to render HTML"))
                .expect("valid response");
        }
    };

    let Ok(mut html) = String::from_utf8(bytes.to_vec()) else {
        return Response::from_parts(parts, Body::from(bytes));
    };

    html = html.replace(OLD_AUTH_BUTTON, "");
    html = html.replace(OLD_AUTH_HINT, NEW_AUTH_HINT);

    if !html.contains("/favicon.svg") {
        html = html.replacen("</head>", &format!("{FAVICON_LINK}\n</head>"), 1);
    }

    parts.headers.remove(header::CONTENT_LENGTH);

    Response::from_parts(parts, Body::from(html))
}
