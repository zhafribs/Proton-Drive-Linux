# Building Proton Drive for Linux

This build produces an AppImage only for the first beta.

## Requirements

- Linux x86_64
- Node.js 20 or newer
- Rust stable (install with rustup)
- WebKitGTK 4.1 development files
- GTK 3 development files
- GStreamer base/good plugins
- `pkg-config`
- `patchelf`
- `fuse`/`libfuse2` for running AppImages
- `libayatana-appindicator` or equivalent when tray support is enabled

On Arch-based systems the core build packages are:

```bash
sudo pacman -S --needed base-devel rustup nodejs npm pkgconf \
  webkit2gtk-4.1 gtk3 gstreamer gst-plugins-base gst-plugins-good \
  libayatana-appindicator patchelf fuse2
rustup default stable
```

## Build

```bash
npm ci
npm run build:appimage
```

On rolling distributions, linuxdeploy's bundled `strip` may be too old for
libraries containing `.relr.dyn`. If the build fails with
`unknown type [0x13] section '.relr.dyn'`, build with:

```bash
NO_STRIP=1 npm run build:appimage
```

This skips stripping and produces a larger AppImage. Release builds should use
a stable Debian/Ubuntu build environment where stripping works.

The build:

1. clones Proton WebClients at the commit in `WebClients.lock`
2. applies the local WebClients patches
3. builds Drive, Account, and Verify; Calendar is excluded
4. builds the Rust/Tauri shell
5. writes the AppImage to `src-tauri/target/release/bundle/appimage/`

## Development

```bash
npm run dev
```

`npm run dev` still expects the WebClients dist directory. Run
`npm run build:web` once first.

## Notes

- Do not build against an unpinned WebClients `main`; update
  `WebClients.lock` deliberately.
- The build is memory-heavy. Build the three web applications sequentially if
  the machine is low on RAM.
- The app is AGPL-3.0-or-later. Keep `NOTICE`, the upstream attribution, and
  the WebClients source link when redistributing.
