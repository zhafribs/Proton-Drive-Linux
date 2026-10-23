use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// A local folder mapped to a remote Proton Drive device root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootRecord {
    pub id: String,
    pub local_path: String,
    pub device_uid: Option<String>,
    pub device_name: Option<String>,
    pub remote_root_uid: Option<String>,
    pub created_at_ns: i64,
    pub updated_at_ns: i64,
}

/// One entry discovered by scanning the local sync root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEntry {
    pub relative_path: String,
    pub kind: String,
    pub size: i64,
    pub mtime_ns: i64,
}

/// Last-known state for one path. The sync planner compares this with the
/// current local and remote views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncItemRecord {
    pub relative_path: String,
    pub local_kind: String,
    pub local_size: Option<i64>,
    pub local_mtime_ns: Option<i64>,
    pub remote_uid: Option<String>,
    pub remote_kind: Option<String>,
    pub remote_size: Option<i64>,
    pub remote_mtime_ms: Option<i64>,
    pub state: String,
    pub last_error: Option<String>,
    pub updated_at_ns: i64,
}

pub const STATE_SYNCED: &str = "synced";
pub const STATE_LOCAL_PENDING: &str = "local_pending";
pub const STATE_REMOTE_PENDING: &str = "remote_pending";
pub const STATE_CONFLICT: &str = "conflict";
pub const STATE_ERROR: &str = "error";

pub fn now_unix_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
