# Proton Drive for Linux (Unofficial) — Web Edition

A fast, web-based Proton Drive desktop client for Linux with two-way folder
sync. This is the **no-CLI** edition. The existing
[Proton-Drive-Linux-CLI](https://github.com/zhafribs/Proton-Drive-Linux-Cli)
project remains separate and unchanged.

> **Unofficial.** This project is not affiliated with Proton AG. Proton Drive
> is a trademark of Proton AG.

## What it is

This app wraps Proton's own Drive web application in a Tauri/WebKitGTK window.
Login, CAPTCHA, 2FA, browsing, previews, and Proton's cryptography are handled
by the bundled web app. The native layer adds:

- one local folder synced to one Linux device under Proton Drive **Computers**
- two-way create/modify/delete sync
- trash-based deletes (never permanent)
- conflict copies that keep both versions
- streamed large-file transfers
- a native Linux Sync panel and tray controls

The app does **not** call the official `proton-drive` CLI and does not use the
CLI project's sync engine.

## Status

Beta `0.1.0`. AppImage only, Linux x86_64.

Expected beta limitations:

- sync runs while the app is running or in the tray; it does not run headless
- rename/move is synced as delete + recreate; the old version stays in trash
- calendar is not included
- Proton WebClients are pinned and must be rebuilt for web-app updates

## Requirements

- Linux x86_64
- WebKitGTK 4.1
- GTK 3
- GStreamer base/good plugins
- a desktop with tray support (for background sync)

## Build

```bash
npm ci
npm run build:appimage
```

The resulting AppImage is under `src-tauri/target/release/bundle/appimage/`.

The build fetches the pinned Proton WebClients checkout, applies the local
patches, builds Drive + Account + Verify, and then packages the Tauri app.
Calendar is intentionally excluded.

## License and attribution

AGPL-3.0-or-later. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

This project is derived from
[DonnieDice/protondrive-linux](https://github.com/DonnieDice/protondrive-linux)
at commit `0b5dd592de297faddb865872907d045d47ee24e4`. Upstream copyright and
history are preserved.

It bundles Proton WebClients, which are licensed under AGPL-3.0. The exact
WebClients source commit and local patches are recorded in this repository so
the corresponding source can be rebuilt.

## Documentation

- [Design](docs/DESIGN.md)
- [Build](docs/BUILD.md)
- [Security](docs/SECURITY.md)
- [Contributing](docs/CONTRIBUTING.md)
- [Implementation plan](docs/superpowers/plans/2026-10-24-proton-drive-linux.md)
