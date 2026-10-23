use crate::sync_types::{RootRecord, SyncItemRecord};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS roots (
    id TEXT PRIMARY KEY,
    local_path TEXT NOT NULL UNIQUE,
    device_uid TEXT,
    device_name TEXT,
    remote_root_uid TEXT,
    created_at_ns INTEGER NOT NULL,
    updated_at_ns INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS items (
    root_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    local_kind TEXT NOT NULL,
    local_size INTEGER,
    local_mtime_ns INTEGER,
    remote_uid TEXT,
    remote_kind TEXT,
    remote_size INTEGER,
    remote_mtime_ms INTEGER,
    state TEXT NOT NULL,
    last_error TEXT,
    updated_at_ns INTEGER NOT NULL,
    PRIMARY KEY (root_id, relative_path),
    FOREIGN KEY (root_id) REFERENCES roots(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_items_root_state
    ON items(root_id, state);
"#;

/// Local-only metadata store for the sync planner.
///
/// Paths are stored as they appear under the sync root. They are required to
/// map local paths to remote UIDs, and never leave the machine.
pub struct SyncDb {
    conn: Mutex<Connection>,
}

impl SyncDb {
    pub fn open(path: &Path) -> Result<Self, String> {
        ensure_private_parent(path)?;
        let conn = Connection::open(path).map_err(|e| format!("open sync database: {e}"))?;
        let db = Self::configure(conn)?;
        set_private_file_permissions(path)?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, String> {
        Self::configure(Connection::open_in_memory().map_err(|e| e.to_string())?)
    }

    fn configure(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(|e| format!("configure sync database: {e}"))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| format!("migrate sync database: {e}"))?;
        conn.execute_batch("PRAGMA user_version = 1;")
            .map_err(|e| format!("set sync database version: {e}"))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.conn
            .lock()
            .map_err(|e| format!("sync database lock poisoned: {e}"))
    }

    pub fn upsert_root(&self, root: &RootRecord) -> Result<(), String> {
        self.conn()?
            .execute(
                "INSERT INTO roots (
                    id, local_path, device_uid, device_name, remote_root_uid,
                    created_at_ns, updated_at_ns
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                    local_path = excluded.local_path,
                    device_uid = excluded.device_uid,
                    device_name = excluded.device_name,
                    remote_root_uid = excluded.remote_root_uid,
                    updated_at_ns = excluded.updated_at_ns",
                params![
                    root.id,
                    root.local_path,
                    root.device_uid,
                    root.device_name,
                    root.remote_root_uid,
                    root.created_at_ns,
                    root.updated_at_ns,
                ],
            )
            .map_err(|e| format!("upsert sync root: {e}"))?;
        Ok(())
    }

    pub fn get_root(&self, local_path: &str) -> Result<Option<RootRecord>, String> {
        self.conn()?
            .query_row(
                "SELECT id, local_path, device_uid, device_name, remote_root_uid,
                        created_at_ns, updated_at_ns
                 FROM roots WHERE local_path = ?1",
                params![local_path],
                row_to_root,
            )
            .optional()
            .map_err(|e| format!("read sync root: {e}"))
    }

    pub fn load_items(&self, root_id: &str) -> Result<Vec<SyncItemRecord>, String> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT relative_path, local_kind, local_size, local_mtime_ns,
                        remote_uid, remote_kind, remote_size, remote_mtime_ms,
                        state, last_error, updated_at_ns
                 FROM items WHERE root_id = ?1
                 ORDER BY relative_path ASC",
            )
            .map_err(|e| format!("prepare sync item query: {e}"))?;
        let rows = stmt
            .query_map(params![root_id], row_to_item)
            .map_err(|e| format!("query sync items: {e}"))?;
        rows.map(|row| row.map_err(|e| format!("read sync item: {e}")))
            .collect()
    }

    pub fn upsert_item(&self, root_id: &str, item: &SyncItemRecord) -> Result<(), String> {
        self.conn()?
            .execute(
                "INSERT INTO items (
                    root_id, relative_path, local_kind, local_size, local_mtime_ns,
                    remote_uid, remote_kind, remote_size, remote_mtime_ms,
                    state, last_error, updated_at_ns
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(root_id, relative_path) DO UPDATE SET
                    local_kind = excluded.local_kind,
                    local_size = excluded.local_size,
                    local_mtime_ns = excluded.local_mtime_ns,
                    remote_uid = excluded.remote_uid,
                    remote_kind = excluded.remote_kind,
                    remote_size = excluded.remote_size,
                    remote_mtime_ms = excluded.remote_mtime_ms,
                    state = excluded.state,
                    last_error = excluded.last_error,
                    updated_at_ns = excluded.updated_at_ns",
                params![
                    root_id,
                    item.relative_path,
                    item.local_kind,
                    item.local_size,
                    item.local_mtime_ns,
                    item.remote_uid,
                    item.remote_kind,
                    item.remote_size,
                    item.remote_mtime_ms,
                    item.state,
                    item.last_error,
                    item.updated_at_ns,
                ],
            )
            .map_err(|e| format!("upsert sync item: {e}"))?;
        Ok(())
    }

    pub fn delete_item(&self, root_id: &str, relative_path: &str) -> Result<(), String> {
        self.conn()?
            .execute(
                "DELETE FROM items WHERE root_id = ?1 AND relative_path = ?2",
                params![root_id, relative_path],
            )
            .map_err(|e| format!("delete sync item: {e}"))?;
        Ok(())
    }

    pub fn delete_root(&self, root_id: &str) -> Result<(), String> {
        self.conn()?
            .execute("DELETE FROM roots WHERE id = ?1", params![root_id])
            .map_err(|e| format!("delete sync root: {e}"))?;
        Ok(())
    }

    pub fn count_items(&self, root_id: &str) -> Result<i64, String> {
        self.conn()?
            .query_row(
                "SELECT COUNT(*) FROM items WHERE root_id = ?1",
                params![root_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("count sync items: {e}"))
    }
}

fn row_to_root(row: &rusqlite::Row<'_>) -> rusqlite::Result<RootRecord> {
    Ok(RootRecord {
        id: row.get(0)?,
        local_path: row.get(1)?,
        device_uid: row.get(2)?,
        device_name: row.get(3)?,
        remote_root_uid: row.get(4)?,
        created_at_ns: row.get(5)?,
        updated_at_ns: row.get(6)?,
    })
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncItemRecord> {
    Ok(SyncItemRecord {
        relative_path: row.get(0)?,
        local_kind: row.get(1)?,
        local_size: row.get(2)?,
        local_mtime_ns: row.get(3)?,
        remote_uid: row.get(4)?,
        remote_kind: row.get(5)?,
        remote_size: row.get(6)?,
        remote_mtime_ms: row.get(7)?,
        state: row.get(8)?,
        last_error: row.get(9)?,
        updated_at_ns: row.get(10)?,
    })
}

fn ensure_private_parent(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "sync database path has no parent".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("create sync data directory: {e}"))?;
    set_private_dir_permissions(parent)
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("set sync data directory permissions: {e}"))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("set sync database permissions: {e}"))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync_types::{now_unix_ns, STATE_SYNCED};
    use tempfile::TempDir;

    fn sample_root(local_path: &str) -> RootRecord {
        let now = now_unix_ns();
        RootRecord {
            id: "root-1".to_string(),
            local_path: local_path.to_string(),
            device_uid: Some("device-1".to_string()),
            device_name: Some("Linux-PC".to_string()),
            remote_root_uid: Some("remote-root".to_string()),
            created_at_ns: now,
            updated_at_ns: now,
        }
    }

    fn sample_item(path: &str) -> SyncItemRecord {
        SyncItemRecord {
            relative_path: path.to_string(),
            local_kind: "file".to_string(),
            local_size: Some(42),
            local_mtime_ns: Some(1_700_000_000_000_000_000),
            remote_uid: Some(format!("uid-{path}")),
            remote_kind: Some("file".to_string()),
            remote_size: Some(42),
            remote_mtime_ms: Some(1_700_000_000_000),
            state: STATE_SYNCED.to_string(),
            last_error: None,
            updated_at_ns: now_unix_ns(),
        }
    }

    #[test]
    fn roots_round_trip() {
        let db = SyncDb::open_in_memory().unwrap();
        let root = sample_root("/home/test/ProtonDrive");
        db.upsert_root(&root).unwrap();
        assert_eq!(db.get_root(&root.local_path).unwrap(), Some(root));
        assert_eq!(db.get_root("/missing").unwrap(), None);
    }

    #[test]
    fn items_round_trip_sorted() {
        let db = SyncDb::open_in_memory().unwrap();
        let root = sample_root("/home/test/ProtonDrive");
        db.upsert_root(&root).unwrap();
        db.upsert_item(&root.id, &sample_item("z.txt")).unwrap();
        db.upsert_item(&root.id, &sample_item("a/b.txt")).unwrap();
        let paths: Vec<_> = db
            .load_items(&root.id)
            .unwrap()
            .into_iter()
            .map(|item| item.relative_path)
            .collect();
        assert_eq!(paths, vec!["a/b.txt", "z.txt"]);
        assert_eq!(db.count_items(&root.id).unwrap(), 2);
    }

    #[test]
    fn upsert_updates_existing_item() {
        let db = SyncDb::open_in_memory().unwrap();
        let root = sample_root("/home/test/ProtonDrive");
        db.upsert_root(&root).unwrap();
        let mut item = sample_item("file.txt");
        db.upsert_item(&root.id, &item).unwrap();
        item.local_size = Some(99);
        item.state = "local_pending".to_string();
        db.upsert_item(&root.id, &item).unwrap();
        let loaded = db.load_items(&root.id).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].local_size, Some(99));
        assert_eq!(loaded[0].state, "local_pending");
    }

    #[test]
    fn delete_item_removes_only_requested_path() {
        let db = SyncDb::open_in_memory().unwrap();
        let root = sample_root("/home/test/ProtonDrive");
        db.upsert_root(&root).unwrap();
        db.upsert_item(&root.id, &sample_item("one.txt")).unwrap();
        db.upsert_item(&root.id, &sample_item("two.txt")).unwrap();
        db.delete_item(&root.id, "one.txt").unwrap();
        let paths: Vec<_> = db
            .load_items(&root.id)
            .unwrap()
            .into_iter()
            .map(|item| item.relative_path)
            .collect();
        assert_eq!(paths, vec!["two.txt"]);
    }

    #[test]
    fn deleting_root_cascades_items() {
        let db = SyncDb::open_in_memory().unwrap();
        let root = sample_root("/home/test/ProtonDrive");
        db.upsert_root(&root).unwrap();
        db.upsert_item(&root.id, &sample_item("one.txt")).unwrap();
        db.delete_root(&root.id).unwrap();
        assert_eq!(db.count_items(&root.id).unwrap(), 0);
    }

    #[test]
    fn database_file_and_parent_are_private() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("private").join("sync-state.sqlite3");
        let _db = SyncDb::open(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            let dir_mode = std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(file_mode, 0o600);
            assert_eq!(dir_mode, 0o700);
        }
    }

    #[test]
    fn reopening_database_preserves_rows() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("sync-state.sqlite3");
        {
            let db = SyncDb::open(&path).unwrap();
            let root = sample_root("/home/test/ProtonDrive");
            db.upsert_root(&root).unwrap();
            db.upsert_item(&root.id, &sample_item("kept.txt")).unwrap();
        }
        let db = SyncDb::open(&path).unwrap();
        let root = db.get_root("/home/test/ProtonDrive").unwrap().unwrap();
        assert_eq!(db.load_items(&root.id).unwrap().len(), 1);
    }
}
