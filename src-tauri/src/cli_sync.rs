use crate::{ensure_official_command_allowed, find_cli_binary, AppState};
use serde::Serialize;
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::State;

const ENGINE_SCRIPT: &str = include_str!("../../scripts/proton-sync.py");
const DEFAULT_REMOTE: &str = "/my-files/ProtonDrive-Linux";
const ENGINE_LOG: &str = "sync-engine.log";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub running: bool,
    pub health: Option<Value>,
    pub log_path: String,
}

pub struct EngineState {
    engine: Mutex<Option<Child>>,
    login: Mutex<Option<Child>>,
    state_home: PathBuf,
    script_path: PathBuf,
    log_path: PathBuf,
}

impl EngineState {
    pub fn new(app_data_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&app_data_dir)
            .map_err(|e| format!("create app data dir: {e}"))?;
        let state_home = app_data_dir.join("state");
        fs::create_dir_all(&state_home)
            .map_err(|e| format!("create engine state dir: {e}"))?;
        let script_path = app_data_dir.join("proton-sync.py");
        fs::write(&script_path, ENGINE_SCRIPT)
            .map_err(|e| format!("write sync engine: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("chmod sync engine: {e}"))?;
        }
        let log_path = app_data_dir.join(ENGINE_LOG);
        Ok(Self {
            engine: Mutex::new(None),
            login: Mutex::new(None),
            state_home,
            script_path,
            log_path,
        })
    }

    fn engine_health_path(&self) -> PathBuf {
        self.state_home.join("proton-sync").join("health.json")
    }

    fn wake_path(&self) -> PathBuf {
        self.state_home.join("proton-sync").join("wake")
    }

    fn append_log(&self) -> Result<std::fs::File, String> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .map_err(|e| format!("open sync engine log: {e}"))
    }

    fn stop_engine(&self) -> Result<(), String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        let Some(mut child) = guard.take() else {
            return Ok(());
        };
        let pid = child.id();
        let _ = Command::new("kill")
            .args(["-TERM", &format!("-{pid}")])
            .status();
        for _ in 0..20 {
            match child.try_wait() {
                Ok(Some(_)) => return Ok(()),
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }

    fn status(&self) -> Result<EngineStatus, String> {
        let mut guard = self.engine.lock().map_err(|e| e.to_string())?;
        let running = match guard.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(None) => true,
                Ok(Some(_)) | Err(_) => {
                    *guard = None;
                    false
                }
            },
            None => false,
        };
        let health = fs::read_to_string(self.engine_health_path())
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        Ok(EngineStatus {
            running,
            health,
            log_path: self.log_path.to_string_lossy().to_string(),
        })
    }
}

fn expand_home(value: &str) -> PathBuf {
    if value == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(value)
}

fn validate_local_root(value: &str) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Unable to locate home directory")?;
    let requested = expand_home(value);
    if !requested.starts_with(&home) {
        return Err("Sync folder must be inside your home directory".to_string());
    }
    fs::create_dir_all(&requested).map_err(|e| format!("create sync folder: {e}"))?;
    crate::validate_sync_root_path(&requested.to_string_lossy())
}

fn find_python() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("PROTON_DRIVE_PYTHON") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Some(path);
        }
    }
    for name in ["python3", "python"] {
        if let Ok(paths) = std::env::var("PATH") {
            for dir in std::env::split_paths(&paths) {
                let path = dir.join(name);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn ensure_remote_root(cli: &PathBuf, remote: &str) {
    let trimmed = remote.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/my-files" {
        return;
    }
    let Some((parent, name)) = trimmed.rsplit_once('/') else {
        return;
    };
    if parent != "/my-files" || name.is_empty() {
        return;
    }
    // Best-effort. If the folder exists, the CLI reports an error; if the
    // session is missing, the engine health later reports needs_login.
    let _ = Command::new(cli)
        .args(["fs", "create-folder", parent, name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[tauri::command]
pub fn engine_default_root(window: tauri::WebviewWindow) -> Result<String, String> {
    ensure_official_command_allowed(&window)?;
    let home = dirs::home_dir().ok_or("Unable to locate home directory")?;
    Ok(home.join("ProtonDrive-Linux").to_string_lossy().to_string())
}

#[tauri::command]
pub fn cli_login(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    ensure_official_command_allowed(&window)?;
    let cli = find_cli_binary().ok_or("Proton Drive CLI not found")?;
    let log = state.engine.append_log()?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let child = Command::new("setsid")
        .arg(&cli)
        .arg("auth")
        .arg("login")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| format!("start CLI login: {e}"))?;
    *state.engine.login.lock().map_err(|e| e.to_string())? = Some(child);
    Ok(())
}

#[tauri::command]
pub fn engine_start(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
    local_root: String,
    remote_root: Option<String>,
) -> Result<EngineStatus, String> {
    ensure_official_command_allowed(&window)?;
    let cli = find_cli_binary().ok_or("Proton Drive CLI not found")?;
    let python = find_python().ok_or("Python 3 is required to run the sync engine")?;
    let local = validate_local_root(&local_root)?;
    let remote = remote_root
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_REMOTE.to_string());
    if !remote.starts_with('/') {
        return Err("Remote root must be an absolute Proton path".to_string());
    }
    ensure_remote_root(&cli, &remote);

    state.engine.stop_engine()?;

    let log = state.engine.append_log()?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let mut command = Command::new("setsid");
    command
        .arg(&python)
        .arg(&state.engine.script_path)
        .arg("--loop")
        .arg("--local")
        .arg(&local)
        .arg("--remote")
        .arg(&remote)
        .env("PROTON_DRIVE_BIN", &cli)
        .env("XDG_STATE_HOME", &state.engine.state_home)
        .env("PROTON_SYNC_INTERVAL", "60")
        .env("PROTON_SYNC_DELETES", "1")
        .env("PROTON_SYNC_INOTIFY", "1")
        .env("PROTON_SYNC_PUSH", "0")
        .env("PROTON_SYNC_WORKERS", "6")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err));

    let child = command
        .spawn()
        .map_err(|e| format!("start sync engine: {e}"))?;
    *state.engine.engine.lock().map_err(|e| e.to_string())? = Some(child);
    std::thread::sleep(Duration::from_millis(250));
    state.engine.status()
}

#[tauri::command]
pub fn engine_stop(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<EngineStatus, String> {
    ensure_official_command_allowed(&window)?;
    state.engine.stop_engine()?;
    state.engine.status()
}

#[tauri::command]
pub fn engine_sync_now(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    ensure_official_command_allowed(&window)?;
    if let Some(parent) = state.engine.wake_path().parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(state.engine.wake_path(), b"1").map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn engine_status(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<EngineStatus, String> {
    ensure_official_command_allowed(&window)?;
    let mut login = state.engine.login.lock().map_err(|e| e.to_string())?;
    if let Some(child) = login.as_mut() {
        if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
            *login = None;
        }
    }
    state.engine.status()
}

#[tauri::command]
pub fn engine_open_log(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    ensure_official_command_allowed(&window)?;
    Ok(state.engine.log_path.to_string_lossy().to_string())
}
