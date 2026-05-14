use std::time::Instant;

use serde::Serialize;

use crate::server::{AppError, AppResult, AppState};

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DownloadPublicItem {
    pub item_id: String,
    pub speed_bps: u64,
    pub active_count: u32,
}

#[derive(Debug)]
pub struct DownloadProgress {
    pub active_count: u32,
    pub bytes_since_tick: u64,
    pub current_speed_bps: u64,
    pub last_tick: Instant,
}

pub struct DownloadSession {
    state: AppState,
    item_id: String,
}

impl DownloadSession {
    pub fn new(state: AppState, item_id: String) -> Self {
        Self { state, item_id }
    }
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

pub async fn mark_download_started(state: &AppState, item_id: &str) {
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

pub async fn record_download_bytes(state: &AppState, item_id: &str, bytes: u64) {
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

pub async fn broadcast_download_events(state: &AppState) -> AppResult<()> {
    let snapshot = download_snapshot(state).await;
    state
        .download_events
        .send(snapshot)
        .map_err(AppError::internal)?;
    Ok(())
}

pub async fn download_snapshot(state: &AppState) -> Vec<DownloadPublicItem> {
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

pub async fn update_download_speeds(state: &AppState) {
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
