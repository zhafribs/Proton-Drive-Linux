# Proton Drive for Linux — Web Edition

**Design document — draft for approval**

> **Superseded auth path (2026-10-24):** the bundled-web-app auth design below
> triggered a Proton account restriction. The current implementation uses the
> official `drive.proton.me` webview plus the official CLI sync engine. See
> [OFFICIAL-WEB-SPIKE.md](OFFICIAL-WEB-SPIKE.md). The sync/safety requirements
> in this document still apply.

- **Status:** Draft, not implemented
- **Chosen direction:** Continue the architecture of `DonnieDice/protondrive-linux`
- **License:** AGPL-3.0-or-later
- **Working repo name:** `Proton-Drive-Linux`
- **Working product name:** Proton Drive for Linux (Unofficial) — Web Edition
- **Existing app:** `Proton-Drive-Linux-CLI` remains unchanged and MIT-licensed
- **First release target:** AppImage, Linux x86_64, beta (`0.1.0`)

---

## 1. Summary

Build a second, fast Proton Drive desktop app for Linux. It will use the same basic approach as the app at `DonnieDice/protondrive-linux`: a Tauri/WebKitGTK shell around Proton's own Drive web app, with a native local sync layer.

The difference is that the sync layer will be treated as a first-class feature, not an experiment. The first beta will deliver:

- Proton's real web UI, login, CAPTCHA, 2FA, browsing, previews, and downloads
- One sync folder mapped to a Linux device under Proton Drive's **Computers** section
- Two-way sync of files and folders
- Trash-based deletes, conflict copies, restart recovery, and large-file streaming
- A native "Linux Sync" panel inside the Drive UI
- An AppImage that runs on normal Linux desktops with WebKitGTK

The existing CLI-backed app stays separate. This app does not call `proton-drive` and does not use the CLI app's sync engine.

---

## 2. Goals

### Must have

1. **Fast, familiar UI** — Proton's own Drive web app, not a custom imitation.
2. **No CLI dependency** — no `proton-drive` binary, no Python sync engine.
3. **Full two-way sync** for one local folder:
   - create, modify, delete files
   - create and delete folders
   - nested trees and empty folders
   - remote changes from another device or the web UI
   - restart recovery and offline retry
4. **Large files** — streamed in chunks, not base64-encoded whole files.
5. **Safe deletes** — remote deletes go to Proton trash; local deletes go to the desktop trash; no permanent deletes.
6. **Conflict safety** — keep both versions instead of overwriting.
7. **Native sync controls** — choose folder, start/stop, sync now, status, errors, conflicts.
8. **Single login** — the web app's session is the only session.
9. **AGPL compliance** — full source, notices, and attribution.

### Nice to have after beta

- rename/move optimization instead of delete + recreate
- multiple sync folders
- selective sync by subfolder
- bandwidth limits
- sync history and per-file activity
- headless systemd service
- non-AppImage packages

### Out of scope for beta

- sharing permissions and invitation management
- symlink sync
- preserving Linux permissions, owners, or extended attributes
- syncing files while the app is fully closed
- Windows or macOS

---

## 3. Why this approach

The reference app gets its speed from Proton's real web application. Login, cryptography, previews, uploads, downloads, and file browsing are Proton's own code. Reimplementing that in Rust would be a large, fragile project and would not produce the same UI.

The reference app's weakness is its sync layer: it is experimental, its upload bridge depends on a patched React component, local deletes are not wired through, remote changes are not fully applied, and it has no complete conflict model. This design keeps the fast web app but replaces the sync experiment with a deliberate sync engine built on the public Drive SDK APIs that are already inside the web app.

We will start from the upstream reference at tag `v1.4.4`, preserve its history and AGPL license, then make substantial changes. This is faster and safer than starting from an empty Tauri project.

---

## 4. Names and separation

| | Existing app | New app |
|---|---|---|
| Repo | `Proton-Drive-Linux-CLI` | `Proton-Drive-Linux` |
| Backend | official Proton CLI | Proton web app + Drive SDK |
| UI | native Rust/egui | Proton web app in Tauri |
| License | MIT | AGPL-3.0-or-later |
| Sync | existing Python/CLI engine | new native/web sync engine |
| Status | stable, maintained | new beta |

The new repo is named `Proton-Drive-Linux` to match the requested split: `Proton-Drive-Linux-CLI` is the official-CLI-backed app, and `Proton-Drive-Linux` is the app without the CLI. The README will describe it as the web-based edition so the two are still distinguishable.

No AGPL code from the new app will be copied into the MIT app. MIT code from the existing app may be reused in the AGPL app if useful; the license is compatible in that direction.

---

## 5. User experience

### First run

1. App opens the Proton Drive web app.
2. User signs in through Proton's normal login, including CAPTCHA and 2FA.
3. A one-time dialog explains:
   - this is an unofficial third-party client
   - Proton may temporarily limit an account after a login from an unofficial
     client; do not use a primary account for beta testing
   - credentials are entered into Proton's web login, not stored by this app
   - sync runs while the app is running
4. User chooses a local folder (default `~/ProtonDrive-Linux`, kept separate from the old CLI app's `~/ProtonDrive`).
5. Sync starts. A Linux device is created or reused under Proton Drive's **Computers** section.

### Normal use

- The web app browses files as usual.
- A new "Linux Sync" entry appears in the Drive drawer/sidebar.
- The panel shows:
  - active/stopped state
  - local folder
  - last successful sync
  - pending uploads/downloads
  - conflicts and errors
  - "Sync now", "Pause", "Choose folder", "Open folder"
- Tray menu: Open, Sync now, Pause/Resume, Quit.
- Closing the window hides to tray; sync continues. Quitting stops sync.
- Autostart can launch the app minimized to tray.

### Sync behaviour

- Local file saves are detected immediately and uploaded after a short debounce.
- Remote changes are detected by Proton's drive event subscription (about 30 seconds worst case) plus a periodic full reconciliation.
- Manual "Sync now" forces a full reconciliation.
- Conflicts create a copy named `name (conflict YYYY-MM-DD HHMMSS).ext` and keep both files.
- Deletes are recoverable on both sides.

---

## 6. Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│ Tauri / WebKitGTK window                                         │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │ Proton WebClients (bundled, AGPL)                          │  │
│  │  Drive UI, Account login, Verify/CAPTCHA                   │  │
│  │                                                            │  │
│  │  ProtonDriveLinuxSyncBridge.tsx                            │  │
│  │   - Drive SDK access (getDrive())                          │  │
│  │   - remote scan, upload, download, trash, events           │  │
│  │   - sync planner and queue                                  │  │
│  │                                                            │  │
│  │  DriveLinuxPanel.tsx                                       │  │
│  │   - sync UI and controls                                   │  │
│  └──────────────────────────┬─────────────────────────────────┘  │
│                             │ Tauri IPC                          │
│  ┌──────────────────────────▼─────────────────────────────────┐  │
│  │ Rust backend                                               │  │
│  │  - local file watcher and safety poller                    │  │
│  │  - path validation and origin checks                       │  │
│  │  - chunked file reads/writes                               │  │
│  │  - local trash and atomic downloads                        │  │
│  │  - SQLite sync-state store                                 │  │
│  │  - HTTP cookie proxy for Proton API                         │  │
│  │  - tray, window, autostart                                 │  │
│  └────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
                              │
                              ▼
                    Proton Drive service
                    (through the bundled web app's authenticated client)
```

### Why split Rust/JS this way?

- **Rust** is good at filesystem watching, path safety, atomic writes, permissions, SQLite, and tray integration.
- **JavaScript** already has Proton's authenticated Drive SDK, crypto, upload/download streams, and event subscription inside the web app.
- Keeping remote operations in JS avoids reimplementing Proton cryptography and avoids the CLI entirely.

The two sides communicate through a small, typed Tauri command/event contract.

---

## 7. Components

### 7.1 Bundled Proton WebClients

- Start from the pinned WebClients build used by upstream `v1.4.4`.
- Build the Drive, Account, and Verify applications. **Do not include Calendar in the beta.**
- Keep the upstream patches for Tauri URL handling, login, CAPTCHA, and drawer integration.
- Add our own patches for:
  - the sync bridge component
  - the Linux sync panel
  - any SDK event hooks
- Remove or hide the Linux Calendar drawer entry so the UI does not link to an unbuilt app.
- Pin the exact WebClients commit in `WebClients.lock`; never build an unreviewed moving `main`.

#### Calendar decision

Calendar is unrelated to Drive sync. Keeping it would add another full Proton application build, more memory during compilation, more patches that can break when Proton changes, and extra AppImage size. It would also add a second product area to test and maintain.

Removing it means the beta has no calendar drawer entry. That is the right trade-off for a Drive-focused app: shorter builds, fewer moving parts, smaller package, and more time spent on sync correctness. The web app still provides Drive, Account, and Verify (login/CAPTCHA); the calendar drawer entry will be hidden or removed by a small patch.

### 7.2 Rust: local watcher and poller

Adapt `src-tauri/src/live_sync.rs`:

- recursive inotify watcher via `notify`
- 30-second full scan as a safety net
- includes files and directories
- ignores symlinks and temporary/lock files
- coalesces bursts and ignores changes caused by our own remote writes (suppression cache)
- emits a native event with root-relative paths
- rejects paths outside the sync root, symlinks, absolute paths, and `..`
- stops cleanly and joins threads

The reference only tracks files. This design tracks folders too.

### 7.3 Rust: file transfer bridge

Remote upload/download must not base64 the entire file. Add a chunked transfer layer:

- `open_upload(root, relativePath)` → transfer ID, size, mtime
- `read_upload_chunk(transferId)` → up to 4 MiB of base64 data, or EOF
- `close_upload(transferId)`
- `begin_download(root, relativePath, expectedSize, mtime)` → transfer ID and temp path
- `write_download_chunk(transferId, dataBase64)` → append
- `finish_download(transferId)` → fsync, set mtime, atomic rename
- `abort_download(transferId)` → delete temp file

JS wraps these commands in a `ReadableStream` for the SDK uploader and a `WritableStream` for the SDK downloader. The SDK handles Proton encryption and integrity checks; Rust handles local disk I/O.

Temp files live beside the destination with a reserved prefix, for example `.proton-part-<random>`, and are excluded from scanning. If the filesystem does not support atomic rename across the temp location, the temp file is created in the destination directory.

### 7.4 Rust: sync-state store

Replace the reference's hash-only sync database with a local state store that can actually map paths to remote nodes.

SQLite file: `{app_data_dir}/sync-state.sqlite3`, directory `0700`, file `0600`.

Tables:

```sql
CREATE TABLE roots (
  id TEXT PRIMARY KEY,
  local_path TEXT NOT NULL,
  device_uid TEXT,
  device_name TEXT,
  remote_root_uid TEXT,
  created_at_ns INTEGER NOT NULL,
  updated_at_ns INTEGER NOT NULL
);

CREATE TABLE items (
  root_id TEXT NOT NULL,
  relative_path TEXT NOT NULL,
  local_kind TEXT NOT NULL,          -- file | dir
  local_size INTEGER,
  local_mtime_ns INTEGER,
  remote_uid TEXT,
  remote_kind TEXT,                  -- file | dir
  remote_size INTEGER,
  remote_mtime_ms INTEGER,
  state TEXT NOT NULL,               -- synced | local_pending | remote_pending | conflict | error
  last_error TEXT,
  updated_at_ns INTEGER NOT NULL,
  PRIMARY KEY (root_id, relative_path),
  FOREIGN KEY (root_id) REFERENCES roots(id) ON DELETE CASCADE
);
```

Paths are raw because the sync engine must map a local path to a remote UID. They never leave the machine. No file contents or credentials are stored.

The state store is the source of truth for "what did this path look like on each side at the last successful sync?"

### 7.5 JS: Drive SDK bridge

Add `ProtonDriveLinuxSyncBridge.tsx` inside the Drive app's `DriveProvider`, near `UploadProvider`.

Responsibilities:

- wait for an authenticated Drive client
- resolve or create the Linux device:
  - `getDrive().iterateDevices()`
  - `getDrive().createDevice(hostname, DeviceType.Linux)`
  - use `device.rootFolderUid`
- get the root node and its `treeEventScopeId`
- subscribe to drive events with `subscribeToTreeEvents`
- perform remote scans with `iterateFolderChildren`
- create missing folders with `createFolder`
- upload with `getFileUploader(...).uploadFromStream(...)`
- update existing files with `getFileRevisionUploader(...)`
- download with `getFileDownloader(...).downloadToStream(...)`
- trash remote nodes with `trashNodes` (never `deleteNodes`)
- run the sync planner and queue
- report status to the native panel

The bridge is a headless React component. It renders nothing.

### 7.6 JS: sync planner

The planner is a pure function over three views:

- local tree
- remote tree
- last-synced state

It produces a list of actions:

- `upload_new`
- `upload_revision`
- `download_new`
- `download_update`
- `mkdir_remote`
- `mkdir_local`
- `trash_remote`
- `trash_local`
- `conflict_keep_both`
- `link_only`
- `remove_state`

Keeping the planner pure makes it testable without a Proton account.

### 7.7 JS: Linux Sync panel

Replace/extend `DriveLinuxPanel.tsx`:

- sync status dot and state
- folder picker via Tauri dialog
- start/stop/pause
- sync now
- pending count, last sync time, error text
- conflict count with "Open folder"
- "Open Proton Drive Computers" action
- advanced settings:
  - sync deletes on/off (default on, always to trash)
  - ignore patterns
  - remote poll fallback interval
  - autostart minimized
- first-run disclosure

---

## 8. Sync model

### 8.1 Scope

- One local folder per app profile.
- One remote root: the Linux device's folder under **Computers**.
- Device name: sanitized hostname; reused if it already exists, otherwise created.
- All paths inside the local folder are in scope except ignored names.
- The remote root is not the whole Proton account; it is the Linux device root. This matches Proton's desktop-computer model and avoids mixing with My Files.

### 8.2 Sync state per path

For each path, the state store remembers:

- local kind, size, and mtime at last successful sync
- remote UID, kind, size, and claimed mtime at last successful sync

A side has changed if its current fingerprint differs from the stored fingerprint.

### 8.3 Decision table

| Local now | Remote now | State | Action |
|---|---|---|---|
| exists | exists | none | if same size/mtime within tolerance: link; else keep both (remote conflict copy, local canonical) |
| exists | exists | synced | if both unchanged: no-op; local only: upload revision; remote only: download; both: conflict |
| exists | missing | none | upload new |
| exists | missing | synced | remote deleted: if local unchanged, trash local (propagate the deletion); if local changed, upload new (keep the edit) |
| missing | exists | none | download new |
| missing | exists | synced | local deleted: if remote unchanged, trash remote; if remote changed, download remote (remote edit wins over local delete) |
| missing | missing | synced | remove state |
| exists | exists | local_pending/remote_pending | resume the pending action, then re-evaluate |

The implementation will be more detailed, but these rules define the safety direction: **never destroy the only copy of user data**.

### 8.4 Conflict policy

When both sides changed since the last sync:

1. Download the remote version to a conflict copy beside the local file.
2. Upload the local version as the canonical remote file.
3. Record both files in state.
4. Notify the user.

Conflict name:

```
name (conflict 2026-10-24 143012).ext
```

If that name exists, append ` (2)`, ` (3)`, etc.

### 8.5 Deletes

- Local deletion → `trashNodes([remoteUid])`. Proton keeps it in trash.
- Remote deletion → move the local file/folder to the desktop trash via `gio trash`; if that fails, move it to an app-managed trash folder, never permanently delete.
- Both deleted → remove state.
- A delete is held if the other side changed after the last sync (see decision table).
- **No code path calls `deleteNodes` or `emptyTrash`.**

### 8.6 Mass-delete guard

Before applying deletes:

- If a scan reports zero local files while the state contains files, and the remote side still has files: hold local deletes and surface an error.
- If a scan reports zero remote files while the state contains files, and the local side still has files: hold remote trashes and surface an error.
- If one cycle would delete more than 25% of tracked items and more than 10 items: pause and require explicit confirmation in the panel.

This protects against a moved disk, a wrong folder, a broken login, or a partial scan.

### 8.7 Renames and moves

Beta treats a rename/move as a delete on one path plus a create on another:

- local move: old remote path is trashed, new remote file is uploaded
- remote move: old local file is trashed, new local file is downloaded

This is correct but not efficient. The old version remains in Proton trash/revision history. A later release can detect same-content moves and use `moveNodes`/local rename instead.

### 8.8 Ignored files

Always skip:

- symlinks
- `.Trash-*`
- LibreOffice locks `.~lock.*#`
- Microsoft Office locks `~$*`
- Emacs locks `.#*`
- Vim swap files `.*.sw[a-p]`
- macOS `._*` and `.DS_Store`
- our own `.proton-part-*` temp files
- user-configured glob patterns

Hidden files are otherwise synced.

### 8.9 Large files and streaming

- Upload: Rust opens the file and serves 4 MiB chunks through Tauri IPC; JS builds a `ReadableStream`; the SDK's `uploadFromStream` encrypts and uploads it.
- Download: the SDK writes decrypted chunks to a JS `WritableStream`; each chunk is sent to Rust; Rust appends to a temp file, fsyncs, sets mtime, and renames atomically.
- No whole-file base64.
- Beta limit: 10 GiB per file (configurable); Proton's own limits still apply.
- One to two transfers run at a time to keep memory and rate limits predictable.

### 8.10 Events and polling

- Remote changes: `subscribeToTreeEvents(root.treeEventScopeId, listener)`.
- Event listener schedules a debounced reconciliation (1–2 s).
- The SDK's event manager polls every 30 seconds for the user's own volume; this is the worst-case remote latency.
- A full reconciliation runs every 5 minutes as a safety net, and on "Sync now".
- Local changes: watcher + 2-second quiet debounce.
- Only one sync cycle runs at a time; new events mark the cycle as dirty and it runs again after the current cycle.
- Failures use exponential backoff and are visible in the panel.

---

## 9. Authentication and security

### Authentication

- Reuse upstream's bundled Account/Verify apps and the Tauri proxy.
- Login, CAPTCHA, 2FA, and cookies stay in Proton's web app.
- No native password form, no password storage, no logging of credentials.
- The Drive SDK is available only after the authenticated `DriveProvider` mounts, so sync cannot start before login.
- On logout, sync pauses and state is retained; on next login, reconcile.

### Security

- All sync commands verify the caller origin is `tauri://localhost` (or `tauri.localhost`).
- All local paths are validated against the canonical sync root; symlink traversal is rejected.
- Remote paths are validated: no `..`, absolute components, or prefixes.
- The state DB and config are `0600`; directories are `0700`.
- Logs never include credentials, cookies, or full file contents; file paths are logged only in debug mode.
- No telemetry is sent by our code.
- The app remains a normal unprivileged desktop process; no setuid, no root.

### Legal and branding

- New repo is AGPL-3.0-or-later, with upstream attribution and Proton WebClients notices.
- The app is clearly labeled **Unofficial**.
- Do not use Proton's logo as the application icon; use a generic cloud/sync icon.
- Include a disclaimer that Proton Drive is a trademark of Proton AG and this project is not affiliated.
- First-run credential-handling disclosure.
- Publish the exact WebClients commit and our patches so the AGPL source is reproducible.

---

## 10. Packaging and build

### Target

- AppImage, Linux x86_64, glibc 2.35+ baseline
- Host requirements: WebKitGTK 4.1, GTK3, libayatana-appindicator, GStreamer base/good plugins
- ~70–100 MB AppImage, similar to upstream
- No CLI, Python, or external Proton binary bundled

### Build pipeline

1. Restore pinned WebClients commit from `WebClients.lock` (cacheable).
2. Apply upstream patches and our patches.
3. Install WebClients dependencies with Yarn.
4. Build Drive + Account + Verify sequentially to stay within memory limits.
5. `npm ci` and `tauri build --bundles appimage`.
6. Produce AppImage + SHA256.

### CI

- GitHub Actions, AppImage only for the first release.
- Node 22, Rust stable, WebKitGTK dev packages, `libayatana-appindicator3-dev`.
- Cache Cargo, Yarn, and WebClients checkout.
- Run Rust tests and the JS planner tests before packaging.
- Release on `v*` tags; upload AppImage and checksum.
- Keep the existing app's release workflow untouched.

### Repository layout

```
Proton-Drive-Linux/
  .github/workflows/build-appimage.yml
  src-tauri/
    src/main.rs
    src/live_sync.rs
    src/sync_db.rs
    src/sync_transfer.rs
    src/sync_commands.rs
    tauri.conf.json
  scripts/
    build-webclients.sh
    patch_drive_linux_sync_bridge.py
    patch_drive_linux_panel.py
    patch_drive_linux_drawer.py
    fetch-webclients.sh
  docs/
    ARCHITECTURE.md
    SYNC.md
    BUILD.md
    SECURITY.md
  WebClients.lock
  package.json
  README.md
  LICENSE
  NOTICE
```

Upstream history is imported, then our commits are added. The repo is independent, not a GitHub fork, but the README and NOTICE credit `DonnieDice/protondrive-linux` and link the upstream commit.

---

## 11. Testing

### Automated

Rust:

- path traversal and symlink rejection
- suppression cache and watcher event mapping
- state DB migrations and transactions
- chunked read/write and atomic finish
- local trash fallback
- mass-delete guard

JS:

- planner decision table
- conflict naming
- ignore patterns
- path normalization
- queue single-flight and debounce

Integration:

- fake local tree + fake remote tree, no Proton account
- simulated download/upload failures
- restart with pending state

### Manual acceptance matrix

A separate Proton test account is strongly recommended. Proton's anti-abuse
system can temporarily limit an account after a login from an unofficial
client, and this has already been reported upstream. Do not use the primary
Proton account for login or sync testing.

If a separate test account is available, use an isolated device and folder:

- remote device: `ProtonDrive-Test-<date>` under **Computers**
- local folder: `/tmp/protondrive-test` or `~/ProtonDrive-Test`
- only disposable test files; no real documents
- any remote test deletions go to Proton trash and are cleaned up afterward
- the test device can be removed from **Computers** after the run

If no separate test account is available, real login and sync acceptance waits.
Local Rust/planner tests continue without a Proton account.

The app must never be pointed at the user's real Documents, Downloads, or home directory during testing.

- login, logout, CAPTCHA, 2FA
- first sync: local-only, remote-only, both same, both different
- create/modify/delete local file
- create/modify/delete remote file
- create/delete folders, empty folders
- nested paths, spaces, Unicode names
- conflict: edit both sides
- delete vs edit on the other side
- large file (1 GiB) upload and download
- restart during a transfer
- offline, then online
- paused sync
- mass-delete guard
- lock/temp/symlink files ignored
- AppImage on Ubuntu 22.04/24.04 and the user's current distro

### Release gate

- no data loss in the acceptance matrix
- no permanent deletes
- sync resumes after restart
- AppImage launches and logs in on a clean machine with documented dependencies
- AGPL notices and source links present

---

## 12. Milestones

### M0 — Repo and build baseline

- Import upstream `v1.4.4` with history and AGPL notices
- Rename product/repo, replace Proton logo with generic icon
- Remove GitLab-specific CI; add GitHub AppImage workflow
- Build and run the AppImage unchanged
- **Gate:** app launches, logs in, browses, downloads

### M1 — Sync foundation

- device root resolution/creation
- local scan + remote scan
- state DB
- chunked transfer bridge
- upload new files/folders
- panel start/stop/status
- **Gate:** local files appear under Computers

### M2 — Two-way sync

- remote downloads
- remote/local deletes to trash
- conflict copies
- first-sync merge rules
- event subscription + safety poll
- **Gate:** bidirectional acceptance tests pass

### M3 — Hardening

- restart/offline/retry
- large files
- mass-delete guard
- error reporting
- tray/autostart
- automated tests
- **Gate:** release criteria met

### M4 — Beta release

- docs, screenshots, disclaimer
- AppImage + checksum
- tag `v0.1.0`
- **Gate:** user acceptance

Dates are not promised in this document. The first working local→remote AppImage is expected after M1; full two-way beta after M3.

---

## 13. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Proton WebClients changes break the build | app cannot ship | pin commit, keep patches small, CI catch, update monthly |
| SDK event APIs change | remote latency or breakage | public API only; fallback full polling |
| Streaming through Tauri/WebKit is unreliable | large files fail | Phase 0 spike; chunk size/backpressure tests; fallback to 100 MiB whole-file |
| Reference sync code is not reusable as-is | more work | new planner/state/streaming; keep only watcher/path safety |
| AGPL obligations | legal | source, notices, exact build inputs |
| Proton trademark/ToS | takedown risk | unofficial naming, no logo, disclaimer, credential disclosure |
| Proton official Linux client | project becomes redundant | ship fast, stay small; users can choose |
| Proton crypto migration | old client breaks | bundled web app updated with Proton; rebuild |
| Background sync not running when app is closed | user expects always-on | tray + autostart; document limitation; headless service later |
| Sync bugs cause data loss | unacceptable | trash-only deletes, conflict copies, mass-delete guard, test matrix |

---

## 14. Decisions

1. **Repo name:** `Proton-Drive-Linux` (product: Proton Drive for Linux, Unofficial — Web Edition). Existing app stays `Proton-Drive-Linux-CLI`.
2. **License:** new repo is AGPL-3.0-or-later; existing app stays MIT.
3. **Upstream base:** import `DonnieDice/protondrive-linux` at `v1.4.4` with history and attribution, then modify. Not a GitHub fork.
4. **Sync scope:** one local folder ↔ one Linux device under **Computers**. No My Files mapping in beta.
5. **Delete policy:** trash-only on both sides; no permanent deletes; mass-delete guard.
6. **Conflict policy:** keep both; remote conflict copy + local canonical.
7. **Background model:** sync runs while the app is running or in the tray; autostart minimized is supported; a headless service is out of scope for beta.
8. **First release:** AppImage only, Linux x86_64, beta `0.1.0`.
9. **Branding:** unofficial name, generic icon, no Proton logo, credential disclosure.
10. **Calendar:** excluded from beta; hide/remove the calendar drawer entry.
11. **Testing:** use a separate Proton test account if possible; never use the primary account for login/sync testing. If unavailable, defer real-account testing and use local/planner tests.
12. **Rename/move:** beta syncs them as delete + recreate; optimization is post-beta.
13. **Implementation gate:** a short Phase 0 spike must verify Drive SDK streaming, device creation, and event subscription inside the bundled app before M1.

---

## 15. Approval

This design is ready for approval. Once approved, create the new repository `Proton-Drive-Linux`, import the upstream base, and run the Phase 0 spike before starting M1.
