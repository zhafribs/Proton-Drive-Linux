#!/usr/bin/env python3
"""Install the Proton Drive Linux sync bridge into the bundled WebClients app."""
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
WEBCLIENTS_DIR = REPO_ROOT / "WebClients"
DRIVE_APP_DIR = WEBCLIENTS_DIR / "applications/drive/src/app"
ASSETS_DIR = REPO_ROOT / "scripts/assets"
BRIDGE_FILENAME = "ProtonDriveLinuxSyncBridge.tsx"
PLANNER_FILENAME = "sync-planner.ts"
PANEL_FILENAME = "DriveLinuxPanel.tsx"


def fail(message: str) -> None:
    raise SystemExit(f"❌ {message}")


def find_drive_provider() -> Path:
    for path in DRIVE_APP_DIR.rglob("DriveProvider.tsx"):
        source = path.read_text(encoding="utf-8")
        if "export function DriveProvider" in source and "<UploadProvider>" in source:
            return path
    fail("Unable to find DriveProvider.tsx with UploadProvider in current WebClients layout")


def patch_drive_provider(path: Path) -> None:
    source = path.read_text(encoding="utf-8")
    import_anchor = "import { PublicSessionProvider } from './_api';\n"
    if import_anchor not in source:
        fail("DriveProvider import anchor changed; cannot inject sync bridge")
    if "ProtonDriveLinuxSyncBridge" not in source:
        source = source.replace(
            import_anchor,
            import_anchor
            + "import { ProtonDriveLinuxSyncBridge } from './ProtonDriveLinuxSyncBridge';\n",
            1,
        )
    if "DriveLinuxPanel" not in source:
        source = source.replace(
            "import { ProtonDriveLinuxSyncBridge } from './ProtonDriveLinuxSyncBridge';\n",
            "import { ProtonDriveLinuxSyncBridge } from './ProtonDriveLinuxSyncBridge';\n"
            "import { DriveLinuxPanel } from './DriveLinuxPanel';\n",
            1,
        )

    mount_anchor = "                                <UploadProvider>\n                                    <SearchProvider>"
    if "<ProtonDriveLinuxSyncBridge />" not in source:
        if mount_anchor not in source:
            fail("DriveProvider mount anchor changed; cannot inject sync bridge")
        source = source.replace(
            mount_anchor,
            "                                <UploadProvider>\n"
            "                                    <ProtonDriveLinuxSyncBridge />\n"
            "                                    <SearchProvider>",
            1,
        )
    if "<DriveLinuxPanel />" not in source:
        source = source.replace(
            "                                    <ProtonDriveLinuxSyncBridge />\n",
            "                                    <ProtonDriveLinuxSyncBridge />\n"
            "                                    <DriveLinuxPanel />\n",
            1,
        )
    path.write_text(source, encoding="utf-8")


def main() -> None:
    if not WEBCLIENTS_DIR.exists():
        fail("WebClients directory is missing")
    for name in (BRIDGE_FILENAME, PLANNER_FILENAME, PANEL_FILENAME):
        source = ASSETS_DIR / name
        if not source.is_file():
            fail(f"Missing sync asset: {source}")

    provider_path = find_drive_provider()
    target_dir = provider_path.parent
    for name in (BRIDGE_FILENAME, PLANNER_FILENAME, PANEL_FILENAME):
        (target_dir / name).write_text(
            (ASSETS_DIR / name).read_text(encoding="utf-8"), encoding="utf-8"
        )

    patch_drive_provider(provider_path)
    print(f"  ✓ Installed sync bridge at {target_dir.relative_to(WEBCLIENTS_DIR)}")


if __name__ == "__main__":
    main()
