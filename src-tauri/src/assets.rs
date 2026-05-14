use axum::{
    http::{
        header::{CACHE_CONTROL, CONTENT_TYPE},
        HeaderValue,
    },
    response::{IntoResponse, Response},
};

const CLIENT_HTML: &str = include_str!("../../public/client.html");
const APP_JS: &str = include_str!("../../public/app.js");
const APP_CORE_JS: &str = include_str!("../../public/app-core.js");
const APP_UTILS_JS: &str = include_str!("../../public/app-utils.js");
const I18N_JS: &str = include_str!("../../public/i18n.js");
const STYLES_CSS: &str = include_str!("../../public/styles.css");

pub async fn client_html() -> impl IntoResponse {
    html(CLIENT_HTML)
}

pub async fn app_js() -> impl IntoResponse {
    with_type(APP_JS, "text/javascript; charset=utf-8")
}

pub async fn app_core_js() -> impl IntoResponse {
    with_type(APP_CORE_JS, "text/javascript; charset=utf-8")
}

pub async fn app_utils_js() -> impl IntoResponse {
    with_type(APP_UTILS_JS, "text/javascript; charset=utf-8")
}

pub async fn i18n_js() -> impl IntoResponse {
    with_type(I18N_JS, "text/javascript; charset=utf-8")
}

pub async fn styles_css() -> impl IntoResponse {
    with_type(STYLES_CSS, "text/css; charset=utf-8")
}

fn html(body: &'static str) -> Response {
    with_type(body, "text/html; charset=utf-8")
}

fn with_type(body: &'static str, content_type: &'static str) -> Response {
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("no-store, no-cache, must-revalidate"),
    );
    response
}
