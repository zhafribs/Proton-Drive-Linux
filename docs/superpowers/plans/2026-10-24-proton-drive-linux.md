# Proton Drive for Linux — Implementation Plan

> **Source of truth:** `/home/zhafrisbs/Proton-Drive-Linux-DESIGN.md`
> **Base:** `DonnieDice/protondrive-linux` at commit `0b5dd592de297faddb865872907d045d47ee24e4` (post-`v1.4.4`)
> **New repo:** `Proton-Drive-Linux`
> **Product:** Proton Drive for Linux (Unofficial) — Web Edition
> **License:** AGPL-3.0-or-later

This plan is for an engineer who has not seen the codebase or the design. Read the design document first. Work in order; each task ends with a test and a commit. Do not skip the Phase 0 spike.

The final app has **no CLI dependency**, **no Calendar build**, and **no permanent deletes**.

---

## Task 0 — Import the upstream base

**Files:**
- Create: `/home/zhafrisbs/Proton-Drive-Linux/` (full upstream history)
- Modify: `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `README.md`, `LICENSE`, `NOTICE`
- Remove: GitLab-specific CI, Calendar build/copy blocks, upstream branding assets

**Steps:**

1. Clone the upstream history locally and check out the approved base:
   ```bash
   git clone /tmp/opencode/protondrive-linux /home/zhafrisbs/Proton-Drive-Linux
   cd /home/zhafrisbs/Proton-Drive-Linux
   git checkout -B main 0b5dd592de297faddb865872907d045d47ee24e4
   git remote rename origin upstream
   ```
2. Create the independent GitHub repo only after the local build passes:
   ```bash
   gh repo create zhafribs/Proton-Drive-Linux --public \
     --description "Proton Drive for Linux (Unofficial) — web-based edition" \
     --source=. --remote=origin --push
   ```
   Do not use a GitHub fork. The README and `NOTICE` must credit the upstream project and commit.
3. Rename the package and product:
   - `package.json`: `"name": "proton-drive-linux"`, `"version": "0.1.0"`, repository URL, description.
   - `src-tauri/Cargo.toml`: package name/version.
   - `src-tauri/tauri.conf.json`: `productName: "Proton Drive for Linux (Unofficial)"`, `version: "0.1.0"`, `identifier: "io.github.zhafrisbs.protondrive-linux"`, `bundle.targets: ["appimage"]`.
4. Replace Proton branding:
   - Remove `src-tauri/icons/proton-drive.svg` and any Proton logo from the bundle.
   - Add a neutral sync/cloud icon: `src-tauri/icons/sync.svg` plus generated PNGs.
   - Do not use Proton's logo as the app icon.
5. Update `LICENSE` to AGPL-3.0-or-later (keep upstream copyright) and add `NOTICE`:
   - upstream repository, commit, copyright
   - Proton WebClients source and license
   - "Not affiliated with Proton AG"
6. Keep Calendar out of the build:
   - the `v1.4.4` base predates the Calendar integration; verify no Calendar
     build/copy blocks exist in `scripts/build-webclients.sh`
   - do not add Calendar later
   - leave Account and Verify in place for login/CAPTCHA
7. Rewrite `README.md` with:
   - what the app is
   - the difference from `Proton-Drive-Linux-CLI`
   - install/run instructions
   - the beta limitations (tray-only background, rename as delete+recreate)
   - AGPL/source link and unofficial disclaimer
8. Run the baseline checks:
   ```bash
   cd /home/zhafrisbs/Proton-Drive-Linux
   cargo test --manifest-path src-tauri/Cargo.toml
   npm ci
   npm run build:appimage
   ```
9. Launch the produced AppImage and verify login + Drive browsing.
10. Commit:
   ```bash
   git add -A
   git commit -m "chore: adopt upstream base as Proton-Drive-Linux"
   ```

**Acceptance:** AppImage builds, opens, logs in, browses Drive, and downloads a file. Calendar is absent. The app icon is not Proton-branded.

---

## Task 1 — Phase 0 spike: verify the Drive SDK inside the bundled app

**Goal:** Prove the APIs this design depends on before building the whole sync engine.

**Files:**
- Create: `docs/spikes/2026-10-24-drive-sdk.md`
- Create: `scripts/assets/ProtonDriveLinuxSpike.tsx` (temporary)
- Modify: `scripts/patch_drive_linux_sync_bridge.py` (temporary injection)

**Steps:**

1. Add a temporary component mounted inside the authenticated Drive provider. It must log to a visible panel and to the console:
   - `getDrive()` resolves.
   - `iterateDevices()` returns devices; create a test Linux device if needed.
   - `getNode(device.rootFolderUid)` returns the root and `treeEventScopeId`.
   - `createFolder(testRootUid, "spike-folder")` succeeds.
   - `trashNodes([folderUid])` succeeds and the folder disappears from `iterateFolderChildren`.
   - `getFileUploader(parentUid, "spike-20mb.bin", metadata)` with a `ReadableStream` uploads 20 MiB.
   - `getFileDownloader(fileUid).downloadToStream(writable)` downloads it and the hash matches.
   - `subscribeToTreeEvents(scopeId, listener)` fires when a file is changed from the Proton web UI.
2. Add a temporary Rust chunk bridge for the 1 GiB test:
   - `sync_begin_upload` / `sync_read_upload_chunk` / `sync_close_upload`
   - `sync_begin_download` / `sync_write_download_chunk` / `sync_finish_download`
   This is the real transfer interface; do not build a second one later.
3. Upload and download a 1 GiB test file through the chunk bridge. Record peak RSS (`/proc/<pid>/status`) and confirm it stays below roughly 300 MiB.
4. Test with a **separate Proton test account** if available, using an isolated device named `ProtonDrive-Spike-<date>` and a disposable local folder. Do not use the primary account or real files. If only the primary account exists, do not run this login spike; continue with local/planner tests and wait for a test account.
5. Record results in `docs/spikes/2026-10-24-drive-sdk.md`:
   - exact SDK method names used
   - event latency
   - memory result
   - any fallback needed
6. Remove the temporary spike component after recording results, but keep the chunk-transfer module.

**Fallback if streaming fails:** keep polling for remote changes and cap uploads at 100 MiB using whole-file `File` objects until a local HTTP streaming bridge is built. Do not silently ship a broken large-file path.

**Acceptance:** All five SDK checks pass, the 1 GiB transfer succeeds, and the spike document is committed.

```bash
git add scripts src-tauri docs/spikes
git commit -m "spike: verify Drive SDK streaming, events, and device APIs"
```

---

## Task 2 — Rust state store

**Files:**
- Create: `src-tauri/src/sync_types.rs`
- Rewrite: `src-tauri/src/sync_db.rs`
- Modify: `src-tauri/Cargo.toml` (add `rusqlite`, `serde`, `tempfile` if absent)

**Interfaces:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncItemRecord {
    pub relative_path: String,
    pub local_kind: String,          // "file" | "dir"
    pub local_size: Option<i64>,
    pub local_mtime_ns: Option<i64>,
    pub remote_uid: Option<String>,
    pub remote_kind: Option<String>,
    pub remote_size: Option<i64>,
    pub remote_mtime_ms: Option<i64>,
    pub state: String,               // "synced" | "local_pending" | "remote_pending" | "conflict" | "error"
    pub last_error: Option<String>,
    pub updated_at_ns: i64,
}

pub struct SyncDb { /* rusqlite::Connection */ }

impl SyncDb {
    pub fn open(path: &Path) -> Result<Self, String>;
    pub fn upsert_root(&self, root: &RootRecord) -> Result<(), String>;
    pub fn get_root(&self, local_path: &str) -> Result<Option<RootRecord>, String>;
    pub fn load_items(&self, root_id: &str) -> Result<Vec<SyncItemRecord>, String>;
    pub fn upsert_item(&self, root_id: &str, item: &SyncItemRecord) -> Result<(), String>;
    pub fn delete_item(&self, root_id: &str, relative_path: &str) -> Result<(), String>;
}
```

**Schema:** use the tables in the design document, section 7.4. Use raw relative paths, directory `0700`, database file `0600`. Do not hash paths; the sync engine must map a path to a remote UID.

**TDD steps:**

1. Write failing tests in `sync_db.rs`:
   - round-trip a root and two items
   - `load_items` returns them sorted by `relative_path`
   - `delete_item` removes only the requested row
   - database file permissions are `0600` on Unix
   - a second `open` works and sees committed rows
2. Run `cargo test --manifest-path src-tauri/Cargo.toml sync_db`.
3. Implement the schema and methods.
4. Re-run until green.
5. Commit: `feat: add local sync state database`.

**Acceptance:** Tests pass; no plaintext path is hashed away; DB is private.

---

## Task 3 — Local scan, path safety, and trash

**Files:**
- Create: `src-tauri/src/sync_fs.rs`
- Modify: `src-tauri/Cargo.toml` (add `trash`, `filetime` if absent)
- Modify: `src-tauri/src/main.rs` (module declaration only)

**Interfaces:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEntry {
    pub relative_path: String,
    pub kind: String,        // "file" | "dir"
    pub size: i64,
    pub mtime_ns: i64,
}

pub fn scan_local_tree(root: &Path) -> Result<Vec<LocalEntry>, String>;
pub fn validate_relative_path(relative: &str) -> Result<PathBuf, String>;
pub fn validate_path_within_root(root: &Path, target: &Path) -> Result<(), String>;
pub fn trash_local_path(root: &Path, relative: &str) -> Result<(), String>;
pub fn set_mtime_ms(path: &Path, mtime_ms: i64) -> Result<(), String>;
```

**Rules:**
- Reject absolute paths, empty paths, `..`, and symlinks.
- Canonicalize the root and the existing target ancestor.
- Skip temporary files and known lock files:
  - `.proton-part-*`
  - `*.swp`, `*.swx`, `.*.sw[a-p]`
  - `.~lock.*#`, `~$*`, `.#*`
  - `.DS_Store`, `._*`
  - `.Trash-*`
- Include directories.
- `trash_local_path` uses the `trash` crate; if that fails, move to `{app_data}/trash/<timestamp>/<relative>`.
- `set_mtime_ms` uses `filetime`.

**TDD steps:**

1. Write tests with `tempfile::tempdir()`:
   - nested files/dirs are returned with POSIX relative paths
   - ignored names are skipped
   - symlinks are skipped
   - `../x`, `/x`, `""` are rejected
   - a path escaping the root is rejected
   - trash fallback preserves file contents
   - mtime round-trip is within 1 second
2. Run `cargo test --manifest-path src-tauri/Cargo.toml sync_fs`.
3. Implement.
4. Commit: `feat: add safe local scan and trash primitives`.

**Acceptance:** Tests pass; no path can escape the root.

---

## Task 4 — Watcher, poller, and suppression

**Files:**
- Create: `src-tauri/src/sync_watcher.rs`
- Modify: `src-tauri/src/main.rs`
- Delete/retire: experimental sync code in `src-tauri/src/live_sync.rs` after its useful tests move

**Interfaces:**

```rust
pub struct SyncWatcher { /* JoinHandle + stop flag */ }

impl SyncWatcher {
    pub fn start(app: AppHandle, root: PathBuf) -> Result<Self, String>;
    pub fn stop(self);
    pub fn suppress(&self, relative_path: &str);
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalChangeEvent {
    pub root: String,
    pub paths: Vec<String>,
    pub kind: String,       // "create" | "modify" | "remove" | "rename"
    pub source: String,     // "watcher" | "poller"
}
```

**Behaviour:**
- `notify::RecommendedWatcher` recursive.
- Map `Create`, `Modify(Data)`, `Modify(Name)`, `Remove`; include both sides of a rename.
- Coalesce events in a bounded 2-second window before emitting `sync://local-change`.
- Run a full scan every 30 seconds and emit the diff as `source: "poller"`.
- Suppression cache: path → `Instant`, TTL 30 seconds, max 4096 entries.
- Ignore all paths from Task 3.
- Stop cleanly and join the watcher thread.

**TDD steps:**

1. Unit-test the pure diff/suppression functions:
   - added, removed, changed by size, changed by mtime
   - suppression expires
   - cache is bounded
   - path traversal is rejected before emission
2. Implement.
3. Commit: `feat: add local watcher and safety poller`.

**Acceptance:** Saving a local file emits one event; downloading a remote file does not echo back as a local change.

---

## Task 5 — Chunked upload/download transfer

**Files:**
- Create: `src-tauri/src/sync_transfer.rs`
- Modify: `src-tauri/src/main.rs`

**Interfaces:**

```rust
pub struct UploadStart { pub transfer_id: u64, pub size: u64, pub mtime_ms: i64 }
pub struct DownloadStart { pub transfer_id: u64 }

pub struct TransferManager { /* Mutex<HashMap<u64, Transfer>> */ }

impl TransferManager {
    pub fn begin_upload(&self, root: &Path, relative: &str) -> Result<UploadStart, String>;
    pub fn read_upload_chunk(&self, id: u64, max_bytes: usize) -> Result<Option<String>, String>; // base64
    pub fn close_upload(&self, id: u64);
    pub fn begin_download(&self, root: &Path, relative: &str, mtime_ms: i64) -> Result<DownloadStart, String>;
    pub fn write_download_chunk(&self, id: u64, data_base64: &str) -> Result<(), String>;
    pub fn finish_download(&self, id: u64) -> Result<(), String>;
    pub fn abort_download(&self, id: u64);
}
```

**Rules:**
- Chunk size from JS: 4 MiB.
- Upload reads from an open `File`; close removes the handle.
- Download writes to `.proton-part-<id>` in the destination directory, fsyncs, sets mtime, renames atomically.
- `finish_download` validates the temp path is inside the root and refuses symlinks.
- `abort_download` always removes the temp file.
- Never base64 the whole file.

**TDD steps:**

1. Tests:
   - upload chunks reassemble to the original bytes
   - download chunks write exact bytes and `finish` replaces an existing file
   - `abort` leaves no temp file
   - target escaping the root is rejected
   - mtime is applied
2. Implement.
3. Commit: `feat: add chunked local file transfer bridge`.

**Acceptance:** A 1 GiB file transfers without loading it all into memory.

---

## Task 6 — Tauri commands and app state

**Files:**
- Create: `src-tauri/src/sync_commands.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/Cargo.toml` if needed

**Commands (all check caller origin first):**

```rust
sync_get_root() -> Option<RootRecord>
sync_set_root(path: String, device_name: String) -> RootRecord
sync_start()
sync_stop()
sync_get_status() -> SyncStatus
sync_scan_local() -> Vec<LocalEntry>
sync_load_items() -> Vec<SyncItemRecord>
sync_put_item(item: SyncItemRecord) -> ()
sync_delete_item(relative_path: String) -> ()
sync_begin_upload(relative_path: String) -> UploadStart
sync_read_upload_chunk(transfer_id: u64, max_bytes: usize) -> Option<String>
sync_close_upload(transfer_id: u64) -> ()
sync_begin_download(relative_path: String, mtime_ms: i64) -> DownloadStart
sync_write_download_chunk(transfer_id: u64, data_base64: String) -> ()
sync_finish_download(transfer_id: u64) -> ()
sync_abort_download(transfer_id: u64) -> ()
sync_trash_local(relative_path: String) -> ()
sync_get_device_name() -> String
```

**App state:**
- `SyncDb`
- `SyncWatcher`
- `TransferManager`
- active root path
- status snapshot

**Rules:**
- Reuse `ensure_sync_command_allowed` from upstream.
- Validate the root on startup and on every command.
- Emit `sync://status`, `sync://error`, and `sync://conflict` events for the UI.
- Do not add a command that reads a whole file for sync.

**TDD steps:**

1. Test the inner helpers (path validation, status transitions) without Tauri.
2. Register commands in `invoke_handler`.
3. Commit: `feat: expose typed sync commands to the web app`.

**Acceptance:** The panel can start/stop sync, scan local files, and transfer chunks.

---

## Task 7 — JavaScript planner

**Files:**
- Create: `scripts/assets/sync-planner.ts`
- Create: `tests/js/planner.test.ts`
- Modify: `package.json` (add `vitest`, `test:planner`)

**Interfaces:**

```ts
export type LocalSide = { exists: true; kind: 'file' | 'dir'; size: number; mtimeMs: number };
export type RemoteSide = { exists: true; kind: 'file' | 'dir'; uid: string; size?: number; mtimeMs?: number };
export type StoredSide = { local?: LocalSide; remote?: RemoteSide; state: string };

export type Action =
  | { type: 'link_only'; path: string }
  | { type: 'mkdir_remote'; path: string }
  | { type: 'mkdir_local'; path: string }
  | { type: 'upload_new'; path: string }
  | { type: 'upload_revision'; path: string; remoteUid: string }
  | { type: 'download_new'; path: string; remoteUid: string }
  | { type: 'download_update'; path: string; remoteUid: string }
  | { type: 'download_conflict'; path: string; remoteUid: string; conflictPath: string }
  | { type: 'trash_remote'; path: string; remoteUid: string }
  | { type: 'trash_local'; path: string }
  | { type: 'remove_state'; path: string }
  | { type: 'hold'; path: string; reason: string };

export function planPath(
  path: string,
  local: LocalSide | undefined,
  remote: RemoteSide | undefined,
  stored: StoredSide | undefined,
  options: { deleteEnabled: boolean; mtimeToleranceMs: number }
): Action[];

export function conflictName(path: string, now: Date): string;
```

**Decision table tests:**

| Case | Expected |
|---|---|
| local only, no state | `upload_new` |
| remote only, no state | `download_new` |
| both, no state, same size and mtime within 2s | `link_only` |
| both, no state, different | `download_conflict` + `upload_new` |
| both, local changed only | `upload_revision` |
| both, remote changed only | `download_update` |
| both changed | `download_conflict` + `upload_revision` |
| local deleted, remote unchanged | `trash_remote` |
| local deleted, remote changed | `download_new` |
| remote deleted, local unchanged | `trash_local` |
| remote deleted, local changed | `upload_new` |
| both deleted | `remove_state` |
| delete disabled | no `trash_*` action; `hold` |
| empty local tree but state has items | `hold` |
| empty remote tree but state has items | `hold` |
| mass delete above threshold | `hold` |

**TDD steps:**

1. Write the table tests first.
2. Run `npm run test:planner` and see them fail.
3. Implement `planPath` and `conflictName`.
4. Re-run green.
5. Commit: `feat: add testable two-way sync planner`.

**Acceptance:** All planner cases pass without a Proton account.

---

## Task 8 — JS Drive bridge: device, scan, upload

**Files:**
- Create: `scripts/assets/ProtonDriveLinuxSyncBridge.tsx`
- Modify: `scripts/patch_drive_linux_sync_bridge.py`
- Modify: `scripts/build-webclients.sh`

**Bridge responsibilities:**
- Mount inside the authenticated `DriveProvider`.
- Resolve the Linux device:
  ```ts
  const drive = getDrive();
  for await (const device of drive.iterateDevices()) { ... }
  const device = await drive.createDevice(deviceName, DeviceType.Linux);
  ```
- Get the root node and `treeEventScopeId`.
- Scan remote recursively with `iterateFolderChildren`.
- Create missing folders with `createFolder`.
- Upload with `getFileUploader(...).uploadFromStream(stream)`.
- Update with `getFileRevisionUploader(...).uploadFromStream(stream)`.
- Use `sync_begin_upload` + `sync_read_upload_chunk` for the stream.
- Update state only after the SDK reports success.
- Expose `window.__PROTON_DRIVE_LINUX_SYNC__ = { syncNow, pause, status }` for the panel.

**Minimum upload cycle:**
1. `sync_scan_local`
2. `sync_load_items`
3. remote scan
4. `planPath` for each path
5. execute `mkdir_remote` and `upload_new` first
6. write state
7. emit status

**TDD/manual:**
- Use `ProtonDrive-Sync-<date>` device and `/tmp/protondrive-sync`.
- Create nested local files and one empty folder.
- Confirm they appear under **Computers**.
- Confirm no duplicate uploads after a second cycle.
- Commit: `feat: add Drive bridge remote scan and uploads`.

**Acceptance:** Local files and folders upload; restarting the app does not re-upload unchanged files.

---

## Task 9 — JS Drive bridge: downloads, deletes, conflicts, events

**Files:**
- Modify: `scripts/assets/ProtonDriveLinuxSyncBridge.tsx`
- Modify: `scripts/assets/sync-planner.ts` only if a test exposes a missing case

**Add:**
- Download:
  ```ts
  const downloader = await drive.getFileDownloader(remoteUid);
  await downloader.downloadToStream(writable);
  ```
  `writable` calls `sync_write_download_chunk` then `sync_finish_download`.
- Remote deletes: `drive.trashNodes([remoteUid])`; never `deleteNodes`.
- Local deletes: `sync_trash_local`.
- Conflicts: download remote to `conflictName(path)` then upload local canonical.
- Mass-delete guard: check totals before executing any delete actions; emit `hold`.
- Events:
  ```ts
  const subscription = await drive.subscribeToTreeEvents(scopeId, () => schedule());
  ```
  `schedule()` debounces 1.5s; one cycle at a time; mark dirty if events arrive during a cycle.
- Safety poll every 5 minutes and on manual "Sync now".
- Suppress local watcher echoes after each download via `sync_mark_suppressed` if needed.

**Manual acceptance:**
- remote create → local download
- remote modify → local update
- remote delete → local trash
- local delete → remote trash
- edit both sides → two files, one named `(conflict ...)`
- delete local while remote changed → remote file is restored, no data loss
- empty folder round-trip
- commit: `feat: complete two-way sync with trash and conflict safety`.

---

## Task 10 — Sync panel, tray, and first-run flow

**Files:**
- Create: `scripts/assets/DriveLinuxPanel.tsx`
- Modify: `scripts/patch_drive_linux_panel.py`
- Modify: `scripts/patch_drive_linux_drawer.py`
- Modify: `src-tauri/src/main.rs` (tray/window)
- Modify: `scripts/build-webclients.sh`

**Panel:**
- status dot: active / paused / syncing / error
- local folder path and "Choose folder" (`invoke('plugin:dialog|open', { options: { directory: true, multiple: false } })`)
- Start / Pause / Sync now
- last successful sync, pending count, conflict count
- error text
- "Open folder"
- "Open Proton Drive Computers"
- settings: deletes on/off, ignore patterns, autostart minimized
- first-run disclosure: unofficial, credentials handled by Proton, sync-while-running

**Tray:**
- Open
- Sync now
- Pause/Resume
- Quit

**Window behaviour:**
- close → hide to tray, keep syncing
- quit → stop watcher, abort transfers, exit
- autostart entry launches with `--minimized`

**Tests:**
- UI renders with no root and with a root
- button handlers call the right commands (mock `invoke`)
- tray menu actions are wired
- commit: `feat: add Linux sync panel and tray controls`.

---

## Task 11 — Build, CI, docs, and beta release

**Files:**
- Create: `.github/workflows/build-appimage.yml`
- Create: `WebClients.lock`
- Modify: `scripts/build-webclients.sh`
- Modify: `README.md`, `docs/BUILD.md`, `docs/SYNC.md`, `docs/SECURITY.md`

**Build:**
- Pin WebClients commit in `WebClients.lock` (start with `bbad1a0a482227b93a2e963a232463aede9b8abf`; update only after a successful build).
- Build Drive + Account + Verify sequentially; skip Calendar.
- AppImage only.
- GitHub Actions: Node 22, Rust stable, WebKitGTK 4.1 dev, GTK3, `libayatana-appindicator3-dev`, GStreamer.
- Cache Yarn, Cargo, and the WebClients checkout.
- Run Rust tests and planner tests before packaging.
- On `v*` tags, attach AppImage + SHA256.

**Docs:**
- Build dependencies and commands
- first-run/login/CAPTCHA
- sync scope and limitations
- trash/conflict/mass-delete safety
- AGPL source/attribution
- troubleshooting (WebKitGTK, tray, missing libs)

**Release gate:**
- [ ] AppImage launches on Ubuntu 22.04/24.04 and the user's distro
- [ ] login, 2FA, CAPTCHA
- [ ] local create/modify/delete
- [ ] remote create/modify/delete
- [ ] nested and empty folders
- [ ] Unicode and spaces
- [ ] conflict
- [ ] 1 GiB upload/download
- [ ] restart/offline recovery
- [ ] no permanent deletes
- [ ] no Proton logo
- [ ] AGPL notices and source link
- [ ] tag `v0.1.0`

---

## Execution notes

- Work on a dedicated branch or worktree, not the user's home directory.
- Use a separate Proton test account. Never use the primary account for login or sync testing.
- Use an isolated test device `ProtonDrive-Test-<date>` and `/tmp/protondrive-test`; no real files.
- If Proton limits an account, stop retrying and follow `docs/ACCOUNT-SAFETY.md`.
- Do not point the sync root at `$HOME`, `~/Documents`, `~/Downloads`, or the existing `~/ProtonDrive` without explicit permission.
- Every remote delete must be `trashNodes`, never `deleteNodes`.
- Every local delete must go to trash.
- Commit after each green test.
- If a task changes the design, update `/home/zhafrisbs/Proton-Drive-Linux-DESIGN.md` and call it out before continuing.
