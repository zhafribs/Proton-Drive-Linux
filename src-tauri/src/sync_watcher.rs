use crate::sync_fs::{scan_local_tree, should_ignore_name};
use notify::event::ModifyKind;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const DEBOUNCE: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_secs(30);
const SUPPRESSION_TTL: Duration = Duration::from_secs(30);
const SUPPRESSION_MAX: usize = 4096;

pub const LOCAL_CHANGE_EVENT: &str = "sync://local-change";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalChangeEvent {
    pub root: String,
    pub paths: Vec<String>,
    pub kind: String,
    pub source: String,
}

type Fingerprint = (String, i64, i64);
type Snapshot = HashMap<String, Fingerprint>;

pub struct SyncWatcher {
    watcher: Option<RecommendedWatcher>,
    worker: Option<JoinHandle<()>>,
    suppressed: Arc<Mutex<HashMap<String, Instant>>>,
}

impl SyncWatcher {
    pub fn start(app: AppHandle, root: PathBuf) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|e| format!("canonicalize watch root: {e}"))?;
        if !root.is_dir() {
            return Err("Sync root must be a directory".to_string());
        }

        let (tx, rx) = mpsc::channel();
        let mut watcher = RecommendedWatcher::new(tx, Config::default())
            .map_err(|e| format!("create file watcher: {e}"))?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|e| format!("watch sync root: {e}"))?;

        let suppressed = Arc::new(Mutex::new(HashMap::new()));
        let worker_suppressed = Arc::clone(&suppressed);
        let worker_root = root.clone();
        let worker = std::thread::Builder::new()
            .name("sync-watcher".to_string())
            .spawn(move || run_worker(rx, app, worker_root, worker_suppressed))
            .map_err(|e| format!("spawn sync watcher: {e}"))?;

        Ok(Self {
            watcher: Some(watcher),
            worker: Some(worker),
            suppressed,
        })
    }

    pub fn stop(mut self) {
        self.watcher.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    pub fn suppress(&self, relative_path: &str) {
        let now = Instant::now();
        let Ok(mut suppressed) = self.suppressed.lock() else {
            return;
        };
        prune_suppression(&mut suppressed, now);
        while suppressed.len() >= SUPPRESSION_MAX {
            let Some(key) = suppressed.keys().next().cloned() else {
                break;
            };
            suppressed.remove(&key);
        }
        suppressed.insert(relative_path.to_string(), now);
    }
}

fn run_worker(
    rx: mpsc::Receiver<notify::Result<Event>>,
    app: AppHandle,
    root: PathBuf,
    suppressed: Arc<Mutex<HashMap<String, Instant>>>,
) {
    let mut pending: HashMap<String, String> = HashMap::new();
    let mut deadline: Option<Instant> = None;
    let mut last_poll = Instant::now();
    let mut snapshot = scan_snapshot(&root);

    loop {
        let now = Instant::now();
        let next_deadline = deadline.unwrap_or(now + POLL_INTERVAL.min(Duration::from_secs(1)));
        let poll_deadline = last_poll + POLL_INTERVAL;
        let wait_until = next_deadline.min(poll_deadline);
        let timeout = wait_until.saturating_duration_since(now).max(Duration::from_millis(50));

        match rx.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                if let Some(kind) = map_event_kind(&event.kind) {
                    for path in event.paths {
                        let Some(relative) = relative_path(&root, &path) else {
                            continue;
                        };
                        if should_skip(&relative, &suppressed, Instant::now()) {
                            continue;
                        }
                        pending.insert(relative, kind.to_string());
                        deadline = Some(Instant::now() + DEBOUNCE);
                    }
                }
            }
            Ok(Err(error)) => eprintln!("[sync_watcher] watcher error: {error}"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        let now = Instant::now();
        if deadline.is_some_and(|deadline| now >= deadline) && !pending.is_empty() {
            emit_pending(&app, &root, &mut pending, "watcher");
            deadline = None;
        }

        if now.duration_since(last_poll) >= POLL_INTERVAL {
            let current = scan_snapshot(&root);
            let changed = diff_snapshots(&snapshot, &current);
            snapshot = current;
            if !changed.is_empty() {
                for path in changed {
                    if !should_skip(&path, &suppressed, Instant::now()) {
                        pending.insert(path, "modify".to_string());
                    }
                }
                if !pending.is_empty() {
                    emit_pending(&app, &root, &mut pending, "poller");
                }
            }
            last_poll = now;
        }
    }
}

fn emit_pending(
    app: &AppHandle,
    root: &Path,
    pending: &mut HashMap<String, String>,
    source: &str,
) {
    let mut paths: Vec<_> = pending.drain().collect();
    paths.sort_by(|a, b| a.0.cmp(&b.0));
    if paths.is_empty() {
        return;
    }
    let kind = if paths.iter().all(|(_, kind)| kind == &paths[0].1) {
        paths[0].1.clone()
    } else {
        "modify".to_string()
    };
    let event = LocalChangeEvent {
        root: root.to_string_lossy().to_string(),
        paths: paths.into_iter().map(|(path, _)| path).collect(),
        kind,
        source: source.to_string(),
    };
    if let Err(error) = app.emit(LOCAL_CHANGE_EVENT, event) {
        eprintln!("[sync_watcher] emit local change: {error}");
    }
}

fn scan_snapshot(root: &Path) -> Snapshot {
    match scan_local_tree(root) {
        Ok(entries) => entries
            .into_iter()
            .map(|entry| {
                (
                    entry.relative_path,
                    (entry.kind, entry.size, entry.mtime_ns),
                )
            })
            .collect(),
        Err(error) => {
            eprintln!("[sync_watcher] scan failed: {error}");
            HashMap::new()
        }
    }
}

pub fn diff_snapshots(previous: &Snapshot, current: &Snapshot) -> Vec<String> {
    let mut changed = Vec::new();
    for (path, fingerprint) in current {
        if previous.get(path) != Some(fingerprint) {
            changed.push(path.clone());
        }
    }
    for path in previous.keys() {
        if !current.contains_key(path) {
            changed.push(path.clone());
        }
    }
    changed.sort();
    changed.dedup();
    changed
}

fn map_event_kind(kind: &EventKind) -> Option<&'static str> {
    match kind {
        EventKind::Create(_) => Some("create"),
        EventKind::Modify(ModifyKind::Name(_)) => Some("rename"),
        EventKind::Modify(_) => Some("modify"),
        EventKind::Remove(_) => Some("remove"),
        _ => None,
    }
}

fn relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    if relative.as_os_str().is_empty() {
        return None;
    }
    Some(relative.to_string_lossy().replace('\\', "/"))
}

fn should_skip(
    relative: &str,
    suppressed: &Arc<Mutex<HashMap<String, Instant>>>,
    now: Instant,
) -> bool {
    let Some(name) = relative.rsplit('/').next() else {
        return true;
    };
    if should_ignore_name(name) {
        return true;
    }
    let Ok(mut suppressed) = suppressed.lock() else {
        return false;
    };
    prune_suppression(&mut suppressed, now);
    suppressed.contains_key(relative)
}

fn prune_suppression(suppressed: &mut HashMap<String, Instant>, now: Instant) {
    suppressed.retain(|_, at| now.saturating_duration_since(*at) < SUPPRESSION_TTL);
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, RemoveKind};

    #[test]
    fn diff_detects_added_changed_and_removed() {
        let mut previous = HashMap::new();
        previous.insert("same.txt".to_string(), ("file".to_string(), 1, 10));
        previous.insert("gone.txt".to_string(), ("file".to_string(), 1, 10));
        previous.insert("changed.txt".to_string(), ("file".to_string(), 1, 10));

        let mut current = HashMap::new();
        current.insert("same.txt".to_string(), ("file".to_string(), 1, 10));
        current.insert("changed.txt".to_string(), ("file".to_string(), 2, 20));
        current.insert("new.txt".to_string(), ("file".to_string(), 1, 10));

        assert_eq!(
            diff_snapshots(&previous, &current),
            vec!["changed.txt", "gone.txt", "new.txt"]
        );
    }

    #[test]
    fn maps_event_kinds() {
        assert_eq!(map_event_kind(&EventKind::Create(CreateKind::File)), Some("create"));
        assert_eq!(map_event_kind(&EventKind::Remove(RemoveKind::File)), Some("remove"));
        assert_eq!(map_event_kind(&EventKind::Modify(ModifyKind::Any)), Some("modify"));
        assert_eq!(map_event_kind(&EventKind::Access(notify::event::AccessKind::Any)), None);
    }

    #[test]
    fn derives_relative_paths_safely() {
        let root = Path::new("/tmp/root");
        assert_eq!(
            relative_path(root, Path::new("/tmp/root/a/b.txt")),
            Some("a/b.txt".to_string())
        );
        assert_eq!(relative_path(root, Path::new("/tmp/other/a.txt")), None);
        assert_eq!(relative_path(root, root), None);
    }

    #[test]
    fn suppression_expires() {
        let now = Instant::now();
        let mut suppressed = HashMap::new();
        suppressed.insert("a.txt".to_string(), now);
        prune_suppression(&mut suppressed, now + Duration::from_secs(1));
        assert!(suppressed.contains_key("a.txt"));
        prune_suppression(&mut suppressed, now + SUPPRESSION_TTL + Duration::from_secs(1));
        assert!(!suppressed.contains_key("a.txt"));
    }
}
