# Official webview + CLI sync (option B)

## Result

The spike is viable.

- Tauri loads the official Proton Drive web app directly from
  `https://drive.proton.me/`.
- Proton's normal login page is shown; no custom auth proxy is involved.
- A Tauri-injected panel is visible on the Proton page.
- The panel can call a Tauri command from the remote Proton origin.
- The command successfully found and executed the official Proton Drive CLI.

Verified locally with:

- `PROTON_DRIVE_OFFICIAL_WEB=1`
- `PROTON_DRIVE_BIN=/path/to/proton-drive`
- official CLI `cli-drive@0.9.0`

The panel now also exposes CLI login, engine start/stop, sync-now, folder
selection, and health status. The engine is the mature CLI-backed engine from
`Proton-Drive-Linux-Cli`; it is embedded in the binary and runs from the app
data directory.

## Architecture

- **Login/UI:** official `drive.proton.me` web app loaded in a Tauri webview.
- **Remote sync operations:** official Proton Drive CLI.
- **Sync engine:** reuse the mature CLI-backed engine from
  `Proton-Drive-Linux-CLI` (MIT) rather than injecting CLI tokens into the
  bundled web app.
- **Local state:** separate state directory so the old app's state is not
  reused accidentally.
- **Scope:** `/my-files` for the first hybrid beta. Computers/devices can be
  added later if the CLI supports them.
- **Panel:** injected into the official page; controls CLI login, engine
  start/stop, sync now, folder selection, and status.

## Security

- Remote Proton origins get only minimal IPC permissions.
- The official webview can call a small set of app commands; each command
  checks the calling origin.
- No Proton password is stored by this app.
- CLI login is Proton's own flow and stores the session in the OS keyring.
- No custom auth proxy, no bundled web-app login, no token injection.

## Why not CLI-token injection into the bundled app

The CLI keyring stores an API session. The bundled Drive web app additionally
needs browser cookies and the `ps-*` web session key. Injecting tokens would be
fragile and would still use the unusual proxy path that likely triggered the
account restriction.

## First acceptance target

1. Official Proton login works in the Tauri window.
2. CLI login works and status is detected.
3. A disposable local folder syncs to `/my-files`.
4. Upload, download, delete-to-trash, conflict, restart, and stop/start work.
5. Only then consider using a real account/folder.
