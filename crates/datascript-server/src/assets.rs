//! Serving the built `web/dist/` front end (embedded via `rust-embed`) or proxying to a Vite
//! dev server when `--dev-proxy` is configured.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use rust_embed::Embed;

use crate::AppState;

/// Built SPA assets from `web/dist/`. Until M4-02 lands, this contains a placeholder `index.html`.
#[derive(Embed)]
#[folder = "../../web/dist/"]
struct WebAssets;

/// Fallback handler for non-`/api/*` routes.
pub(crate) async fn serve_frontend(
    State(state): State<Arc<AppState>>,
    request: Request,
) -> Response {
    if let Some(proxy_base) = &state.dev_proxy {
        return proxy_to_dev(proxy_base, request).await;
    }
    serve_embedded(request.uri().path())
}

fn serve_embedded(path: &str) -> Response {
    let trimmed = path.trim_start_matches('/');
    let key = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };

    if let Some(asset) = WebAssets::get(key) {
        return asset_response(key, asset.data.into_owned());
    }

    // SPA fallback: routes without a file extension (e.g. `/query/1`) serve `index.html`;
    // missing static files with an extension (e.g. `/missing.js`) return 404.
    let has_extension = key
        .rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'));
    if !has_extension && let Some(index) = WebAssets::get("index.html") {
        return asset_response("index.html", index.data.into_owned());
    }

    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn asset_response(path: &str, bytes: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let content_type = HeaderValue::from_str(mime.as_ref())
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    response
}

async fn proxy_to_dev(proxy_base: &str, request: Request) -> Response {
    let path_and_query = request.uri().path_and_query().map_or("/", |pq| pq.as_str());
    let base = proxy_base.trim_end_matches('/');
    let target_url = format!("{base}{path_and_query}");
    let Ok(target_uri) = target_url.parse::<Uri>() else {
        return (StatusCode::BAD_GATEWAY, "invalid dev proxy URL").into_response();
    };

    let (mut parts, body) = request.into_parts();
    parts.uri = target_uri;
    parts.headers.remove(header::HOST);

    let client = Client::builder(TokioExecutor::new()).build_http::<Body>();
    let proxied = Request::from_parts(parts, body);
    match client.request(proxied).await {
        Ok(resp) => {
            let (resp_parts, resp_body) = resp.into_parts();
            let body = Body::new(resp_body.map_err(axum::Error::new));
            Response::from_parts(resp_parts, body)
        }
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            format!("dev proxy error contacting {proxy_base}: {err}"),
        )
            .into_response(),
    }
}
