use crate::sync_db::SyncDb;
use crate::sync_fs::{scan_local_tree, trash_local_path, validate_relative_path};
use crate::sync_transfer::{
    DownloadStart, TransferManager, UploadStart,
};
use crate::sync_types::{now_unix_ns, LocalEntry, RootRecord, SyncItemRecord};
use crate::sync_watcher::SyncWatcher;
use crate::AppState;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::State;

const ERR_SYNC_NOT_READY: &str = "Sync is not configured";
const ERR_SYNC_STATE: &str = "Sync state is unavailable";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub active: bool,
    pub root_path: Option<String>,
    pub item_count: i64,
    pub last_error: Option<String>,
}

pub struct SyncState {
    db: Mutex<Option<SyncDb>>,
    watcher: Mutex<Option<SyncWatcher>>,
    transfers: TransferManager,
    active_root: Mutex<Option<RootRecord>>,
    fallback_trash_root: PathBuf,
}

impl SyncState {
    pub fn new(app_data_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&app_data_dir)
            .map_err(|e| format!("create app data directory: {e}"))?;
        let db = SyncDb::open(&app_data_dir.join("sync-state.sqlite3"))?;
        let fallback_trash_root = app_data_dir.join("trash");
        fs::create_dir_all(&fallback_trash_root)
            .map_err(|e| format!("create fallback trash directory: {e}"))?;
        Ok(Self {
            db: Mutex::new(Some(db)),
            watcher: Mutex::new(None),
            transfers: TransferManager::default(),
            active_root: Mutex::new(None),
            fallback_trash_root,
        })
    }

    fn require_root(&self) -> Result<RootRecord, String> {
        self.active_root
            .lock()
            .map_err(|e| format!("{ERR_SYNC_STATE}: {e}"))?
            .clone()
            .ok_or_else(|| ERR_SYNC_NOT_READY.to_string())
    }

    fn db(&self) -> Result<std::sync::MutexGuard<'_, Option<SyncDb>>, String> {
        self.db
            .lock()
            .map_err(|e| format!("{ERR_SYNC_STATE}: {e}"))
    }

    fn status(&self) -> Result<SyncStatus, String> {
        let root = self.active_root.lock().map_err(|e| e.to_string())?.clone();
        let active = self
            .watcher
            .lock()
            .map_err(|e| e.to_string())?
            .is_some();
        let item_count = if let Some(root) = &root {
            self.db()?
                .as_ref()
                .map(|db| db.count_items(&root.id))
                .transpose()?
                .unwrap_or(0)
        } else {
            0
        };
        Ok(SyncStatus {
            active,
            root_path: root.map(|root| root.local_path),
            item_count,
            last_error: None,
        })
    }
}

fn require_allowed(window: &tauri::WebviewWindow) -> Result<(), String> {
    crate::ensure_sync_command_allowed(window)
}

#[tauri::command]
pub fn sync_get_root(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<Option<RootRecord>, String> {
    require_allowed(&window)?;
    let active = state
        .sync
        .active_root
        .lock()
        .map_err(|e| e.to_string())?;
    Ok(active.clone())
}

#[tauri::command]
pub fn sync_set_root(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    path: String,
    device_name: String,
) -> Result<RootRecord, String> {
    require_allowed(&window)?;
    let root_path = crate::validate_sync_root_path(&path)?;
    let local_path = root_path.to_string_lossy().to_string();
    let now = now_unix_ns();

    let db_guard = state.sync.db()?;
    let db = db_guard.as_ref().ok_or_else(|| ERR_SYNC_STATE.to_string())?;
    let existing = db.get_root(&local_path)?;
    let root = RootRecord {
        id: existing
            .as_ref()
            .map(|root| root.id.clone())
            .unwrap_or_else(|| format!("root-{now}")),
        local_path,
        device_uid: existing.as_ref().and_then(|root| root.device_uid.clone()),
        device_name: Some(device_name),
        remote_root_uid: existing
            .as_ref()
            .and_then(|root| root.remote_root_uid.clone()),
        created_at_ns: existing
            .as_ref()
            .map(|root| root.created_at_ns)
            .unwrap_or(now),
        updated_at_ns: now,
    };
    db.upsert_root(&root)?;
    drop(db_guard);

    *state
        .sync
        .active_root
        .lock()
        .map_err(|e| e.to_string())? = Some(root.clone());
    Ok(root)
}

#[tauri::command]
pub fn sync_start(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<SyncStatus, String> {
    require_allowed(&window)?;
    let root = state.sync.require_root()?;
    let mut watcher = state.sync.watcher.lock().map_err(|e| e.to_string())?;
    if watcher.is_none() {
        *watcher = Some(SyncWatcher::start(
            app,
            PathBuf::from(&root.local_path),
        )?);
    }
    drop(watcher);
    state.sync.status()
}

#[tauri::command]
pub fn sync_stop(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<SyncStatus, String> {
    require_allowed(&window)?;
    if let Some(watcher) = state
        .sync
        .watcher
        .lock()
        .map_err(|e| e.to_string())?
        .take()
    {
        watcher.stop();
    }
    state.sync.status()
}

#[tauri::command]
pub fn sync_get_status(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<SyncStatus, String> {
    require_allowed(&window)?;
    state.sync.status()
}

#[tauri::command]
pub fn sync_scan_local(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<LocalEntry>, String> {
    require_allowed(&window)?;
    let root = state.sync.require_root()?;
    scan_local_tree(&PathBuf::from(root.local_path))
}

#[tauri::command]
pub fn sync_load_items(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<SyncItemRecord>, String> {
    require_allowed(&window)?;
    let root = state.sync.require_root()?;
    state
        .sync
        .db()?
        .as_ref()
        .ok_or_else(|| ERR_SYNC_STATE.to_string())?
        .load_items(&root.id)
}

#[tauri::command]
pub fn sync_put_item(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    item: SyncItemRecord,
) -> Result<(), String> {
    require_allowed(&window)?;
    let root = state.sync.require_root()?;
    state
        .sync
        .db()?
        .as_ref()
        .ok_or_else(|| ERR_SYNC_STATE.to_string())?
        .upsert_item(&root.id, &item)
}

#[tauri::command]
pub fn sync_delete_item(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    relative_path: String,
) -> Result<(), String> {
    require_allowed(&window)?;
    validate_relative_path(&relative_path)?;
    let root = state.sync.require_root()?;
    state
        .sync
        .db()?
        .as_ref()
        .ok_or_else(|| ERR_SYNC_STATE.to_string())?
        .delete_item(&root.id, &relative_path)
}

#[tauri::command]
pub fn sync_begin_upload(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    relative_path: String,
) -> Result<UploadStart, String> {
    require_allowed(&window)?;
    let root = state.sync.require_root()?;
    state
        .sync
        .transfers
        .begin_upload(&PathBuf::from(root.local_path), &relative_path)
}

#[tauri::command]
pub fn sync_read_upload_chunk(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    transfer_id: u64,
    max_bytes: usize,
) -> Result<Option<String>, String> {
    require_allowed(&window)?;
    state
        .sync
        .transfers
        .read_upload_chunk(transfer_id, max_bytes)
}

#[tauri::command]
pub fn sync_close_upload(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    transfer_id: u64,
) -> Result<(), String> {
    require_allowed(&window)?;
    state.sync.transfers.close_upload(transfer_id)
}

#[tauri::command]
pub fn sync_begin_download(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    relative_path: String,
    mtime_ms: i64,
    expected_size: Option<u64>,
) -> Result<DownloadStart, String> {
    require_allowed(&window)?;
    let root = state.sync.require_root()?;
    state.sync.transfers.begin_download(
        &PathBuf::from(root.local_path),
        &relative_path,
        mtime_ms,
        expected_size,
    )
}

#[tauri::command]
pub fn sync_write_download_chunk(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    transfer_id: u64,
    data_base64: String,
) -> Result<(), String> {
    require_allowed(&window)?;
    state
        .sync
        .transfers
        .write_download_chunk(transfer_id, &data_base64)
}

#[tauri::command]
pub fn sync_finish_download(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    transfer_id: u64,
) -> Result<(), String> {
    require_allowed(&window)?;
    state.sync.transfers.finish_download(transfer_id)
}

#[tauri::command]
pub fn sync_abort_download(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    transfer_id: u64,
) -> Result<(), String> {
    require_allowed(&window)?;
    state.sync.transfers.abort_download(transfer_id)
}

#[tauri::command]
pub fn sync_trash_local(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    relative_path: String,
) -> Result<(), String> {
    require_allowed(&window)?;
    let root = state.sync.require_root()?;
    trash_local_path(
        &PathBuf::from(root.local_path),
        &relative_path,
        &state.sync.fallback_trash_root,
    )
}

#[tauri::command]
pub fn sync_get_device_name(window: tauri::WebviewWindow) -> Result<String, String> {
    require_allowed(&window)?;
    Ok(default_device_name())
}

fn default_device_name() -> String {
    let raw = std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            fs::read_to_string("/etc/hostname")
                .ok()
                .map(|value| value.trim().to_string())
        })
        .unwrap_or_else(|| "Linux-PC".to_string());

    sanitize_device_name(&raw)
}

fn sanitize_device_name(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "Linux-PC".to_string()
    } else {
        trimmed.chars().take(64).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_name_is_sanitized() {
        assert_eq!(sanitize_device_name("dev box"), "dev-box");
        assert_eq!(sanitize_device_name(""), "Linux-PC");
        assert_eq!(sanitize_device_name("..."), "Linux-PC");
        let sanitized = sanitize_device_name("dev box/../../bad");
        assert!(!sanitized.contains('/'));
        assert!(sanitized
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
        assert!(sanitize_device_name(&"a".repeat(100)).len() <= 64);
    }
}
