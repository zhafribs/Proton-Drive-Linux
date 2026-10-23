use crate::sync_fs::{set_mtime_ms, validate_path_within_root, validate_relative_path};
use base64::Engine as _;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const ERR_TRANSFER_NOT_FOUND: &str = "Sync transfer not found";
const ERR_TRANSFER_KIND: &str = "Sync transfer has the wrong kind";
const ERR_TRANSFER_PATH: &str = "Invalid sync transfer path";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadStart {
    pub transfer_id: u64,
    pub size: u64,
    pub mtime_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStart {
    pub transfer_id: u64,
}

enum Transfer {
    Upload { file: File },
    Download {
        root: PathBuf,
        target: PathBuf,
        temp: PathBuf,
        file: File,
        mtime_ms: i64,
        written: u64,
        expected_size: Option<u64>,
    },
}

pub struct TransferManager {
    next_id: AtomicU64,
    transfers: Mutex<HashMap<u64, Transfer>>,
}

impl Drop for TransferManager {
    fn drop(&mut self) {
        if let Ok(mut transfers) = self.transfers.lock() {
            for (_, transfer) in transfers.drain() {
                if let Transfer::Download { temp, .. } = transfer {
                    let _ = fs::remove_file(temp);
                }
            }
        }
    }
}

impl Default for TransferManager {
    fn default() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            transfers: Mutex::new(HashMap::new()),
        }
    }
}

impl TransferManager {
    fn lock(&self) -> Result<MutexGuard<'_, HashMap<u64, Transfer>>, String> {
        self.transfers
            .lock()
            .map_err(|e| format!("sync transfer lock poisoned: {e}"))
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn begin_upload(
        &self,
        root: &std::path::Path,
        relative: &str,
    ) -> Result<UploadStart, String> {
        let root = canonical_root(root)?;
        let relative_path = validate_relative_path(relative)?;
        let target = root.join(&relative_path);
        validate_path_within_root(&root, &target)?;

        let metadata =
            fs::symlink_metadata(&target).map_err(|e| format!("stat upload source: {e}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ERR_TRANSFER_PATH.to_string());
        }

        let file = File::open(&target).map_err(|e| format!("open upload source: {e}"))?;
        let size = metadata.len();
        let mtime_ms = system_time_ms(metadata.modified().ok());
        let id = self.next_id();
        self.lock()?.insert(id, Transfer::Upload { file });
        Ok(UploadStart {
            transfer_id: id,
            size,
            mtime_ms,
        })
    }

    /// Read the next upload chunk as base64. `None` means EOF and removes the
    /// transfer handle.
    pub fn read_upload_chunk(
        &self,
        id: u64,
        max_bytes: usize,
    ) -> Result<Option<String>, String> {
        if max_bytes == 0 {
            return Err("chunk size must be positive".to_string());
        }
        let limit = max_bytes.min(MAX_CHUNK_BYTES);
        let mut transfers = self.lock()?;
        let chunk = {
            let transfer = transfers
                .get_mut(&id)
                .ok_or_else(|| ERR_TRANSFER_NOT_FOUND.to_string())?;
            match transfer {
                Transfer::Upload { file, .. } => {
                    let mut buffer = vec![0_u8; limit];
                    let read = file
                        .read(&mut buffer)
                        .map_err(|e| format!("read upload chunk: {e}"))?;
                    if read == 0 {
                        None
                    } else {
                        buffer.truncate(read);
                        Some(base64::engine::general_purpose::STANDARD.encode(buffer))
                    }
                }
                Transfer::Download { .. } => return Err(ERR_TRANSFER_KIND.to_string()),
            }
        };
        if chunk.is_none() {
            transfers.remove(&id);
        }
        Ok(chunk)
    }

    pub fn close_upload(&self, id: u64) -> Result<(), String> {
        self.lock()?.remove(&id);
        Ok(())
    }

    pub fn begin_download(
        &self,
        root: &std::path::Path,
        relative: &str,
        mtime_ms: i64,
        expected_size: Option<u64>,
    ) -> Result<DownloadStart, String> {
        let root = canonical_root(root)?;
        let relative_path = validate_relative_path(relative)?;
        let target = root.join(&relative_path);
        validate_path_within_root(&root, &target)?;

        if let Ok(metadata) = fs::symlink_metadata(&target) {
            if metadata.file_type().is_symlink() || metadata.is_dir() {
                return Err(ERR_TRANSFER_PATH.to_string());
            }
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create download parent: {e}"))?;
        }

        let id = self.next_id();
        let temp = target
            .parent()
            .ok_or_else(|| ERR_TRANSFER_PATH.to_string())?
            .join(format!(".proton-part-{id}"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| format!("create download temp file: {e}"))?;
        self.lock()?.insert(
            id,
            Transfer::Download {
                root,
                target,
                temp,
                file,
                mtime_ms,
                written: 0,
                expected_size,
            },
        );
        Ok(DownloadStart { transfer_id: id })
    }

    pub fn write_download_chunk(&self, id: u64, data_base64: &str) -> Result<(), String> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data_base64)
            .map_err(|e| format!("decode download chunk: {e}"))?;
        if bytes.len() > MAX_CHUNK_BYTES {
            return Err("download chunk is too large".to_string());
        }

        let mut transfers = self.lock()?;
        let transfer = transfers
            .get_mut(&id)
            .ok_or_else(|| ERR_TRANSFER_NOT_FOUND.to_string())?;
        match transfer {
            Transfer::Download {
                file,
                written,
                expected_size,
                ..
            } => {
                let next = written.saturating_add(bytes.len() as u64);
                if let Some(expected) = expected_size {
                    if next > *expected {
                        return Err("download exceeded expected size".to_string());
                    }
                }
                file.write_all(&bytes)
                    .map_err(|e| format!("write download chunk: {e}"))?;
                *written = next;
                Ok(())
            }
            Transfer::Upload { .. } => Err(ERR_TRANSFER_KIND.to_string()),
        }
    }

    pub fn finish_download(&self, id: u64) -> Result<(), String> {
        let transfer = self
            .lock()?
            .remove(&id)
            .ok_or_else(|| ERR_TRANSFER_NOT_FOUND.to_string())?;
        let Transfer::Download {
            root,
            target,
            temp,
            file,
            mtime_ms,
            written,
            expected_size,
        } = transfer
        else {
            return Err(ERR_TRANSFER_KIND.to_string());
        };

        if let Some(expected) = expected_size {
            if written != expected {
                let _ = fs::remove_file(&temp);
                return Err(format!(
                    "download size mismatch: expected {expected}, got {written}"
                ));
            }
        }

        file.sync_all()
            .map_err(|e| format!("sync downloaded file: {e}"))?;
        drop(file);
        if let Err(error) = validate_path_within_root(&root, &target) {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
        fs::rename(&temp, &target).map_err(|e| {
            let _ = fs::remove_file(&temp);
            format!("finish downloaded file: {e}")
        })?;
        set_mtime_ms(&target, mtime_ms)?;
        Ok(())
    }

    pub fn abort_download(&self, id: u64) -> Result<(), String> {
        let transfer = self.lock()?.remove(&id);
        if let Some(Transfer::Download { temp, .. }) = transfer {
            let _ = fs::remove_file(temp);
        }
        Ok(())
    }
}

fn canonical_root(root: &std::path::Path) -> Result<PathBuf, String> {
    root.canonicalize()
        .map_err(|e| format!("canonicalize sync root: {e}"))
}

fn system_time_ms(time: Option<SystemTime>) -> i64 {
    time.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use tempfile::TempDir;

    fn read_all_upload(manager: &TransferManager, id: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        loop {
            match manager.read_upload_chunk(id, 3).unwrap() {
                Some(chunk) => bytes.extend(STANDARD.decode(chunk).unwrap()),
                None => break,
            }
        }
        bytes
    }

    fn write_chunks(manager: &TransferManager, id: u64, data: &[u8], chunk_size: usize) {
        for chunk in data.chunks(chunk_size) {
            manager
                .write_download_chunk(id, &STANDARD.encode(chunk))
                .unwrap();
        }
    }

    #[test]
    fn upload_chunks_reassemble_original_bytes() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let data = b"0123456789abcdef";
        fs::write(root.join("file.txt"), data).unwrap();

        let manager = TransferManager::default();
        let start = manager.begin_upload(root, "file.txt").unwrap();
        assert_eq!(start.size, data.len() as u64);
        assert_eq!(read_all_upload(&manager, start.transfer_id), data);
    }

    #[test]
    fn download_chunks_write_and_finish() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let data = b"downloaded bytes";
        let manager = TransferManager::default();
        let start = manager
            .begin_download(root, "nested/file.txt", 1_700_000_000_123, Some(data.len() as u64))
            .unwrap();
        write_chunks(&manager, start.transfer_id, data, 4);
        manager.finish_download(start.transfer_id).unwrap();

        assert_eq!(fs::read(root.join("nested/file.txt")).unwrap(), data);
        assert!(!root.join("nested").read_dir().unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".proton-part-")
        }));
        let mtime_ms = fs::metadata(root.join("nested/file.txt"))
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!((mtime_ms - 1_700_000_000_123).abs() < 2_000);
    }

    #[test]
    fn finish_replaces_existing_target() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::write(root.join("file.txt"), b"old").unwrap();
        let manager = TransferManager::default();
        let start = manager.begin_download(root, "file.txt", 0, Some(3)).unwrap();
        write_chunks(&manager, start.transfer_id, b"new", 3);
        manager.finish_download(start.transfer_id).unwrap();
        assert_eq!(fs::read(root.join("file.txt")).unwrap(), b"new");
    }

    #[test]
    fn abort_leaves_no_temp_file_or_target() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let manager = TransferManager::default();
        let start = manager.begin_download(root, "file.txt", 0, None).unwrap();
        manager
            .write_download_chunk(start.transfer_id, &STANDARD.encode(b"partial"))
            .unwrap();
        manager.abort_download(start.transfer_id).unwrap();
        assert!(!root.join("file.txt").exists());
        assert!(!root.read_dir().unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".proton-part-")
        }));
    }

    #[test]
    fn rejects_path_traversal() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let manager = TransferManager::default();
        assert!(manager.begin_download(root, "../escape.txt", 0, None).is_err());
        assert!(manager.begin_upload(root, "../escape.txt").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_download_target() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&outside, b"secret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link.txt")).unwrap();

        let manager = TransferManager::default();
        assert!(manager.begin_download(&root, "link.txt", 0, None).is_err());
    }

    #[test]
    fn rejects_size_mismatch() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let manager = TransferManager::default();
        let start = manager.begin_download(root, "file.txt", 0, Some(10)).unwrap();
        manager
            .write_download_chunk(start.transfer_id, &STANDARD.encode(b"short"))
            .unwrap();
        assert!(manager.finish_download(start.transfer_id).is_err());
        assert!(!root.join("file.txt").exists());
    }

    #[test]
    fn rejects_unknown_transfer_ids() {
        let manager = TransferManager::default();
        assert!(manager.read_upload_chunk(999, 10).is_err());
        assert!(manager.write_download_chunk(999, "").is_err());
        assert!(manager.finish_download(999).is_err());
    }
}
