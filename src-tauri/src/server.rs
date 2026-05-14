use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use async_stream::stream;
use axum::{
    body::{Body, Bytes},
    extract::{connect_info::ConnectInfo, DefaultBodyLimit, Multipart, Path as AxumPath, State},
    http::{
        header::{
            ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
            CONTENT_TYPE, RANGE,
        },
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{IntoResponse, Response, Sse},
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use qrcode::{render::svg, QrCode};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{self, File},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{broadcast, oneshot, Mutex},
};
use tower_http::cors::CorsLayer;
use uuid::Uuid;

use crate::assets;
use crate::localization::tr;
use crate::network::{lan_ipv4_addresses, LanAddress};
const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024 * 1024;
const DOWNLOAD_BUFFER_BYTES: usize = 1024 * 1024;

type AppResult<T> = Result<T, AppError>;

#[derive(Clone)]
struct AppState {
    port: u16,
    data_path: PathBuf,
    client_upload_dir: PathBuf,
    lan_ip: String,
    local_addresses: Arc<HashSet<IpAddr>>,
    lock: Arc<Mutex<()>>,
    events: broadcast::Sender<Vec<PublicItem>>,
    download_events: broadcast::Sender<Vec<DownloadPublicItem>>,
    downloads: Arc<Mutex<HashMap<String, DownloadProgress>>>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub port: u16,
    pub ip: String,
    pub url: String,
    pub qr: String,
    pub addresses: Vec<ShareAddress>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ShareAddress {
    pub name: Option<String>,
    pub ip: String,
    pub url: String,
    pub qr: String,
}

pub struct RunningServer {
    shutdown: Option<oneshot::Sender<()>>,
    download_shutdown: Option<oneshot::Sender<()>>,
    state: AppState,
}

#[derive(Deserialize)]
struct LocalFilePayload {
    paths: Vec<String>,
}

#[derive(Debug)]
struct AppError {
    status: StatusCode,
    message: String,
    headers: HeaderMap,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Item {
    id: String,
    kind: String,
    title: String,
    content: Option<String>,
    mime: Option<String>,
    size: u64,
    source: String,
    storage_path: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PublicItem {
    id: String,
    kind: String,
    title: String,
    content: Option<String>,
    mime: Option<String>,
    size: u64,
    source: String,
    exists: bool,
    created_at: String,
    updated_at: String,
}

#[derive(Deserialize)]
struct TextPayload {
    content: String,
    source: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DownloadPublicItem {
    item_id: String,
    speed_bps: u64,
    active_count: u32,
}

#[derive(Debug)]
struct DownloadProgress {
    active_count: u32,
    bytes_since_tick: u64,
    current_speed_bps: u64,
    last_tick: Instant,
}

struct DownloadSession {
    state: AppState,
    item_id: String,
}

impl Drop for DownloadSession {
    fn drop(&mut self) {
        let state = self.state.clone();
        let item_id = self.item_id.clone();
        tokio::spawn(async move {
            mark_download_finished(&state, &item_id).await;
        });
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let mut headers = self.headers;
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        (
            self.status,
            headers,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

impl AppError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            headers: HeaderMap::new(),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
            headers: HeaderMap::new(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
            headers: HeaderMap::new(),
        }
    }

    fn range_not_satisfiable(file_size: u64) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes */{file_size}")).unwrap(),
        );
        Self {
            status: StatusCode::RANGE_NOT_SATISFIABLE,
            message: tr("range_invalid", &[("file_size", file_size.to_string())]),
            headers,
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
            headers: HeaderMap::new(),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        AppError::internal(value)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        AppError::internal(value)
    }
}

impl From<axum::extract::multipart::MultipartError> for AppError {
    fn from(value: axum::extract::multipart::MultipartError) -> Self {
        AppError::bad_request(value.to_string())
    }
}

impl From<broadcast::error::SendError<Vec<PublicItem>>> for AppError {
    fn from(value: broadcast::error::SendError<Vec<PublicItem>>) -> Self {
        AppError::internal(value)
    }
}

pub async fn start(port: u16) -> Result<ServerInfo, String> {
    let data_dir = app_data_dir().map_err(|error| error.to_string())?;
    let data_path = data_dir.join("items.json");
    let client_upload_dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);

    fs::create_dir_all(&data_dir)
        .await
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&client_upload_dir)
        .await
        .map_err(|error| error.to_string())?;

    let lan_addresses = lan_ipv4_addresses();
    let lan_ip = lan_addresses
        .first()
        .map(|address| address.ip.clone())
        .unwrap_or_else(|| "127.0.0.1".to_string());

    let mut local_addresses = HashSet::from([
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
    ]);
    if let Ok(addresses) = local_ip_address::list_afinet_netifas() {
        for (_, ip) in addresses {
            local_addresses.insert(ip);
        }
    }

    let info = server_info(port, &lan_addresses).map_err(|error| error.to_string())?;
    let (events_sender, _) = broadcast::channel(64);
    let (download_events_sender, _) = broadcast::channel(64);
    let state = AppState {
        port,
        data_path,
        client_upload_dir,
        lan_ip,
        local_addresses: Arc::new(local_addresses),
        lock: Arc::new(Mutex::new(())),
        events: events_sender,
        download_events: download_events_sender,
        downloads: Arc::new(Mutex::new(HashMap::new())),
    };
    let (download_shutdown_tx, mut download_shutdown_rx) = oneshot::channel::<()>();
    let download_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    update_download_speeds(&download_state).await;
                    if let Err(error) = broadcast_download_events(&download_state).await {
                        eprintln!("FileShare download status broadcast failed: {}", error.message);
                    }
                }
                _ = &mut download_shutdown_rx => break,
            }
        }
    });

    let app = Router::new()
        .route("/", get(assets::client_html))
        .route("/client.html", get(assets::client_html))
        .route("/app.js", get(assets::app_js))
        .route("/app-core.js", get(assets::app_core_js))
        .route("/app-utils.js", get(assets::app_utils_js))
        .route("/i18n.js", get(assets::i18n_js))
        .route("/styles.css", get(assets::styles_css))
        .route("/api/items", get(items))
        .route("/api/share-info", get(share_info))
        .route("/api/client-info", get(client_info))
        .route("/api/events", get(sse_events))
        .route("/api/download-events", get(download_events))
        .route("/api/text", post(add_text))
        .route(
            "/api/upload",
            post(upload).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route("/api/local-file", post(add_local_files))
        .route("/api/items/:id/download", get(download))
        .route("/api/items/:id", delete(remove))
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .map_err(|error| bind_error_message(port, error))?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        let server = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        });
        if let Err(error) = server.await {
            eprintln!("FileShare HTTP server failed: {error}");
        }
    });

    SERVER_HANDLE.set(Mutex::new(None)).ok();
    if let Some(handle) = SERVER_HANDLE.get() {
        *handle.lock().await = Some(RunningServer {
            shutdown: Some(shutdown_tx),
            download_shutdown: Some(download_shutdown_tx),
            state,
        });
    }

    Ok(info)
}

static SERVER_HANDLE: std::sync::OnceLock<Mutex<Option<RunningServer>>> =
    std::sync::OnceLock::new();

pub async fn stop() -> Result<(), String> {
    let Some(handle) = SERVER_HANDLE.get() else {
        return Ok(());
    };
    let mut guard = handle.lock().await;
    if let Some(mut running) = guard.take() {
        if let Some(download_shutdown) = running.download_shutdown.take() {
            let _ = download_shutdown.send(());
        }
        if let Some(shutdown) = running.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
    Ok(())
}

pub async fn add_admin_local_files(paths: Vec<PathBuf>) -> Result<usize, String> {
    let Some(handle) = SERVER_HANDLE.get() else {
        return Err(tr("server_not_started", &[]));
    };
    let state = {
        let guard = handle.lock().await;
        guard
            .as_ref()
            .map(|running| running.state.clone())
            .ok_or_else(|| tr("server_not_started", &[]))?
    };

    add_local_file_paths(&state, paths)
        .await
        .map(|items| items.len())
        .map_err(|error| error.message)
}

pub async fn download_filename(id: &str) -> Result<String, String> {
    let path = item_storage_path(id).await?;
    Ok(path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| "download".to_string()))
}

pub async fn copy_item_to_path(id: &str, target_path: &Path) -> Result<(), String> {
    let source_path = item_storage_path(id).await?;
    if source_path == target_path {
        return Ok(());
    }

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|error| error.to_string())?;
    }

    fs::copy(&source_path, target_path)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn item_file_path(id: &str) -> Result<PathBuf, String> {
    item_storage_path(id).await
}

async fn items(State(state): State<AppState>) -> AppResult<Json<Vec<PublicItem>>> {
    Ok(Json(public_items(&read_items(&state).await?)))
}

async fn client_info(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
) -> AppResult<Json<serde_json::Value>> {
    require_local_addr(&state, address.ip())?;
    let info = server_info(
        state.port,
        &[LanAddress {
            name: None,
            ip: state.lan_ip.clone(),
        }],
    )
    .map_err(AppError::internal)?;
    Ok(Json(serde_json::json!(info)))
}

async fn share_info(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    let addresses = lan_ipv4_addresses();
    let share_addresses = if addresses.is_empty() {
        vec![LanAddress {
            name: None,
            ip: state.lan_ip.clone(),
        }]
    } else {
        addresses
    };
    let info = server_info(state.port, &share_addresses).map_err(AppError::internal)?;
    Ok(Json(serde_json::json!(info)))
}

async fn sse_events(
    State(state): State<AppState>,
) -> Sse<
    impl futures_core::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    let initial = public_items(&read_items(&state).await.unwrap_or_default());
    let mut receiver = state.events.subscribe();
    let stream = stream! {
        yield Ok(axum::response::sse::Event::default().json_data(initial).unwrap());
        loop {
            match receiver.recv().await {
                Ok(items) => yield Ok(axum::response::sse::Event::default().json_data(items).unwrap()),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream)
}

async fn download_events(
    State(state): State<AppState>,
) -> Sse<
    impl futures_core::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    let initial = download_snapshot(&state).await;
    let mut receiver = state.download_events.subscribe();
    let stream = stream! {
        yield Ok(axum::response::sse::Event::default().json_data(initial).unwrap());
        loop {
            match receiver.recv().await {
                Ok(items) => yield Ok(axum::response::sse::Event::default().json_data(items).unwrap()),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream)
}

async fn add_text(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Json(payload): Json<TextPayload>,
) -> AppResult<Json<PublicItem>> {
    let source = payload.source.unwrap_or_else(|| "client".to_string());
    if source == "admin" {
        require_local_addr(&state, address.ip())?;
    }
    let content = payload.content.trim().to_string();
    if content.is_empty() {
        return Err(AppError::bad_request(tr("empty_text", &[])));
    }

    let item = Item {
        id: Uuid::new_v4().to_string(),
        kind: "text".to_string(),
        title: text_title(&content),
        content: Some(content.clone()),
        mime: None,
        size: content.as_bytes().len() as u64,
        source,
        storage_path: String::new(),
        created_at: now(),
        updated_at: now(),
    };

    let public = add_items(&state, vec![item]).await?.remove(0);
    Ok(Json(public))
}

async fn upload(
    State(state): State<AppState>,
    ConnectInfo(_address): ConnectInfo<SocketAddr>,
    mut multipart: Multipart,
) -> AppResult<Json<Vec<PublicItem>>> {
    let mut source = "client".to_string();
    let mut items = Vec::new();

    while let Some(field) = multipart.next_field().await? {
        let name = field.name().unwrap_or_default().to_string();
        if name == "source" {
            source = field.text().await.unwrap_or_else(|_| "client".to_string());
            continue;
        }
        if name == "file" {
            let filename = field
                .file_name()
                .map(safe_name)
                .unwrap_or_else(|| "file".to_string());
            let mime = field.content_type().map(|value| value.to_string());
            let storage_path = next_available_path(&state.client_upload_dir, &filename)
                .await
                .map_err(AppError::internal)?;
            let mut file = File::create(&storage_path).await?;
            let mut size = 0_u64;
            let mut field = field;

            while let Some(chunk) = field.chunk().await? {
                size += chunk.len() as u64;
                file.write_all(&chunk).await?;
            }

            if size == 0 {
                let _ = fs::remove_file(&storage_path).await;
                continue;
            }

            items.push(Item {
                id: Uuid::new_v4().to_string(),
                kind: "file".to_string(),
                title: filename,
                content: None,
                mime,
                size,
                source: source.clone(),
                storage_path: storage_path.to_string_lossy().to_string(),
                created_at: now(),
                updated_at: now(),
            });
        }
    }

    if source == "admin" {
        for item in &items {
            let _ = fs::remove_file(&item.storage_path).await;
        }
        return Err(AppError::bad_request(tr("admin_picker_required", &[])));
    }
    if items.is_empty() {
        return Err(AppError::bad_request(tr("please_select_file", &[])));
    }

    Ok(Json(add_items(&state, items).await?))
}

async fn add_local_files(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Json(payload): Json<LocalFilePayload>,
) -> AppResult<Json<Vec<PublicItem>>> {
    require_local_addr(&state, address.ip())?;
    let paths = payload.paths.into_iter().map(PathBuf::from).collect();
    Ok(Json(add_local_file_paths(&state, paths).await?))
}

async fn add_local_file_paths(
    state: &AppState,
    paths: Vec<PathBuf>,
) -> AppResult<Vec<PublicItem>> {
    if paths.is_empty() {
        return Err(AppError::bad_request(tr("please_select_file", &[])));
    }

    let mut items = Vec::new();
    for path in paths {
        let metadata = fs::metadata(&path)
            .await
            .map_err(|_| AppError::bad_request(tr("selected_file_missing", &[])))?;
        if !metadata.is_file() {
            return Err(AppError::bad_request(tr("only_file", &[])));
        }
        let title = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| value.to_string())
            .unwrap_or_else(|| "file".to_string());
        items.push(Item {
            id: Uuid::new_v4().to_string(),
            kind: "file".to_string(),
            title,
            content: None,
            mime: mime_guess::from_path(&path)
                .first_raw()
                .map(|value| value.to_string()),
            size: metadata.len(),
            source: "admin".to_string(),
            storage_path: path.to_string_lossy().to_string(),
            created_at: now(),
            updated_at: now(),
        });
    }

    add_items(state, items).await
}

async fn download(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let items = read_items(&state).await?;
    let item = items
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| AppError::not_found(tr("item_missing", &[])))?;

    if item.kind == "text" {
        return Err(AppError::bad_request(tr("text_no_download", &[])));
    }

    let path = PathBuf::from(&item.storage_path);
    let mut file = File::open(&path)
        .await
        .map_err(|_| AppError::not_found(tr("source_missing", &[])))?;
    let metadata = file.metadata().await?;
    let file_size = metadata.len();
    let fallback_name = ascii_fallback_name(&item.title);
    let range = parse_range_header(headers.get(RANGE), file_size)?;
    let download_id = item.id.clone();
    mark_download_started(&state, &download_id).await;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(
            item.mime
                .as_deref()
                .unwrap_or("application/octet-stream"),
        )
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response_headers.insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"; filename*=UTF-8''{}",
            fallback_name,
            urlencoding::encode(&item.title)
        ))
        .unwrap(),
    );

    if let Some((start, end)) = range {
        let length = end - start + 1;
        file.seek(std::io::SeekFrom::Start(start)).await?;
        response_headers.insert(
            CONTENT_LENGTH,
            HeaderValue::from_str(&length.to_string()).unwrap(),
        );
        response_headers.insert(
            CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{file_size}")).unwrap(),
        );
        let stream = stream! {
            let _download_session = DownloadSession {
                state: state.clone(),
                item_id: download_id.clone(),
            };
            let mut reader = file.take(length);
            let mut buffer = vec![0u8; DOWNLOAD_BUFFER_BYTES];
            loop {
                let read = match reader.read(&mut buffer).await {
                    Ok(read) => read,
                    Err(error) => {
                        yield Err::<Bytes, std::io::Error>(error);
                        break;
                    }
                };
                if read == 0 {
                    break;
                }
                record_download_bytes(&state, &download_id, read as u64).await;
                yield Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(&buffer[..read]));
            }
        };
        return Ok((
            StatusCode::PARTIAL_CONTENT,
            response_headers,
            Body::from_stream(stream),
        )
            .into_response());
    }

    response_headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&file_size.to_string()).unwrap(),
    );
    let stream = stream! {
        let _download_session = DownloadSession {
            state: state.clone(),
            item_id: download_id.clone(),
        };
        let mut reader = file;
        let mut buffer = vec![0u8; DOWNLOAD_BUFFER_BYTES];
        loop {
            let read = match reader.read(&mut buffer).await {
                Ok(read) => read,
                Err(error) => {
                    yield Err::<Bytes, std::io::Error>(error);
                    break;
                }
            };
            if read == 0 {
                break;
            }
            record_download_bytes(&state, &download_id, read as u64).await;
            yield Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(&buffer[..read]));
        }
    };
    Ok((response_headers, Body::from_stream(stream)).into_response())
}

async fn remove(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    AxumPath(id): AxumPath<String>,
) -> AppResult<Json<serde_json::Value>> {
    require_local_addr(&state, address.ip())?;
    let _guard = state.lock.lock().await;
    let mut items = read_items_unlocked(&state).await?;
    let before = items.len();
    items.retain(|entry| entry.id != id);
    if items.len() == before {
        return Err(AppError::not_found(tr("item_missing", &[])));
    }
    write_items_unlocked(&state, &items).await?;
    state.events.send(public_items(&items))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn add_items(state: &AppState, mut new_items: Vec<Item>) -> AppResult<Vec<PublicItem>> {
    let _guard = state.lock.lock().await;
    let mut items = read_items_unlocked(state).await?;
    let public = public_items(&new_items);
    new_items.append(&mut items);
    write_items_unlocked(state, &new_items).await?;
    state.events.send(public_items(&new_items))?;
    Ok(public)
}

async fn mark_download_started(state: &AppState, item_id: &str) {
    let mut downloads = state.downloads.lock().await;
    let entry = downloads
        .entry(item_id.to_string())
        .or_insert(DownloadProgress {
            active_count: 0,
            bytes_since_tick: 0,
            current_speed_bps: 0,
            last_tick: Instant::now(),
        });
    entry.active_count = entry.active_count.saturating_add(1);
    if entry.active_count == 1 {
        entry.bytes_since_tick = 0;
        entry.current_speed_bps = 0;
        entry.last_tick = Instant::now();
    }
    drop(downloads);
    broadcast_download_events(state).await.ok();
}

async fn record_download_bytes(state: &AppState, item_id: &str, bytes: u64) {
    let mut downloads = state.downloads.lock().await;
    if let Some(entry) = downloads.get_mut(item_id) {
        entry.bytes_since_tick = entry.bytes_since_tick.saturating_add(bytes);
    }
}

async fn mark_download_finished(state: &AppState, item_id: &str) {
    let mut downloads = state.downloads.lock().await;
    if let Some(entry) = downloads.get_mut(item_id) {
        entry.active_count = entry.active_count.saturating_sub(1);
        if entry.active_count == 0 {
            downloads.remove(item_id);
        }
    }
    drop(downloads);
    broadcast_download_events(state).await.ok();
}

async fn broadcast_download_events(state: &AppState) -> AppResult<()> {
    let snapshot = download_snapshot(state).await;
    state
        .download_events
        .send(snapshot)
        .map_err(AppError::internal)?;
    Ok(())
}

async fn download_snapshot(state: &AppState) -> Vec<DownloadPublicItem> {
    let downloads = state.downloads.lock().await;
    downloads
        .iter()
        .filter(|(_, entry)| entry.active_count > 0)
        .map(|(item_id, entry)| DownloadPublicItem {
            item_id: item_id.clone(),
            speed_bps: entry.current_speed_bps,
            active_count: entry.active_count,
        })
        .collect()
}

async fn update_download_speeds(state: &AppState) {
    let mut downloads = state.downloads.lock().await;
    let now = Instant::now();
    downloads.retain(|_, entry| {
        if entry.active_count == 0 {
            return false;
        }
        let elapsed = now.saturating_duration_since(entry.last_tick).as_secs_f64();
        entry.current_speed_bps = if elapsed > 0.0 {
            (entry.bytes_since_tick as f64 / elapsed) as u64
        } else {
            0
        };
        entry.bytes_since_tick = 0;
        entry.last_tick = now;
        true
    });
}

async fn read_items(state: &AppState) -> AppResult<Vec<Item>> {
    let _guard = state.lock.lock().await;
    read_items_unlocked(state).await
}

async fn read_items_unlocked(state: &AppState) -> AppResult<Vec<Item>> {
    if !state.data_path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read_to_string(&state.data_path).await?;
    if data.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str(&data)?)
}

async fn write_items_unlocked(state: &AppState, items: &[Item]) -> AppResult<()> {
    let data = serde_json::to_string_pretty(items)?;
    fs::write(&state.data_path, data).await?;
    Ok(())
}

async fn item_storage_path(id: &str) -> Result<PathBuf, String> {
    let data_dir = app_data_dir().map_err(|error| error.to_string())?;
    let data_path = data_dir.join("items.json");
    let data = if fs::try_exists(&data_path)
        .await
        .map_err(|error| error.to_string())?
    {
        fs::read_to_string(&data_path)
            .await
            .map_err(|error| error.to_string())?
    } else {
        String::new()
    };

    let items: Vec<Item> = if data.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&data).map_err(|error| error.to_string())?
    };

    let item = items
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| tr("item_missing", &[]))?;

    if item.kind == "text" {
        return Err(tr("text_no_download", &[]));
    }

    let path = PathBuf::from(item.storage_path);
    let exists = fs::try_exists(&path)
        .await
        .map_err(|error| error.to_string())?;
    if !exists {
        return Err(tr("source_missing", &[]));
    }

    Ok(path)
}

fn public_items(items: &[Item]) -> Vec<PublicItem> {
    items
        .iter()
        .map(|item| PublicItem {
            id: item.id.clone(),
            kind: item.kind.clone(),
            title: item.title.clone(),
            content: item.content.clone(),
            mime: item.mime.clone(),
            size: item.size,
            source: item.source.clone(),
            exists: item.kind == "text" || Path::new(&item.storage_path).exists(),
            created_at: item.created_at.clone(),
            updated_at: item.updated_at.clone(),
        })
        .collect()
}

fn parse_range_header(
    header: Option<&HeaderValue>,
    file_size: u64,
) -> AppResult<Option<(u64, u64)>> {
    let Some(header) = header else {
        return Ok(None);
    };
    if file_size == 0 {
        return Err(AppError::range_not_satisfiable(file_size));
    }

    let value = header
        .to_str()
        .map_err(|_| AppError::range_not_satisfiable(file_size))?
        .trim();
    let Some(range) = value.strip_prefix("bytes=") else {
        return Err(AppError::range_not_satisfiable(file_size));
    };
    if range.contains(',') {
        return Err(AppError::range_not_satisfiable(file_size));
    }

    let Some((start_raw, end_raw)) = range.split_once('-') else {
        return Err(AppError::range_not_satisfiable(file_size));
    };

    if start_raw.is_empty() {
        let suffix_length = end_raw
            .parse::<u64>()
            .map_err(|_| AppError::range_not_satisfiable(file_size))?;
        if suffix_length == 0 {
            return Err(AppError::range_not_satisfiable(file_size));
        }
        let start = file_size.saturating_sub(suffix_length);
        return Ok(Some((start, file_size - 1)));
    }

    let start = start_raw
        .parse::<u64>()
        .map_err(|_| AppError::range_not_satisfiable(file_size))?;
    if start >= file_size {
        return Err(AppError::range_not_satisfiable(file_size));
    }

    let end = if end_raw.is_empty() {
        file_size - 1
    } else {
        end_raw
            .parse::<u64>()
            .map_err(|_| AppError::range_not_satisfiable(file_size))?
            .min(file_size - 1)
    };
    if end < start {
        return Err(AppError::range_not_satisfiable(file_size));
    }

    Ok(Some((start, end)))
}

fn require_local_addr(state: &AppState, address: IpAddr) -> AppResult<()> {
    if state.local_addresses.contains(&address) {
        return Ok(());
    }

    Err(AppError::forbidden(tr("forbidden", &[])))
}

fn server_info(port: u16, lan_addresses: &[LanAddress]) -> Result<ServerInfo, qrcode::types::QrError> {
    let mut addresses = Vec::new();
    for address in lan_addresses {
        let url = format!("http://{}:{}", address.ip, port);
        let qr = qr_svg(&url)?;
        addresses.push(ShareAddress {
            name: address.name.clone(),
            ip: address.ip.clone(),
            url,
            qr,
        });
    }
    if addresses.is_empty() {
        let url = format!("http://127.0.0.1:{}", port);
        addresses.push(ShareAddress {
            name: Some("Localhost".to_string()),
            ip: "127.0.0.1".to_string(),
            qr: qr_svg(&url)?,
            url,
        });
    }
    let primary = addresses[0].clone();
    Ok(ServerInfo {
        port,
        ip: primary.ip,
        url: primary.url,
        qr: primary.qr,
        addresses,
    })
}

fn qr_svg(url: &str) -> Result<String, qrcode::types::QrError> {
    let qr = QrCode::new(url.as_bytes())?
        .render::<svg::Color>()
        .min_dimensions(128, 128)
        .dark_color(svg::Color("#303133"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(qr)
}

fn app_data_dir() -> Result<PathBuf, std::io::Error> {
    if let Some(base) = dirs::data_local_dir().or_else(dirs::data_dir) {
        return Ok(base.join("FileShare"));
    }

    Ok(std::env::current_dir()?.join("data"))
}

async fn next_available_path(dir: &Path, filename: &str) -> Result<PathBuf, std::io::Error> {
    let original = Path::new(filename);
    let stem = original
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let ext = original
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();

    let mut candidate = dir.join(filename);
    let mut index = 1;
    while fs::try_exists(&candidate).await? {
        candidate = dir.join(format!("{stem} ({index}){ext}"));
        index += 1;
    }
    Ok(candidate)
}

fn safe_name(value: &str) -> String {
    let name = Path::new(value)
        .file_name()
        .and_then(|part| part.to_str())
        .unwrap_or("file");
    let cleaned: String = name
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ' | '(' | ')') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

fn ascii_fallback_name(value: &str) -> String {
    let fallback: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ' | '(' | ')') {
                ch
            } else {
                '_'
            }
        })
        .collect();

    if fallback.trim().is_empty() {
        "download".to_string()
    } else {
        fallback
    }
}

fn text_title(content: &str) -> String {
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let title: String = compact.chars().take(40).collect();
    if title.is_empty() {
        tr("text_snippet", &[])
    } else {
        title
    }
}

fn bind_error_message(port: u16, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AddrInUse {
        tr("port_taken", &[("port", port.to_string())])
    } else {
        tr(
            "port_failed",
            &[("port", port.to_string()), ("error", error.to_string())],
        )
    }
}

fn now() -> String {
    Utc::now().to_rfc3339()
}
