use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_stream::stream;
use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath, State},
    http::{
        header::{
            ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
            CONTENT_TYPE, RANGE,
        },
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{sse::KeepAlive, IntoResponse, Response, Sse},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use qrcode::{render::svg, QrCode};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{broadcast, oneshot, watch, Mutex},
};
use uuid::Uuid;
use tower_http::cors::CorsLayer;
#[cfg(target_os = "windows")]
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

use crate::assets;
use crate::downloads::{
    broadcast_download_events, download_snapshot, mark_download_started, record_download_bytes,
    update_download_speeds, DownloadProgress, DownloadPublicItem, DownloadSession,
};
use crate::localization::tr;
use crate::network::{lan_ipv4_addresses, LanAddress};
const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024 * 1024;
const DOWNLOAD_BUFFER_BYTES: usize = 1024 * 1024;

pub(crate) type AppResult<T> = Result<T, AppError>;

#[derive(Clone)]
pub(crate) struct AppState {
    port: u16,
    data_path: PathBuf,
    client_upload_dir: PathBuf,
    upload_temp_dir: Arc<tempfile::TempDir>,
    lan_ip: String,
    lock: Arc<Mutex<()>>,
    events: broadcast::Sender<Vec<PublicItem>>,
    share_stopped: watch::Sender<bool>,
    pub(crate) download_events: broadcast::Sender<Vec<DownloadPublicItem>>,
    pub(crate) downloads: Arc<Mutex<HashMap<String, DownloadProgress>>>,
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

#[derive(Debug)]
pub(crate) struct AppError {
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
}

struct PendingUpload {
    temp_path: PathBuf,
    final_path: Option<PathBuf>,
}

impl PendingUpload {
    fn new(temp_path: PathBuf) -> Self {
        Self {
            temp_path,
            final_path: None,
        }
    }

    fn commit(&mut self) {
        self.temp_path = PathBuf::new();
        self.final_path = None;
    }
}

impl Drop for PendingUpload {
    fn drop(&mut self) {
        if !self.temp_path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.temp_path);
        }
        if let Some(path) = self.final_path.take() {
            let _ = std::fs::remove_file(path);
        }
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

    pub(crate) fn internal(error: impl std::fmt::Display) -> Self {
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
    let upload_temp_dir = create_upload_temp_dir(&data_dir).map_err(|error| error.to_string())?;

    let lan_addresses = lan_ipv4_addresses();
    let lan_ip = lan_addresses
        .first()
        .map(|address| address.ip.clone())
        .unwrap_or_else(|| "127.0.0.1".to_string());

    let info = server_info(port, &lan_addresses).map_err(|error| error.to_string())?;
    let events_sender = metadata_events();
    let (download_events_sender, _) = broadcast::channel(64);
    let state = AppState {
        port,
        data_path,
        client_upload_dir,
        upload_temp_dir,
        lan_ip,
        lock: metadata_lock(),
        events: events_sender,
        share_stopped: watch::channel(false).0,
        download_events: download_events_sender,
        downloads: Arc::new(Mutex::new(HashMap::new())),
    };
    let (download_shutdown_tx, mut download_shutdown_rx) = oneshot::channel::<()>();
    let download_state = state.clone();
    // 下载状态单独按秒聚合后通过 SSE 推给管理端，用于显示正在下载和速率。
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

    let app = client_router(state.clone());

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

fn client_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(assets::client_html))
        .route("/client.html", get(assets::client_html))
        .route("/app.js", get(assets::app_js))
        .route("/app-core.js", get(assets::app_core_js))
        .route("/app-utils.js", get(assets::app_utils_js))
        .route("/i18n.js", get(assets::i18n_js))
        .route("/styles.css", get(assets::styles_css))
        .route("/api/items", get(items))
        .route("/api/share-info", get(share_info))
        .route("/api/events", get(sse_events))
        .route("/api/download-events", get(download_events))
        .route("/api/text", post(add_text))
        .route(
            "/api/upload",
            post(upload).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route("/api/items/:id/download", get(download))
        // Cross-origin access is unrestricted; administrative writes use Tauri IPC.
        .layer(CorsLayer::permissive())
        .with_state(state)
}

static SERVER_HANDLE: std::sync::OnceLock<Mutex<Option<RunningServer>>> =
    std::sync::OnceLock::new();

fn metadata_lock() -> Arc<Mutex<()>> {
    static LOCK: std::sync::OnceLock<Arc<Mutex<()>>> = std::sync::OnceLock::new();
    // Requests draining after a restart still write the same metadata file.
    LOCK.get_or_init(|| Arc::new(Mutex::new(()))).clone()
}

fn metadata_events() -> broadcast::Sender<Vec<PublicItem>> {
    static EVENTS: std::sync::OnceLock<broadcast::Sender<Vec<PublicItem>>> = std::sync::OnceLock::new();
    // A draining upload may finish after sharing restarts. Notify the new clients too.
    EVENTS.get_or_init(|| broadcast::channel(64).0).clone()
}

pub async fn stop() -> Result<(), String> {
    let Some(handle) = SERVER_HANDLE.get() else {
        return Ok(());
    };
    let mut guard = handle.lock().await;
    if let Some(mut running) = guard.take() {
        // Notify active streams before graceful shutdown drains the HTTP responses.
        running.state.share_stopped.send_replace(true);
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

pub async fn add_admin_text(content: String) -> Result<(), String> {
    let state = running_state().await?;
    add_text_item(&state, content, "admin")
        .await
        .map(|_| ())
        .map_err(|error| error.message)
}

pub async fn remove_admin_item(id: String) -> Result<(), String> {
    let state = running_state().await?;
    remove_item(&state, &id)
        .await
        .map_err(|error| error.message)
}

async fn running_state() -> Result<AppState, String> {
    let Some(handle) = SERVER_HANDLE.get() else {
        return Err(tr("server_not_started", &[]));
    };
    let guard = handle.lock().await;
    guard
        .as_ref()
        .map(|running| running.state.clone())
        .ok_or_else(|| tr("server_not_started", &[]))
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
    let mut receiver = state.events.subscribe();
    let mut stopped = state.share_stopped.subscribe();
    let initial = public_items(&read_items(&state).await.unwrap_or_default());
    let stream = stream! {
        let already_stopped = *stopped.borrow();
        if already_stopped {
            yield Ok(share_stopped_event());
            return;
        }
        yield Ok(axum::response::sse::Event::default().json_data(initial).unwrap());
        loop {
            let result = tokio::select! {
                biased;
                _ = stopped.changed() => {
                    yield Ok(share_stopped_event());
                    break;
                }
                result = receiver.recv() => result,
            };
            match result {
                Ok(items) => yield Ok(axum::response::sse::Event::default().json_data(items).unwrap()),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let snapshot = public_items(&read_items(&state).await.unwrap_or_default());
                    yield Ok(axum::response::sse::Event::default().json_data(snapshot).unwrap());
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn download_events(
    State(state): State<AppState>,
) -> Sse<
    impl futures_core::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    let mut receiver = state.download_events.subscribe();
    let mut stopped = state.share_stopped.subscribe();
    let initial = download_snapshot(&state).await;
    let stream = stream! {
        let already_stopped = *stopped.borrow();
        if already_stopped {
            yield Ok(share_stopped_event());
            return;
        }
        yield Ok(axum::response::sse::Event::default().json_data(initial).unwrap());
        loop {
            let result = tokio::select! {
                biased;
                _ = stopped.changed() => {
                    yield Ok(share_stopped_event());
                    break;
                }
                result = receiver.recv() => result,
            };
            match result {
                Ok(items) => yield Ok(axum::response::sse::Event::default().json_data(items).unwrap()),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let snapshot = download_snapshot(&state).await;
                    yield Ok(axum::response::sse::Event::default().json_data(snapshot).unwrap());
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn share_stopped_event() -> axum::response::sse::Event {
    axum::response::sse::Event::default()
        .event("share-stopped")
        .data("{}")
}

async fn add_text(
    State(state): State<AppState>,
    Json(payload): Json<TextPayload>,
) -> AppResult<Json<PublicItem>> {
    add_text_item(&state, payload.content, "client").await
}

async fn add_text_item(
    state: &AppState,
    content: String,
    source: &str,
) -> AppResult<Json<PublicItem>> {
    let content = content.trim().to_string();
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
        source: source.to_string(),
        storage_path: String::new(),
        created_at: now(),
        updated_at: now(),
    };

    let public = add_items(state, vec![item]).await?.remove(0);
    Ok(Json(public))
}

async fn upload(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> AppResult<Json<Vec<PublicItem>>> {
    let mut items = Vec::new();
    let mut pending_uploads = Vec::new();

    while let Some(field) = multipart.next_field().await? {
        let name = field.name().unwrap_or_default().to_string();
        if name == "source" {
            let _ = field.text().await;
            continue;
        }
        if name == "file" {
            let filename = field
                .file_name()
                .map(safe_name)
                .unwrap_or_else(|| "file".to_string());
            let mime = field.content_type().map(|value| value.to_string());
            let upload = create_upload_temp_file(state.upload_temp_dir.path()).await?;
            let mut pending = PendingUpload::new(upload.0);
            // Keep the file binding after the cleanup guard so Windows closes it before cleanup runs.
            let mut file = upload.1;
            let mut size = 0_u64;
            let mut field = field;

            while let Some(chunk) = field.chunk().await? {
                size += chunk.len() as u64;
                file.write_all(&chunk).await?;
            }

            file.flush().await?;
            file.sync_all().await?;
            drop(file);

            if size == 0 {
                continue;
            }

            let storage_path = persist_upload_without_overwrite(
                &pending.temp_path,
                &state.client_upload_dir,
                &filename,
            )
            .await?;
            pending.final_path = Some(storage_path.clone());
            fs::remove_file(&pending.temp_path).await?;
            pending.temp_path = PathBuf::new();
            let stored_name = storage_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(&filename)
                .to_string();

            items.push(Item {
                id: Uuid::new_v4().to_string(),
                kind: "file".to_string(),
                title: stored_name,
                content: None,
                mime,
                size,
                source: "client".to_string(),
                storage_path: storage_path.to_string_lossy().to_string(),
                created_at: now(),
                updated_at: now(),
            });
            pending_uploads.push(pending);
        }
    }

    if items.is_empty() {
        return Err(AppError::bad_request(tr("please_select_file", &[])));
    }

    let public = add_items(&state, items).await?;
    for pending in &mut pending_uploads {
        pending.commit();
    }
    Ok(Json(public))
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
    // 浏览器、播放器和下载器会依赖 Range；这里同时支持完整下载和分段下载。
    let range = parse_range_header(headers.get(RANGE), file_size)?;
    let download_id = item.id.clone();
    mark_download_started(&state, &download_id).await;
    let download_session = DownloadSession::new(state.clone(), download_id.clone());
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
            let _download_session = download_session;
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
        let _download_session = download_session;
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
            // 每个分片写入下载计数，后台任务会把它折算成管理端看到的速率。
            record_download_bytes(&state, &download_id, read as u64).await;
            yield Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(&buffer[..read]));
        }
    };
    Ok((response_headers, Body::from_stream(stream)).into_response())
}

async fn remove_item(state: &AppState, id: &str) -> AppResult<()> {
    let _guard = state.lock.lock().await;
    let mut items = read_items_unlocked(&state).await?;
    let before = items.len();
    items.retain(|entry| entry.id != id);
    if items.len() == before {
        return Err(AppError::not_found(tr("item_missing", &[])));
    }
    write_items_unlocked(&state, &items).await?;
    let _ = state.events.send(public_items(&items));
    Ok(())
}

async fn add_items(state: &AppState, mut new_items: Vec<Item>) -> AppResult<Vec<PublicItem>> {
    let _guard = state.lock.lock().await;
    let mut items = read_items_unlocked(state).await?;
    let public = public_items(&new_items);
    new_items.append(&mut items);
    write_items_unlocked(state, &new_items).await?;
    let _ = state.events.send(public_items(&new_items));
    Ok(public)
}

async fn read_items(state: &AppState) -> AppResult<Vec<Item>> {
    let _guard = state.lock.lock().await;
    read_items_unlocked(state).await
}

async fn read_items_unlocked(state: &AppState) -> AppResult<Vec<Item>> {
    read_items_from_disk(&state.data_path).await
}

async fn write_items_unlocked(state: &AppState, items: &[Item]) -> AppResult<()> {
    let data = serde_json::to_string_pretty(items)?;
    let backup_path = metadata_backup_path(&state.data_path);

    if let Ok(existing) = fs::read_to_string(&state.data_path).await {
        if !existing.trim().is_empty() && serde_json::from_str::<Vec<Item>>(&existing).is_ok() {
            replace_file_contents(&backup_path, existing.as_bytes()).await?;
        }
    }

    replace_file_contents(&state.data_path, data.as_bytes()).await?;
    Ok(())
}

async fn read_items_from_disk(data_path: &Path) -> AppResult<Vec<Item>> {
    match read_items_file(data_path).await {
        Ok(Some(items)) => return Ok(items),
        Ok(None) => {}
        Err(primary_error) => {
            let backup_path = metadata_backup_path(data_path);
            match read_items_file(&backup_path).await {
                Ok(Some(items)) => {
                    let backup_data = fs::read(&backup_path).await?;
                    replace_file_contents(data_path, &backup_data).await?;
                    eprintln!(
                        "FileShare restored metadata from backup after read failure: {}",
                        primary_error.message
                    );
                    return Ok(items);
                }
                _ => return Err(primary_error),
            }
        }
    }

    let backup_path = metadata_backup_path(data_path);
    if let Some(items) = read_items_file(&backup_path).await? {
        let backup_data = fs::read(&backup_path).await?;
        replace_file_contents(data_path, &backup_data).await?;
        return Ok(items);
    }
    Ok(Vec::new())
}

async fn read_items_file(path: &Path) -> AppResult<Option<Vec<Item>>> {
    if !fs::try_exists(path).await? {
        return Ok(None);
    }
    let data = fs::read_to_string(path).await?;
    if data.trim().is_empty() {
        return Err(AppError::internal("metadata file is empty"));
    }
    Ok(Some(serde_json::from_str(&data)?))
}

fn metadata_backup_path(data_path: &Path) -> PathBuf {
    data_path.with_extension("json.bak")
}

async fn replace_file_contents(path: &Path, data: &[u8]) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::internal("metadata path has no parent"))?;
    fs::create_dir_all(parent).await?;

    let temp_path = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("items.json"),
        Uuid::new_v4()
    ));
    let mut temp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .await?;

    let write_result = async {
        temp_file.write_all(data).await?;
        temp_file.flush().await?;
        temp_file.sync_all().await
    }
    .await;
    drop(temp_file);

    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path).await;
        return Err(error.into());
    }

    if let Err(error) = replace_path(&temp_path, path).await {
        let _ = fs::remove_file(&temp_path).await;
        return Err(error.into());
    }
    sync_parent_directory(parent).await?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
async fn replace_path(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    fs::rename(source, target).await
}

#[cfg(target_os = "windows")]
async fn replace_path(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    use std::os::windows::ffi::OsStrExt;

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
async fn sync_parent_directory(parent: &Path) -> Result<(), std::io::Error> {
    File::open(parent).await?.sync_all().await
}

#[cfg(not(unix))]
async fn sync_parent_directory(_parent: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

async fn item_storage_path(id: &str) -> Result<PathBuf, String> {
    let data_dir = app_data_dir().map_err(|error| error.to_string())?;
    let data_path = data_dir.join("items.json");
    let items = read_items_from_disk(&data_path)
        .await
        .map_err(|error| error.message)?;

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
            // 文件可能被用户从原位置移动或删除，列表每次输出时都重新标记可用性。
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

async fn create_upload_temp_file(dir: &Path) -> AppResult<(PathBuf, File)> {
    loop {
        let path = dir.join(format!(".fileshare-{}.part", Uuid::new_v4()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

fn create_upload_temp_dir(data_dir: &Path) -> Result<Arc<tempfile::TempDir>, std::io::Error> {
    let parent = data_dir.join("uploads-in-progress");
    std::fs::create_dir_all(&parent)?;
    // Each server instance owns only its private directory. Never scan Downloads
    // by filename, and keep old requests' directories alive until they finish.
    tempfile::Builder::new()
        .prefix("session-")
        .tempdir_in(parent)
        .map(Arc::new)
}

async fn persist_upload_without_overwrite(
    temp_path: &Path,
    dir: &Path,
    filename: &str,
) -> AppResult<PathBuf> {
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

    let mut index = 1;
    let mut candidate = dir.join(filename);

    loop {
        match fs::hard_link(temp_path, &candidate).await {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                candidate = dir.join(format!("{stem} ({index}){ext}"));
                index += 1;
            }
            Err(_) => match copy_upload_to_new_file(temp_path, &candidate).await {
                Ok(()) => return Ok(candidate),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    candidate = dir.join(format!("{stem} ({index}){ext}"));
                    index += 1;
                }
                Err(error) => return Err(error.into()),
            },
        }
    }
}

async fn copy_upload_to_new_file(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    let mut source_file = File::open(source).await?;
    let mut target_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .await?;

    let result = async {
        tokio::io::copy(&mut source_file, &mut target_file).await?;
        target_file.flush().await?;
        target_file.sync_all().await
    }
    .await;

    if result.is_err() {
        drop(target_file);
        let _ = fs::remove_file(target).await;
    }
    result
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{header::{ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN}, Method, Request};
    use tower::ServiceExt;
    use futures_core::Stream;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("fileshare-{name}-{}", Uuid::new_v4()))
    }

    fn test_item(id: &str) -> Item {
        Item {
            id: id.to_string(),
            kind: "text".to_string(),
            title: "test".to_string(),
            content: Some("test".to_string()),
            mime: None,
            size: 4,
            source: "client".to_string(),
            storage_path: String::new(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn test_state(dir: &Path) -> AppState {
        let (events, _) = broadcast::channel(4);
        let (download_events, _) = broadcast::channel(4);
        AppState {
            port: 5421,
            data_path: dir.join("items.json"),
            client_upload_dir: dir.to_path_buf(),
            upload_temp_dir: create_upload_temp_dir(dir).unwrap(),
            lan_ip: "127.0.0.1".to_string(),
            lock: Arc::new(Mutex::new(())),
            events,
            share_stopped: watch::channel(false).0,
            download_events,
            downloads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[tokio::test]
    async fn stopping_sends_a_terminal_event_and_closes_both_streams() {
        for path in ["/api/events", "/api/download-events"] {
            let state = test_state(&test_dir("stop-event"));
            let response = client_router(state.clone()).oneshot(
                Request::builder().uri(path).body(Body::empty()).unwrap()
            ).await.unwrap();
            let mut stream = response.into_body().into_data_stream();
            let initial = std::future::poll_fn(|cx| std::pin::Pin::new(&mut stream).poll_next(cx))
                .await.unwrap().unwrap();
            assert!(String::from_utf8_lossy(&initial).contains("data: []"));
            state.share_stopped.send_replace(true);
            let remaining = tokio::time::timeout(Duration::from_secs(1), async {
                let mut remaining = Vec::new();
                while let Some(chunk) = std::future::poll_fn(|cx| std::pin::Pin::new(&mut stream).poll_next(cx)).await {
                    remaining.extend_from_slice(&chunk.unwrap());
                }
                remaining
            }).await.expect("stopped SSE must reach EOF");
            let text = String::from_utf8(remaining).unwrap();
            assert_eq!(text.matches("event: share-stopped").count(), 1, "{path}");
        }
    }

    #[tokio::test]
    async fn streams_opened_during_shutdown_receive_the_terminal_event() {
        for path in ["/api/events", "/api/download-events"] {
            let state = test_state(&test_dir("late-stop-event"));
            state.share_stopped.send_replace(true);
            let response = client_router(state).oneshot(
                Request::builder().uri(path).body(Body::empty()).unwrap()
            ).await.unwrap();
            let bytes = tokio::time::timeout(Duration::from_secs(1), axum::body::to_bytes(response.into_body(), 1024))
                .await.expect("late SSE must reach EOF").unwrap();
            assert!(String::from_utf8_lossy(&bytes).contains("event: share-stopped"));
            assert!(!String::from_utf8_lossy(&bytes).contains("data: []"));
        }
    }

    #[tokio::test]
    async fn all_origins_can_read_lists_and_open_event_streams() {
        let dir = test_dir("desktop-cors");
        let app = client_router(test_state(&dir));
        for origin in ["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost", "http://127.0.0.1:1430", "http://localhost:1431", "https://example.com", "null"] {
            for path in ["/api/items", "/api/events", "/api/download-events"] {
                let response = app.clone().oneshot(
                    Request::builder().uri(path).header("origin", origin).body(Body::empty()).unwrap()
                ).await.unwrap();
                assert_eq!(response.status(), StatusCode::OK, "{origin} {path}");
                assert_eq!(response.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
            }
        }
    }

    #[tokio::test]
    async fn cors_allows_preflight_without_restoring_admin_routes() {
        let dir = test_dir("cors-boundary");
        let app = client_router(test_state(&dir));
        for path in ["/api/text", "/api/upload", "/api/local-file", "/api/items/missing"] {
            let response = app.clone().oneshot(
                Request::builder().method(Method::OPTIONS).uri(path)
                    .header("origin", "tauri://localhost")
                    .header("access-control-request-method", "POST")
                    .header("access-control-request-headers", "content-type,x-test")
                    .body(Body::empty()).unwrap()
            ).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
            assert_eq!(response.headers()[ACCESS_CONTROL_ALLOW_METHODS], "*");
            assert_eq!(response.headers()[ACCESS_CONTROL_ALLOW_HEADERS], "*");
        }
        for (method, path) in [(Method::POST, "/api/local-file"), (Method::DELETE, "/api/items/missing")] {
            let response = app.clone().oneshot(
                Request::builder().method(method).uri(path).body(Body::empty()).unwrap()
            ).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }

    #[test]
    fn parses_supported_range_forms() {
        let exact = HeaderValue::from_static("bytes=10-19");
        let open = HeaderValue::from_static("bytes=90-");
        let suffix = HeaderValue::from_static("bytes=-10");

        assert_eq!(
            parse_range_header(Some(&exact), 100).unwrap(),
            Some((10, 19))
        );
        assert_eq!(
            parse_range_header(Some(&open), 100).unwrap(),
            Some((90, 99))
        );
        assert_eq!(
            parse_range_header(Some(&suffix), 100).unwrap(),
            Some((90, 99))
        );
        assert!(parse_range_header(Some(&HeaderValue::from_static("bytes=100-101")), 100).is_err());
        assert!(parse_range_header(Some(&HeaderValue::from_static("bytes=0-1,3-4")), 100).is_err());
    }

    #[tokio::test]
    async fn concurrent_uploads_never_overwrite_same_name() {
        let dir = test_dir("concurrent-upload");
        fs::create_dir_all(&dir).await.unwrap();
        let mut tasks = Vec::new();

        for index in 0..8_u8 {
            let dir = dir.clone();
            tasks.push(tokio::spawn(async move {
                let (temp_path, mut file) = create_upload_temp_file(&dir).await.unwrap();
                let content = vec![index; 32];
                file.write_all(&content).await.unwrap();
                file.flush().await.unwrap();
                drop(file);
                let stored = persist_upload_without_overwrite(&temp_path, &dir, "same.bin")
                    .await
                    .unwrap();
                fs::remove_file(temp_path).await.unwrap();
                (stored, content)
            }));
        }

        let mut stored_paths = std::collections::HashSet::new();
        for task in tasks {
            let (path, expected) = task.await.unwrap();
            assert!(stored_paths.insert(path.clone()));
            assert_eq!(fs::read(path).await.unwrap(), expected);
        }
        assert_eq!(stored_paths.len(), 8);
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn pending_upload_removes_partial_and_uncommitted_files() {
        let dir = test_dir("pending-cleanup");
        fs::create_dir_all(&dir).await.unwrap();
        let temp_path = dir.join(".upload.part");
        let final_path = dir.join("upload.bin");
        fs::write(&temp_path, b"partial").await.unwrap();
        fs::write(&final_path, b"complete").await.unwrap();

        {
            let mut pending = PendingUpload::new(temp_path.clone());
            pending.final_path = Some(final_path.clone());
        }

        assert!(!temp_path.exists());
        assert!(!final_path.exists());
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn session_temp_cleanup_preserves_user_files_and_other_sessions() {
        let dir = test_dir("startup-cleanup");
        fs::create_dir_all(&dir).await.unwrap();
        let user_file = dir.join(".fileshare-user-upload.part");
        fs::write(&user_file, b"keep").await.unwrap();
        let old_session = create_upload_temp_dir(&dir).unwrap();
        let active_upload = old_session.path().join("active.part");
        fs::write(&active_upload, b"still uploading").await.unwrap();
        let new_session = create_upload_temp_dir(&dir).unwrap();
        let new_path = new_session.path().to_path_buf();
        drop(new_session);
        assert!(!new_path.exists());
        assert!(active_upload.exists());
        assert_eq!(fs::read(&user_file).await.unwrap(), b"keep");
        drop(old_session);
        assert!(!active_upload.exists());
        assert!(user_file.exists());
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn metadata_update_succeeds_without_sse_subscribers() {
        let dir = test_dir("no-sse-subscriber");
        fs::create_dir_all(&dir).await.unwrap();
        let state = test_state(&dir);

        let added = add_items(&state, vec![test_item("persisted")])
            .await
            .unwrap();

        assert_eq!(added.len(), 1);
        assert_eq!(read_items(&state).await.unwrap()[0].id, "persisted");
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn admin_sharing_maps_original_paths_and_removal_preserves_files() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        let source = dir.path().join("original.txt");
        fs::write(&source, b"original contents").await.unwrap();
        let added = add_local_file_paths(&state, vec![source.clone()]).await.unwrap();
        assert_eq!(added.len(), 1);
        let stored = read_items(&state).await.unwrap();
        assert_eq!(Path::new(&stored[0].storage_path), source);
        assert_eq!(stored[0].source, "admin");
        remove_item(&state, &added[0].id).await.unwrap();
        assert_eq!(fs::read(&source).await.unwrap(), b"original contents");
        assert!(read_items(&state).await.unwrap().is_empty());
        add_local_file_paths(&state, vec![source.clone()]).await.unwrap();
        fs::remove_file(source).await.unwrap();
        assert!(!public_items(&read_items(&state).await.unwrap())[0].exists);
    }

    #[tokio::test]
    async fn head_and_unpolled_downloads_release_activity() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        let path = dir.path().join("source.txt");
        fs::write(&path, b"download fixture").await.unwrap();
        let mut item = test_item("download");
        item.kind = "file".to_string();
        item.storage_path = path.to_string_lossy().into_owned();
        add_items(&state, vec![item]).await.unwrap();
        for method in [Method::HEAD, Method::GET] {
            for range in [None, Some("bytes=0-3")] {
                let mut request = Request::builder().method(method.clone())
                    .uri("/api/items/download/download");
                if let Some(range) = range { request = request.header("range", range); }
                let response = client_router(state.clone()).oneshot(
                    request.body(Body::empty()).unwrap()
                ).await.unwrap();
                assert!(response.status().is_success());
                drop(response);
                tokio::time::timeout(Duration::from_secs(1), async {
                    while !state.downloads.lock().await.is_empty() {
                        tokio::task::yield_now().await;
                    }
                }).await.expect("discarded downloads must clear activity");
            }
        }
    }

    #[tokio::test]
    async fn overlapping_generations_preserve_metadata_and_notify_new_clients() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = test_state(dir.path());
        let mut new = test_state(dir.path());
        old.lock = metadata_lock();
        new.lock = metadata_lock();
        old.events = metadata_events();
        new.events = metadata_events();
        let mut events = new.events.subscribe();
        let (first, second) = tokio::join!(
            add_items(&old, vec![test_item("old-upload")]),
            add_items(&new, vec![test_item("new-upload")])
        );
        first.unwrap();
        second.unwrap();
        let items = read_items(&new).await.unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|item| item.id == "old-upload"));
        assert!(items.iter().any(|item| item.id == "new-upload"));
        let latest = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = events.recv().await.unwrap();
                if snapshot.len() == 2 { break snapshot; }
            }
        }).await.expect("new clients must see draining uploads");
        assert_eq!(latest.len(), 2);
    }

    #[tokio::test]
    async fn corrupted_metadata_is_restored_from_backup() {
        let dir = test_dir("metadata-recovery");
        fs::create_dir_all(&dir).await.unwrap();
        let data_path = dir.join("items.json");
        let backup_path = metadata_backup_path(&data_path);
        let expected = vec![test_item("restored")];
        let backup_data = serde_json::to_vec_pretty(&expected).unwrap();

        replace_file_contents(&backup_path, &backup_data)
            .await
            .unwrap();
        fs::write(&data_path, b"{broken json").await.unwrap();

        let restored = read_items_from_disk(&data_path).await.unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, "restored");
        let primary: Vec<Item> =
            serde_json::from_slice(&fs::read(&data_path).await.unwrap()).unwrap();
        assert_eq!(primary[0].id, "restored");
        fs::remove_dir_all(dir).await.unwrap();
    }
}
