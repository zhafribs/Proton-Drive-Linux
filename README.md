# Proton Drive for Linux (Unofficial) — Web Edition

A web-based Proton Drive desktop client for Linux with two-way folder sync.

This is the **option B** edition: it uses Proton's official web app for login
and browsing, and the official Proton Drive CLI for sync. It does **not** use
the custom auth proxy that triggered Proton account restrictions in the
bundled-web-app experiment.

> **Unofficial.** This project is not affiliated with Proton AG. Proton Drive
> is a trademark of Proton AG.

> **Account safety:** never test a new build against your primary account.
> Use a separate Proton account and a disposable local/remote folder. See
> [Account safety](docs/ACCOUNT-SAFETY.md).

## What it is

- **UI/login:** official `https://drive.proton.me/` loaded directly in Tauri.
  Proton's normal web login, CAPTCHA, 2FA, browsing, and previews are used.
- **Sync:** the official `proton-drive` CLI, driven by the mature CLI-backed
  sync engine from `Proton-Drive-Linux-Cli`.
- **Panel:** a small injected Linux Sync panel for CLI login, start/stop,
  sync-now, folder selection, and health.
- **Scope:** one local folder ↔ one remote folder (default
  `/my-files/ProtonDrive-Linux`).
- **Safety:** trash-based deletes, conflict handling, and mass-delete guards
  come from the CLI sync engine.

The app still supports a bundled-web fallback for development, but the default
is the official webview.

## Status

Beta `0.1.0`. AppImage, Linux x86_64.

Current status:

- official Proton Drive webview login: verified
- injected panel + Tauri IPC on the Proton page: verified
- official CLI detection from the panel: verified
- CLI login / sync engine wiring: implemented, awaiting a live test with a
  disposable account/folder
- no custom auth proxy in the default path

Expected limitations:

- the CLI must be installed and signed in for sync
- Python 3 is required locally during development; release packaging will
  bundle the runtime
- sync runs while the app is running
- rename/move and conflict behavior are inherited from the CLI engine
- the bundled WebClients fallback is still built for now, so the local AppImage
  is larger than necessary

## Requirements

- Linux x86_64
- WebKitGTK 4.1, GTK 3, GStreamer base/good
- official `proton-drive` CLI (installed locally for now)
- Python 3 for the sync engine (development; to be bundled)

## Build

```bash
npm ci
npm run build:appimage
```

The resulting AppImage is under `src-tauri/target/release/bundle/appimage/`.

The current build still fetches/builds Proton WebClients for the fallback mode.
The default runtime path is the official remote webview.

## Usage

1. Launch the AppImage.
2. Sign in with Proton's official web login.
3. Click **CLI login** in the Linux Sync panel and complete the official CLI
   browser flow.
4. Set a disposable local folder and remote folder for the first test.
5. Click **Start sync**.
6. Use **Sync now** to request an immediate cycle.
7. Check status/health in the panel.

For the first test use something like:

- local: `/tmp/protondrive-test`
- remote: `/my-files/ProtonDrive-Test`

Do not point the first test at real documents.

## License and attribution

AGPL-3.0-or-later. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

The project is derived from
[DonnieDice/protondrive-linux](https://github.com/DonnieDice/protondrive-linux)
at commit `0b5dd592de297faddb865872907d045d47ee24e4`.

The CLI sync engine is reused from
[Proton-Drive-Linux-Cli](https://github.com/zhafribs/Proton-Drive-Linux-Cli)
under the MIT license. See `NOTICE`.

## Documentation

- [Official-web spike notes](docs/OFFICIAL-WEB-SPIKE.md)
- [Account safety](docs/ACCOUNT-SAFETY.md)
- [Design](docs/DESIGN.md)
- [Build](docs/BUILD.md)
- [Security](docs/SECURITY.md)
- [Implementation plan](docs/superpowers/plans/2026-10-24-proton-drive-linux.md)
