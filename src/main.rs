use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderValue, StatusCode, header::ACCESS_CONTROL_ALLOW_ORIGIN},
    response::{IntoResponse, Response},
    routing::get,
};
use mimalloc::MiMalloc;
use regex::Regex;
use reqwest::header;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

static SAFE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^/repos/[\w.-]+/(Breeze|Breeze-plugin-[\w.-]+)/releases/latest$")
        .expect("safe path regex should be valid")
});

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    github_token: String,
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
}

#[derive(Deserialize)]
struct ProxyQuery {
    path: Option<String>,
}

#[derive(Clone)]
struct CacheEntry {
    status: StatusCode,
    body: Value,
    expires_at: Instant,
}

const CACHE_TTL_SECONDS: u64 = 3600;

#[tokio::main]
async fn main() {
    let github_token = match env::var("GITHUB_TOKEN") {
        Ok(token) => token,
        Err(_) => {
            eprintln!("Missing required env: GITHUB_TOKEN");
            std::process::exit(1);
        }
    };

    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3000);

    let client = reqwest::Client::builder()
        .user_agent("Breeze-Proxy")
        .build()
        .expect("failed to create reqwest client");

    let state = AppState {
        client,
        github_token,
        cache: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/proxy", get(handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("github-proxy listening on http://{addr}/proxy?path=...");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");

    axum::serve(listener, app)
        .await
        .expect("failed to run server");
}

async fn handler(State(state): State<AppState>, Query(query): Query<ProxyQuery>) -> Response {
    let Some(path) = query.path else {
        log_cache_event("BYPASS", None, "reason=missing_path");
        return build_json_response(
            StatusCode::FORBIDDEN,
            json!({ "error": "Access Denied" }),
            "BYPASS",
        );
    };

    if !SAFE_PATTERN.is_match(&path) {
        log_cache_event("BYPASS", Some(&path), "reason=path_not_allowed");
        return build_json_response(
            StatusCode::FORBIDDEN,
            json!({ "error": "Access Denied" }),
            "BYPASS",
        );
    }

    if let Some((cached_status, cached_body)) = get_cache(&state, &path).await {
        log_cache_event("HIT", Some(&path), "source=memory");
        return build_json_response(cached_status, cached_body, "HIT");
    }

    let url = format!("https://api.github.com{path}");
    let result = state
        .client
        .get(url)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.github_token),
        )
        .header(header::ACCEPT, "application/vnd.github.v3+json")
        .send()
        .await;

    match result {
        Ok(response) => {
            let upstream_status =
                StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

            let body: Value = match response.json().await {
                Ok(data) => data,
                Err(_) => json!({ "error": "Invalid upstream response" }),
            };

            if upstream_status.is_success() {
                set_cache(&state, &path, upstream_status, body.clone()).await;
                log_cache_event("MISS", Some(&path), "source=upstream cached=true");
            } else {
                log_cache_event("MISS", Some(&path), "source=upstream cached=false");
            }

            build_json_response(upstream_status, body, "MISS")
        }
        Err(_) => {
            log_cache_event("BYPASS", Some(&path), "reason=upstream_request_failed");
            build_json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": "Internal Error" }),
                "BYPASS",
            )
        }
    }
}

async fn get_cache(state: &AppState, path: &str) -> Option<(StatusCode, Value)> {
    let now = Instant::now();
    let mut cache = state.cache.write().await;

    if let Some(entry) = cache.get(path) {
        if entry.expires_at > now {
            return Some((entry.status, entry.body.clone()));
        }
    }

    cache.remove(path);
    None
}

async fn set_cache(state: &AppState, path: &str, status: StatusCode, body: Value) {
    let expires_at = Instant::now() + Duration::from_secs(CACHE_TTL_SECONDS);
    let entry = CacheEntry {
        status,
        body,
        expires_at,
    };

    let mut cache = state.cache.write().await;
    cache.insert(path.to_owned(), entry);
}

fn build_json_response(status: StatusCode, body: Value, cache_status: &'static str) -> Response {
    let mut response = Json(body).into_response();
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    response
        .headers_mut()
        .insert("x-proxy-cache", HeaderValue::from_static(cache_status));
    response
}

fn log_cache_event(cache_status: &str, path: Option<&str>, detail: &str) {
    let path = path.unwrap_or("-");
    println!("[cache:{cache_status}] path={path} {detail}");
}
