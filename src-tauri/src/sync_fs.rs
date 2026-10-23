use crate::sync_types::LocalEntry;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const ERR_INVALID_PATH: &str = "Invalid sync path";
const ERR_PATH_ESCAPE: &str = "Sync path escapes the sync root";
const ERR_SCAN: &str = "Failed to scan the sync folder";
const ERR_TRASH: &str = "Failed to move the file to trash";

/// Recursively scan the sync root.
///
/// Symlinks and known lock/temp files are skipped. Paths are returned as
/// POSIX-style paths relative to the root and sorted for deterministic tests.
pub fn scan_local_tree(root: &Path) -> Result<Vec<LocalEntry>, String> {
    let root = root.canonicalize().map_err(|e| {
        eprintln!("[sync_fs] canonicalize root {:?}: {e}", root);
        ERR_SCAN.to_string()
    })?;
    if !root.is_dir() {
        return Err("Sync root must be a directory".to_string());
    }

    let mut entries = Vec::new();
    scan_dir(&root, &root, &mut entries)?;
    entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(entries)
}

fn scan_dir(root: &Path, dir: &Path, entries: &mut Vec<LocalEntry>) -> Result<(), String> {
    let mut children: Vec<_> = fs::read_dir(dir)
        .map_err(|e| {
            eprintln!("[sync_fs] read_dir {:?}: {e}", dir);
            ERR_SCAN.to_string()
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            eprintln!("[sync_fs] read entry in {:?}: {e}", dir);
            ERR_SCAN.to_string()
        })?;
    children.sort_by_key(|entry| entry.file_name());

    for entry in children {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            return Err("Sync folder contains a non-UTF-8 name".to_string());
        };
        if should_ignore_name(name) {
            continue;
        }

        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|e| {
            eprintln!("[sync_fs] stat {:?}: {e}", path);
            ERR_SCAN.to_string()
        })?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            continue;
        }

        let relative_path = path
            .strip_prefix(root)
            .map_err(|_| ERR_SCAN.to_string())?
            .to_string_lossy()
            .replace('\\', "/");

        if file_type.is_dir() {
            entries.push(LocalEntry {
                relative_path: relative_path.clone(),
                kind: "dir".to_string(),
                size: 0,
                mtime_ns: system_time_ns(metadata.modified().ok()),
            });
            scan_dir(root, &path, entries)?;
        } else if file_type.is_file() {
            entries.push(LocalEntry {
                relative_path,
                kind: "file".to_string(),
                size: metadata.len().min(i64::MAX as u64) as i64,
                mtime_ns: system_time_ns(metadata.modified().ok()),
            });
        }
    }

    Ok(())
}

fn system_time_ns(time: Option<SystemTime>) -> i64 {
    time.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

pub fn should_ignore_name(name: &str) -> bool {
    name == ".DS_Store"
        || name.starts_with("._")
        || name.starts_with(".proton-part-")
        || name.starts_with(".Trash-")
        || name.starts_with(".~lock.")
        || name.starts_with("~$")
        || name.starts_with(".#")
        || name.ends_with(".swp")
        || name.ends_with(".swx")
        || name.ends_with(".swo")
}

/// Validate a path relative to the sync root.
///
/// Absolute paths, `.`/`..` components, empty paths, and NUL bytes are
/// rejected. Normal path components are allowed.
pub fn validate_relative_path(relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty() || relative.contains('\0') {
        return Err(ERR_INVALID_PATH.to_string());
    }
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(ERR_INVALID_PATH.to_string());
    }
    let mut components = path.components();
    let mut saw_normal = false;
    for component in components.by_ref() {
        match component {
            Component::Normal(_) => saw_normal = true,
            Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ERR_INVALID_PATH.to_string())
            }
        }
    }
    if !saw_normal {
        return Err(ERR_INVALID_PATH.to_string());
    }
    Ok(path.to_path_buf())
}

/// Validate that a target path is inside the canonical sync root, and that no
/// existing ancestor is a symlink outside the root.
pub fn validate_path_within_root(root: &Path, target: &Path) -> Result<(), String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|_| ERR_PATH_ESCAPE.to_string())?;

    let target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        canonical_root.join(target)
    };

    let mut existing = target.as_path();
    loop {
        if existing.exists() || existing.symlink_metadata().is_ok() {
            break;
        }
        existing = existing.parent().ok_or_else(|| ERR_PATH_ESCAPE.to_string())?;
    }

    if let Ok(metadata) = existing.symlink_metadata() {
        if metadata.file_type().is_symlink() {
            return Err(ERR_PATH_ESCAPE.to_string());
        }
    }

    let canonical_existing = existing
        .canonicalize()
        .map_err(|_| ERR_PATH_ESCAPE.to_string())?;
    if !canonical_existing.starts_with(&canonical_root) {
        return Err(ERR_PATH_ESCAPE.to_string());
    }
    Ok(())
}

/// Move a path to the desktop trash, falling back to an app-managed trash
/// directory if the platform trash is unavailable.
pub fn trash_local_path(
    root: &Path,
    relative: &str,
    fallback_trash_root: &Path,
) -> Result<(), String> {
    let relative = validate_relative_path(relative)?;
    let target = root.join(&relative);
    validate_path_within_root(root, &target)?;
    if !target.exists() {
        return Ok(());
    }

    match trash::delete(&target) {
        Ok(()) => Ok(()),
        Err(error) => {
            eprintln!("[sync_fs] desktop trash failed for {:?}: {error}", target);
            move_to_fallback_trash(&target, &relative, fallback_trash_root).map_err(|e| {
                eprintln!("[sync_fs] fallback trash failed: {e}");
                ERR_TRASH.to_string()
            })
        }
    }
}

fn move_to_fallback_trash(
    target: &Path,
    relative: &Path,
    fallback_trash_root: &Path,
) -> Result<(), String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let destination = fallback_trash_root.join(stamp.to_string()).join(relative);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    match fs::rename(target, &destination) {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            if target.is_file() {
                fs::copy(target, &destination).map_err(|e| e.to_string())?;
                fs::remove_file(target).map_err(|e| e.to_string())?;
                Ok(())
            } else {
                Err(rename_error.to_string())
            }
        }
    }
}

pub fn create_local_dir(root: &Path, relative: &str) -> Result<(), String> {
    let relative = validate_relative_path(relative)?;
    let target = root.join(relative);
    validate_path_within_root(root, &target)?;
    if target.exists() {
        if target.is_dir() {
            return Ok(());
        }
        return Err("A file already exists at the directory path".to_string());
    }
    fs::create_dir_all(&target).map_err(|e| format!("create local directory: {e}"))
}

pub fn set_mtime_ms(path: &Path, mtime_ms: i64) -> Result<(), String> {
    let seconds = mtime_ms.div_euclid(1000);
    let nanos = (mtime_ms.rem_euclid(1000) * 1_000_000) as u32;
    filetime::set_file_mtime(path, filetime::FileTime::from_unix_time(seconds, nanos))
        .map_err(|e| format!("set file mtime: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn scans_nested_files_and_directories() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("root.txt"), b"root").unwrap();
        fs::write(root.join("a/b/file.txt"), b"nested").unwrap();

        let entries = scan_local_tree(root).unwrap();
        let by_path: Vec<_> = entries
            .iter()
            .map(|entry| (entry.relative_path.as_str(), entry.kind.as_str(), entry.size))
            .collect();
        assert_eq!(
            by_path,
            vec![
                ("a", "dir", 0),
                ("a/b", "dir", 0),
                ("a/b/file.txt", "file", 6),
                ("root.txt", "file", 4),
            ]
        );
    }

    #[test]
    fn skips_lock_temp_and_symlink_entries() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::write(root.join("keep.txt"), b"keep").unwrap();
        fs::write(root.join(".DS_Store"), b"x").unwrap();
        fs::write(root.join(".proton-part-123"), b"x").unwrap();
        fs::write(root.join("~$document.docx"), b"x").unwrap();
        fs::write(root.join(".file.swp"), b"x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("keep.txt"), root.join("link.txt")).unwrap();

        let entries = scan_local_tree(root).unwrap();
        let paths: Vec<_> = entries
            .into_iter()
            .map(|entry| entry.relative_path)
            .collect();
        assert_eq!(paths, vec!["keep.txt"]);
    }

    #[test]
    fn validates_normal_relative_paths() {
        assert_eq!(
            validate_relative_path("nested/file.txt").unwrap(),
            PathBuf::from("nested/file.txt")
        );
        assert!(validate_relative_path("").is_err());
        assert!(validate_relative_path("../file.txt").is_err());
        assert!(validate_relative_path("/tmp/file.txt").is_err());
        assert!(validate_relative_path("./file.txt").is_err());
        assert!(validate_relative_path("a/../b").is_err());
        assert!(validate_relative_path("a\0b").is_err());
    }

    #[test]
    fn rejects_paths_outside_root() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), b"x").unwrap();
        assert!(validate_path_within_root(&root, &root.join("ok.txt")).is_ok());
        assert!(validate_path_within_root(&root, &outside.join("secret.txt")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_targets_and_symlink_ancestors() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link-dir")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("link-file")).unwrap();

        assert!(validate_path_within_root(&root, &root.join("link-dir/secret.txt")).is_err());
        assert!(validate_path_within_root(&root, &root.join("link-file")).is_err());
    }

    #[test]
    fn fallback_trash_preserves_file_contents() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("root");
        let fallback = temp.path().join("trash");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("gone.txt"), b"data").unwrap();

        let relative = validate_relative_path("gone.txt").unwrap();
        move_to_fallback_trash(&root.join("gone.txt"), &relative, &fallback).unwrap();
        assert!(!root.join("gone.txt").exists());

        let trashed: Vec<_> = walk_files(&fallback);
        assert_eq!(trashed.len(), 1);
        assert_eq!(fs::read(&trashed[0]).unwrap(), b"data");
    }

    #[test]
    fn creates_local_directories_safely() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        create_local_dir(root, "a/b").unwrap();
        assert!(root.join("a/b").is_dir());
        assert!(create_local_dir(root, "../escape").is_err());
    }

    #[test]
    fn mtime_round_trip_is_close() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("file.txt");
        fs::write(&file, b"x").unwrap();
        set_mtime_ms(&file, 1_700_000_000_123).unwrap();
        let mtime = fs::metadata(&file).unwrap().modified().unwrap();
        let ms = mtime
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!((ms - 1_700_000_000_123).abs() < 2_000);
    }

    fn walk_files(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.extend(walk_files(&path));
            } else {
                files.push(path);
            }
        }
        files
    }
}
