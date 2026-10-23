import { useEffect, useState } from 'react';

type TauriApi = {
    core?: {
        invoke<T = unknown>(command: string, args?: Record<string, unknown>): Promise<T>;
    };
};

type RootRecord = {
    localPath: string;
    deviceName?: string | null;
};

type SyncStatus = {
    active: boolean;
    rootPath?: string | null;
    itemCount: number;
    lastError?: string | null;
};

type BridgeApi = {
    start: () => Promise<void>;
    syncNow: () => Promise<void> | undefined;
    stop: () => void;
    status: () => { phase: string; pending: number; lastError?: string };
};

const getTauri = (): TauriApi | undefined =>
    (window as unknown as { __TAURI__?: TauriApi }).__TAURI__;

const getBridge = (): BridgeApi | undefined =>
    (window as unknown as { __PROTON_DRIVE_LINUX_SYNC__?: BridgeApi }).__PROTON_DRIVE_LINUX_SYNC__;

const invoke = async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
    const tauri = getTauri();
    if (!tauri?.core) {
        throw new Error('Tauri API is unavailable');
    }
    return tauri.core.invoke<T>(command, args);
};

const phaseColor = (phase: string): string => {
    switch (phase) {
        case 'idle':
            return '#21a179';
        case 'syncing':
        case 'scanning':
            return '#6d4aff';
        case 'error':
            return '#d93025';
        default:
            return '#8a8f98';
    }
};

export const DriveLinuxPanel = () => {
    const [open, setOpen] = useState(false);
    const [root, setRoot] = useState<RootRecord | undefined>(undefined);
    const [folder, setFolder] = useState('');
    const [deviceName, setDeviceName] = useState('Linux-PC');
    const [status, setStatus] = useState<SyncStatus>({
        active: false,
        itemCount: 0,
    });
    const [phase, setPhase] = useState('idle');
    const [pending, setPending] = useState(0);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | undefined>(undefined);

    const refresh = async () => {
        const [syncRoot, syncStatus] = await Promise.all([
            invoke<RootRecord | null>('sync_get_root'),
            invoke<SyncStatus>('sync_get_status'),
        ]);
        setRoot(syncRoot ?? undefined);
        setStatus(syncStatus);
        if (syncRoot?.localPath && !folder) {
            setFolder(syncRoot.localPath);
        }
        if (syncRoot?.deviceName) {
            setDeviceName(syncRoot.deviceName);
        }
    };

    useEffect(() => {
        const onStatus = (event: Event) => {
            const detail = (event as CustomEvent<{ phase: string; pending: number; lastError?: string }>)
                .detail;
            if (!detail) {
                return;
            }
            setPhase(detail.phase);
            setPending(detail.pending);
            setError(detail.lastError);
        };
        window.addEventListener('proton-sync-status', onStatus);

        void (async () => {
            try {
                const [defaultFolder, name] = await Promise.all([
                    invoke<string>('sync_default_root'),
                    invoke<string>('sync_get_device_name'),
                ]);
                setFolder(defaultFolder);
                setDeviceName(name);
                await refresh();
            } catch (err) {
                setError(err instanceof Error ? err.message : String(err));
            }
        })();

        const timer = setInterval(() => {
            void refresh().catch(() => undefined);
        }, 3000);

        return () => {
            window.removeEventListener('proton-sync-status', onStatus);
            clearInterval(timer);
        };
    }, []);

    const useFolder = async () => {
        setBusy(true);
        setError(undefined);
        try {
            const bridge = getBridge();
            if (bridge) {
                bridge.stop();
            }
            const saved = await invoke<RootRecord>('sync_set_root', {
                path: folder,
                deviceName,
            });
            setRoot(saved);
            await invoke('sync_start');
            await getBridge()?.start();
            await refresh();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
        } finally {
            setBusy(false);
        }
    };

    const start = async () => {
        setBusy(true);
        setError(undefined);
        try {
            if (!root) {
                await useFolder();
                return;
            }
            await invoke('sync_start');
            await getBridge()?.start();
            await refresh();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
        } finally {
            setBusy(false);
        }
    };

    const pause = async () => {
        setBusy(true);
        setError(undefined);
        try {
            getBridge()?.stop();
            await invoke('sync_stop');
            await refresh();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
        } finally {
            setBusy(false);
        }
    };

    const syncNow = async () => {
        setError(undefined);
        try {
            await getBridge()?.syncNow();
            await refresh();
        } catch (err) {
            setError(err instanceof Error ? err.message : String(err));
        }
    };

    if (!open) {
        return (
            <button
                type="button"
                onClick={() => setOpen(true)}
                style={{
                    position: 'fixed',
                    right: 18,
                    bottom: 18,
                    zIndex: 10000,
                    border: 0,
                    borderRadius: 999,
                    padding: '10px 16px',
                    background: '#6d4aff',
                    color: '#fff',
                    fontWeight: 600,
                    cursor: 'pointer',
                    boxShadow: '0 6px 20px rgba(0,0,0,.25)',
                }}
            >
                Linux Sync
            </button>
        );
    }

    return (
        <div
            style={{
                position: 'fixed',
                right: 18,
                bottom: 18,
                zIndex: 10000,
                width: 340,
                maxWidth: 'calc(100vw - 36px)',
                padding: 16,
                borderRadius: 14,
                background: '#1b1b1f',
                color: '#f5f5f7',
                boxShadow: '0 12px 40px rgba(0,0,0,.45)',
                font: '13px/1.4 system-ui, sans-serif',
            }}
        >
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                <strong>Linux Sync</strong>
                <button
                    type="button"
                    onClick={() => setOpen(false)}
                    style={{ background: 'transparent', color: '#bbb', border: 0, cursor: 'pointer' }}
                >
                    Close
                </button>
            </div>

            <div style={{ marginTop: 10, color: '#f0b95b' }}>
                Unofficial client. Do not use your primary Proton account for beta testing.
            </div>

            <label style={{ display: 'block', marginTop: 12, color: '#bbb' }}>
                Local folder
                <input
                    value={folder}
                    onChange={(event) => setFolder(event.target.value)}
                    spellCheck={false}
                    style={{
                        boxSizing: 'border-box',
                        width: '100%',
                        marginTop: 4,
                        padding: '7px 9px',
                        borderRadius: 8,
                        border: '1px solid #444',
                        background: '#111',
                        color: '#fff',
                    }}
                />
            </label>

            <label style={{ display: 'block', marginTop: 8, color: '#bbb' }}>
                Device name
                <input
                    value={deviceName}
                    onChange={(event) => setDeviceName(event.target.value)}
                    spellCheck={false}
                    style={{
                        boxSizing: 'border-box',
                        width: '100%',
                        marginTop: 4,
                        padding: '7px 9px',
                        borderRadius: 8,
                        border: '1px solid #444',
                        background: '#111',
                        color: '#fff',
                    }}
                />
            </label>

            <div style={{ marginTop: 8 }}>
                <button
                    type="button"
                    onClick={() => void useFolder()}
                    disabled={busy || !folder.trim()}
                    style={{ marginRight: 6, padding: '7px 10px' }}
                >
                    Use folder
                </button>
                <button
                    type="button"
                    onClick={() => void start()}
                    disabled={busy}
                    style={{ marginRight: 6, padding: '7px 10px' }}
                >
                    Start
                </button>
                <button
                    type="button"
                    onClick={() => void pause()}
                    disabled={busy || !status.active}
                    style={{ marginRight: 6, padding: '7px 10px' }}
                >
                    Pause
                </button>
                <button type="button" onClick={() => void syncNow()} style={{ padding: '7px 10px' }}>
                    Sync now
                </button>
            </div>

            <div style={{ marginTop: 12, display: 'flex', alignItems: 'center', gap: 8 }}>
                <span
                    style={{
                        width: 9,
                        height: 9,
                        borderRadius: 999,
                        background: phaseColor(phase),
                        display: 'inline-block',
                    }}
                />
                <span>
                    {phase}
                    {pending > 0 ? ` · ${pending} pending` : ''}
                </span>
                <span style={{ marginLeft: 'auto', color: '#aaa' }}>
                    {status.itemCount} items
                </span>
            </div>

            {root?.localPath && (
                <div style={{ marginTop: 6, color: '#9a9a9a', overflowWrap: 'anywhere' }}>
                    {root.localPath}
                </div>
            )}

            {error && (
                <div style={{ marginTop: 8, color: '#ff8a80', overflowWrap: 'anywhere' }}>
                    {error}
                </div>
            )}

            <div style={{ marginTop: 10, color: '#888', fontSize: 12 }}>
                Account limited? Stop retrying and use Proton&apos;s appeal form.{' '}
                <a
                    href="https://proton.me/support/appeal-abuse"
                    target="_blank"
                    rel="noreferrer"
                    style={{ color: '#9b8cff' }}
                >
                    Open support
                </a>
            </div>
        </div>
    );
};
