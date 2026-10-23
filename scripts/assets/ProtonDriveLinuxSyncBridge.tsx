import { useEffect, useRef } from 'react';

import { DeviceType, getDrive } from '@proton/drive';

import {
    DEFAULT_PLAN_OPTIONS,
    massDeleteGuard,
    planPath,
    type Action,
    type LocalSide,
    type RemoteSide,
    type StoredSide,
} from './sync-planner';

type TauriApi = {
    core?: {
        invoke<T = unknown>(command: string, args?: Record<string, unknown>): Promise<T>;
    };
    event?: {
        listen<T = unknown>(
            event: string,
            handler: (payload: { payload: T }) => void
        ): Promise<() => void>;
    };
};

type RootRecord = {
    localPath: string;
    deviceName?: string | null;
};

type LocalEntry = {
    relativePath: string;
    kind: 'file' | 'dir';
    size: number;
    mtimeNs: number;
};

type RemoteEntry = {
    uid: string;
    kind: 'file' | 'dir';
    size?: number;
    mtimeMs?: number;
};

type SyncItemRecord = {
    relativePath: string;
    localKind: 'file' | 'dir';
    localSize?: number | null;
    localMtimeNs?: number | null;
    remoteUid?: string | null;
    remoteKind?: 'file' | 'dir' | null;
    remoteSize?: number | null;
    remoteMtimeMs?: number | null;
    state: string;
    lastError?: string | null;
    updatedAtNs: number;
};

type UploadStart = { transferId: number; size: number; mtimeMs: number };
type DownloadStart = { transferId: number };

type RootInfo = { rootUid: string; scopeId: string; deviceName: string };
type SyncPhase = 'idle' | 'scanning' | 'syncing' | 'error' | 'paused';

type SyncStatus = {
    phase: SyncPhase;
    pending: number;
    lastError?: string;
    lastRunAt?: number;
};

const LOCAL_CHANGE_EVENT = 'sync://local-change';
const STATUS_EVENT = 'proton-sync-status';
const CHUNK_BYTES = 4 * 1024 * 1024;

const getTauri = (): TauriApi | undefined =>
    (window as unknown as { __TAURI__?: TauriApi }).__TAURI__;

const getDriveClient = (): any => getDrive() as any;

const base64ToBytes = (value: string): Uint8Array => {
    const binary = window.atob(value);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
        bytes[index] = binary.charCodeAt(index);
    }
    return bytes;
};

const bytesToBase64 = (bytes: Uint8Array): string => {
    let binary = '';
    const chunkSize = 0x8000;
    for (let offset = 0; offset < bytes.length; offset += chunkSize) {
        binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
    }
    return window.btoa(binary);
};

const nsToMs = (ns: number | null | undefined): number =>
    typeof ns === 'number' ? Math.round(ns / 1_000_000) : 0;

const dirname = (path: string): string => {
    const slash = path.lastIndexOf('/');
    return slash >= 0 ? path.slice(0, slash) : '';
};

const basename = (path: string): string => {
    const slash = path.lastIndexOf('/');
    return slash >= 0 ? path.slice(slash + 1) : path;
};

const depth = (path: string): number => path.split('/').filter(Boolean).length;

const toLocalSide = (entry: LocalEntry): LocalSide => ({
    kind: entry.kind,
    size: entry.size,
    mtimeMs: nsToMs(entry.mtimeNs),
});

const toRemoteSide = (entry: RemoteEntry): RemoteSide => ({
    uid: entry.uid,
    kind: entry.kind,
    size: entry.size,
    mtimeMs: entry.mtimeMs,
});

const toStoredSide = (item: SyncItemRecord): StoredSide => ({
    state: item.state,
    local:
        item.localKind && item.localSize !== null && item.localSize !== undefined
            ? {
                  kind: item.localKind,
                  size: item.localSize,
                  mtimeMs: nsToMs(item.localMtimeNs),
              }
            : undefined,
    remote:
        item.remoteUid && item.remoteKind
            ? {
                  uid: item.remoteUid,
                  kind: item.remoteKind,
                  size: item.remoteSize ?? undefined,
                  mtimeMs: item.remoteMtimeMs ?? undefined,
              }
            : undefined,
});

const makeItem = (
    path: string,
    local: LocalSide | undefined,
    remote: RemoteSide | undefined,
    state = 'synced'
): SyncItemRecord => ({
    relativePath: path,
    localKind: local?.kind ?? 'file',
    localSize: local?.size ?? null,
    localMtimeNs: local ? Math.round(local.mtimeMs * 1_000_000) : null,
    remoteUid: remote?.uid ?? null,
    remoteKind: remote?.kind ?? null,
    remoteSize: remote?.size ?? null,
    remoteMtimeMs: remote?.mtimeMs ?? null,
    state,
    lastError: null,
    updatedAtNs: Date.now() * 1_000_000,
});

export const ProtonDriveLinuxSyncBridge = () => {
    const running = useRef(false);
    const stopped = useRef(false);
    const syncing = useRef<Promise<void> | undefined>(undefined);
    const dirty = useRef(false);
    const debounce = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
    const status = useRef<SyncStatus>({ phase: 'idle', pending: 0 });
    const rootInfo = useRef<RootInfo | undefined>(undefined);
    const remoteFolders = useRef<Map<string, string>>(new Map());
    const subscription = useRef<{ dispose: () => void } | undefined>(undefined);
    const unlisten = useRef<(() => void) | undefined>(undefined);

    const emitStatus = (next: Partial<SyncStatus>) => {
        status.current = { ...status.current, ...next };
        window.dispatchEvent(new CustomEvent(STATUS_EVENT, { detail: status.current }));
    };

    const invoke = async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
        const tauri = getTauri();
        if (!tauri?.core) {
            throw new Error('Tauri API is unavailable');
        }
        return tauri.core.invoke<T>(command, args);
    };

    const resolveRoot = async (deviceName: string): Promise<RootInfo> => {
        const drive = getDriveClient();
        for await (const device of drive.iterateDevices()) {
            if (device.type !== DeviceType.Linux) {
                continue;
            }
            const name = device.name?.ok ? device.name.value : undefined;
            if (name === deviceName) {
                const node = await drive.getNode(device.rootFolderUid);
                return {
                    rootUid: device.rootFolderUid,
                    scopeId: node.treeEventScopeId,
                    deviceName,
                };
            }
        }
        const device = await drive.createDevice(deviceName, DeviceType.Linux);
        const node = await drive.getNode(device.rootFolderUid);
        return { rootUid: device.rootFolderUid, scopeId: node.treeEventScopeId, deviceName };
    };

    const scanRemote = async (rootUid: string): Promise<Map<string, RemoteEntry>> => {
        const drive = getDriveClient();
        const entries = new Map<string, RemoteEntry>();
        const folders = new Map<string, string>();

        const walk = async (parentUid: string, prefix: string) => {
            for await (const node of drive.iterateFolderChildren(parentUid)) {
                const name = node.name?.ok ? node.name.value : undefined;
                if (!name) {
                    continue;
                }
                const relativePath = prefix ? `${prefix}/${name}` : name;
                const kind = node.type === 'folder' ? 'dir' : 'file';
                const entry: RemoteEntry = {
                    uid: node.uid,
                    kind,
                    size: node.activeRevision?.claimedSize,
                    mtimeMs: node.activeRevision?.claimedModificationTime?.getTime(),
                };
                entries.set(relativePath, entry);
                if (kind === 'dir') {
                    folders.set(relativePath, node.uid);
                    await walk(node.uid, relativePath);
                }
            }
        };

        await walk(rootUid, '');
        remoteFolders.current = folders;
        return entries;
    };

    const ensureRemoteFolder = async (path: string): Promise<string> => {
        if (!rootInfo.current) {
            throw new Error('Sync root is not resolved');
        }
        if (!path) {
            return rootInfo.current.rootUid;
        }
        const existing = remoteFolders.current.get(path);
        if (existing) {
            return existing;
        }
        const parent = dirname(path);
        const parentUid = await ensureRemoteFolder(parent);
        const drive = getDriveClient();
        const node = await drive.createFolder(parentUid, basename(path));
        remoteFolders.current.set(path, node.uid);
        return node.uid;
    };

    const upload = async (
        path: string,
        local: LocalSide,
        existingUid?: string
    ): Promise<RemoteSide> => {
        const start = await invoke<UploadStart>('sync_begin_upload', { relativePath: path });
        const stream = new ReadableStream<Uint8Array>({
            async pull(controller) {
                const chunk = await invoke<string | null>('sync_read_upload_chunk', {
                    transferId: start.transferId,
                    maxBytes: CHUNK_BYTES,
                });
                if (chunk === null) {
                    controller.close();
                    return;
                }
                controller.enqueue(base64ToBytes(chunk));
            },
            async cancel() {
                await invoke('sync_close_upload', { transferId: start.transferId }).catch(() => undefined);
            },
        });

        const metadata = {
            mediaType: 'application/octet-stream',
            expectedSize: start.size,
            modificationTime: new Date(start.mtimeMs),
        };

        try {
            const drive = getDriveClient();
            const uploader = existingUid
                ? await drive.getFileRevisionUploader(existingUid, metadata)
                : await drive.getFileUploader(
                      await ensureRemoteFolder(dirname(path)),
                      basename(path),
                      metadata
                  );
            const controller = await uploader.uploadFromStream(stream, []);
            const result = await controller.completion();
            return {
                uid: result.nodeUid,
                kind: 'file',
                size: local.size,
                mtimeMs: local.mtimeMs,
            };
        } finally {
            await invoke('sync_close_upload', { transferId: start.transferId }).catch(() => undefined);
        }
    };

    const download = async (path: string, remote: RemoteSide): Promise<void> => {
        const start = await invoke<DownloadStart>('sync_begin_download', {
            relativePath: path,
            mtimeMs: remote.mtimeMs ?? 0,
            expectedSize: remote.size,
        });
        const writable = new WritableStream<Uint8Array>({
            async write(chunk) {
                await invoke('sync_write_download_chunk', {
                    transferId: start.transferId,
                    dataBase64: bytesToBase64(chunk),
                });
            },
        });

        try {
            const drive = getDriveClient();
            const downloader = await drive.getFileDownloader(remote.uid);
            const controller = downloader.downloadToStream(writable);
            await controller.completion();
            await invoke('sync_finish_download', { transferId: start.transferId });
            await invoke('sync_mark_suppressed', { relativePath: path }).catch(() => undefined);
        } catch (error) {
            await invoke('sync_abort_download', { transferId: start.transferId }).catch(() => undefined);
            throw error;
        }
    };

    const trashRemote = async (uid: string): Promise<void> => {
        const drive = getDriveClient();
        for await (const result of drive.trashNodes([uid])) {
            if (!result.ok) {
                throw result.error ?? new Error('Failed to trash remote node');
            }
        }
    };

    const saveState = async (
        path: string,
        local: LocalSide | undefined,
        remote: RemoteSide | undefined,
        state = 'synced'
    ) => {
        await invoke('sync_put_item', { item: makeItem(path, local, remote, state) });
    };

    const removeState = async (path: string) => {
        await invoke('sync_delete_item', { relativePath: path }).catch(() => undefined);
    };

    const execute = async (
        action: Action,
        localMap: Map<string, LocalEntry>,
        remoteMap: Map<string, RemoteEntry>
    ) => {
        switch (action.type) {
            case 'link_only':
                await saveState(
                    action.path,
                    localMap.has(action.path) ? toLocalSide(localMap.get(action.path)!) : undefined,
                    remoteMap.has(action.path) ? toRemoteSide(remoteMap.get(action.path)!) : undefined
                );
                return;
            case 'mkdir_remote': {
                const uid = await ensureRemoteFolder(action.path);
                const local = localMap.get(action.path);
                await saveState(
                    action.path,
                    local ? toLocalSide(local) : { kind: 'dir', size: 0, mtimeMs: 0 },
                    { uid, kind: 'dir' }
                );
                return;
            }
            case 'mkdir_local': {
                await invoke('sync_mkdir_local', { relativePath: action.path });
                const remote = remoteMap.get(action.path);
                await saveState(
                    action.path,
                    { kind: 'dir', size: 0, mtimeMs: 0 },
                    remote ? toRemoteSide(remote) : undefined
                );
                return;
            }
            case 'upload_new':
            case 'upload_revision': {
                const local = localMap.get(action.path);
                if (!local) {
                    throw new Error(`Local file disappeared: ${action.path}`);
                }
                const existingUid = action.type === 'upload_revision' ? action.remoteUid : undefined;
                const remote = await upload(action.path, toLocalSide(local), existingUid);
                await saveState(action.path, toLocalSide(local), remote);
                return;
            }
            case 'download_new':
            case 'download_update': {
                const remote = remoteMap.get(action.path);
                if (!remote) {
                    throw new Error(`Remote file disappeared: ${action.path}`);
                }
                await download(action.path, toRemoteSide(remote));
                await saveState(action.path, toLocalSide({
                    relativePath: action.path,
                    kind: 'file',
                    size: remote.size ?? 0,
                    mtimeNs: (remote.mtimeMs ?? 0) * 1_000_000,
                }), toRemoteSide(remote));
                return;
            }
            case 'download_conflict': {
                const remote = remoteMap.get(action.path);
                if (!remote) {
                    throw new Error(`Remote file disappeared: ${action.path}`);
                }
                await download(action.conflictPath, toRemoteSide(remote));
                return;
            }
            case 'trash_remote':
                await trashRemote(action.remoteUid);
                await removeState(action.path);
                return;
            case 'trash_local':
                await invoke('sync_trash_local', { relativePath: action.path });
                await removeState(action.path);
                return;
            case 'remove_state':
                await removeState(action.path);
                return;
            case 'hold':
                throw new Error(action.reason);
        }
    };

    const runCycle = async () => {
        const root = await invoke<RootRecord | null>('sync_get_root');
        if (!root) {
            emitStatus({ phase: 'paused', pending: 0 });
            return;
        }

        emitStatus({ phase: 'scanning', pending: 0, lastError: undefined });
        const deviceName = root.deviceName || (await invoke<string>('sync_get_device_name'));
        rootInfo.current = await resolveRoot(deviceName);
        await invoke('sync_start');

        const localEntries = await invoke<LocalEntry[]>('sync_scan_local');
        const items = await invoke<SyncItemRecord[]>('sync_load_items');
        const remoteEntries = await scanRemote(rootInfo.current.rootUid);

        const localMap = new Map(localEntries.map((entry) => [entry.relativePath, entry]));
        const remoteMap = remoteEntries;
        const storedMap = new Map(items.map((item) => [item.relativePath, item]));
        const paths = new Set<string>([...localMap.keys(), ...remoteMap.keys(), ...storedMap.keys()]);
        const sortedPaths = [...paths].sort((a, b) => depth(a) - depth(b) || a.localeCompare(b));

        let actions: Action[] = [];
        for (const path of sortedPaths) {
            const local = localMap.get(path);
            const remote = remoteMap.get(path);
            const stored = storedMap.get(path);
            actions.push(
                ...planPath(
                    path,
                    local ? toLocalSide(local) : undefined,
                    remote ? toRemoteSide(remote) : undefined,
                    stored ? toStoredSide(stored) : undefined,
                    DEFAULT_PLAN_OPTIONS
                )
            );
        }

        const deleteCount = actions.filter(
            (action) => action.type === 'trash_local' || action.type === 'trash_remote'
        ).length;
        const guard = massDeleteGuard({
            localCount: localEntries.length,
            remoteCount: remoteEntries.size,
            trackedCount: items.length,
            deleteCount,
        });
        if (guard) {
            actions = actions.map((action) =>
                action.type === 'trash_local' || action.type === 'trash_remote'
                    ? { type: 'hold', path: action.path, reason: guard }
                    : action
            );
        }

        const priority: Record<Action['type'], number> = {
            mkdir_remote: 0,
            mkdir_local: 0,
            upload_new: 1,
            upload_revision: 1,
            download_new: 1,
            download_update: 1,
            download_conflict: 1,
            link_only: 2,
            trash_remote: 3,
            trash_local: 3,
            remove_state: 4,
            hold: 5,
        };
        actions.sort(
            (a, b) =>
                priority[a.type] - priority[b.type] ||
                depth(a.path) - depth(b.path) ||
                a.path.localeCompare(b.path)
        );

        emitStatus({ phase: 'syncing', pending: actions.length });
        let firstError: string | undefined;
        for (const action of actions) {
            if (stopped.current) {
                break;
            }
            try {
                await execute(action, localMap, remoteMap);
            } catch (error) {
                const message = error instanceof Error ? error.message : String(error);
                firstError ||= message;
                console.error(`[LinuxSync] ${action.type} failed for ${action.path}: ${message}`);
            }
        }

        emitStatus({
            phase: firstError ? 'error' : 'idle',
            pending: 0,
            lastError: firstError,
            lastRunAt: Date.now(),
        });
    };

    const syncNow = async () => {
        if (!running.current || stopped.current) {
            return;
        }
        if (syncing.current) {
            dirty.current = true;
            return syncing.current;
        }
        syncing.current = runCycle()
            .catch((error) => {
                emitStatus({
                    phase: 'error',
                    pending: 0,
                    lastError: error instanceof Error ? error.message : String(error),
                });
            })
            .finally(() => {
                syncing.current = undefined;
                if (dirty.current) {
                    dirty.current = false;
                    void schedule(0);
                }
            });
        return syncing.current;
    };

    const schedule = (delay = 1500) => {
        if (!running.current || stopped.current) {
            return;
        }
        if (debounce.current) {
            clearTimeout(debounce.current);
        }
        debounce.current = setTimeout(() => {
            debounce.current = undefined;
            void syncNow();
        }, delay);
    };

    const stopEngine = () => {
        stopped.current = true;
        running.current = false;
        subscription.current?.dispose();
        subscription.current = undefined;
        unlisten.current?.();
        unlisten.current = undefined;
    };

    const start = async () => {
        if (running.current) {
            return;
        }
        const root = await invoke<RootRecord | null>('sync_get_root');
        if (!root) {
            emitStatus({ phase: 'paused', pending: 0 });
            return;
        }
        running.current = true;
        stopped.current = false;
        try {
            const deviceName = root.deviceName || (await invoke<string>('sync_get_device_name'));
            rootInfo.current = await resolveRoot(deviceName);
            const drive = getDriveClient();
            subscription.current = await drive.subscribeToTreeEvents(
                rootInfo.current.scopeId,
                () => {
                    schedule(1500);
                }
            );
            const tauri = getTauri();
            unlisten.current = tauri?.event
                ? await tauri.event.listen(LOCAL_CHANGE_EVENT, () => schedule(1500))
                : undefined;

            (window as unknown as { __PROTON_DRIVE_LINUX_SYNC__?: unknown }).__PROTON_DRIVE_LINUX_SYNC__ = {
                start,
                syncNow,
                status: () => status.current,
                stop: stopEngine,
            };

            await syncNow();
        } catch (error) {
            running.current = false;
            stopEngine();
            throw error;
        }
    };

    useEffect(() => {
        const tauri = getTauri();
        if (!tauri?.core) {
            return;
        }

        const timer = setInterval(() => {
            if (!running.current) {
                void start().catch((error) => {
                    console.error('[LinuxSync] start failed', error);
                });
            }
        }, 2000);

        (window as unknown as { __PROTON_DRIVE_LINUX_SYNC__?: unknown }).__PROTON_DRIVE_LINUX_SYNC__ = {
            start,
            syncNow,
            status: () => status.current,
            stop: stopEngine,
        };

        void start().catch((error) => {
            console.error('[LinuxSync] initial start failed', error);
        });

        return () => {
            clearInterval(timer);
            if (debounce.current) {
                clearTimeout(debounce.current);
            }
            stopEngine();
        };
    }, []);

    return null;
};
