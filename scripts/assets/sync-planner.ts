export type SideKind = 'file' | 'dir';

export type LocalSide = {
    kind: SideKind;
    size: number;
    mtimeMs: number;
};

export type RemoteSide = {
    uid: string;
    kind: SideKind;
    size?: number;
    mtimeMs?: number;
};

export type StoredSide = {
    state: string;
    local?: LocalSide;
    remote?: RemoteSide;
};

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

export type PlanOptions = {
    deleteEnabled: boolean;
    mtimeToleranceMs: number;
    now?: Date;
};

export const DEFAULT_PLAN_OPTIONS: PlanOptions = {
    deleteEnabled: true,
    mtimeToleranceMs: 2000,
};

export function planPath(
    path: string,
    local: LocalSide | undefined,
    remote: RemoteSide | undefined,
    stored: StoredSide | undefined,
    options: PlanOptions = DEFAULT_PLAN_OPTIONS
): Action[] {
    if (!local && !remote) {
        return stored ? [{ type: 'remove_state', path }] : [];
    }

    if (local && !remote) {
        if (!stored) {
            return local.kind === 'dir'
                ? [{ type: 'mkdir_remote', path }]
                : [{ type: 'upload_new', path }];
        }
        // Directory deletion is handled conservatively for beta: keep the
        // local directory and recreate the remote parent rather than moving a
        // whole subtree to trash based on an empty remote listing.
        if (local.kind === 'dir') {
            return [{ type: 'mkdir_remote', path }];
        }
        const localChanged = hasLocalChanged(local, stored, options);
        if (localChanged) {
            return [{ type: 'upload_new', path }];
        }
        return options.deleteEnabled
            ? [{ type: 'trash_local', path }]
            : [{ type: 'hold', path, reason: 'remote deleted while deletes are disabled' }];
    }

    if (!local && remote) {
        if (!stored) {
            return remote.kind === 'dir'
                ? [{ type: 'mkdir_local', path }]
                : [{ type: 'download_new', path, remoteUid: remote.uid }];
        }
        const remoteChanged = hasRemoteChanged(remote, stored, options);
        if (remote.kind === 'dir') {
            if (remoteChanged) {
                return [{ type: 'mkdir_local', path }];
            }
            return options.deleteEnabled
                ? [{ type: 'trash_remote', path, remoteUid: remote.uid }]
                : [{ type: 'hold', path, reason: 'local deleted while deletes are disabled' }];
        }
        if (remoteChanged) {
            return [{ type: 'download_new', path, remoteUid: remote.uid }];
        }
        return options.deleteEnabled
            ? [{ type: 'trash_remote', path, remoteUid: remote.uid }]
            : [{ type: 'hold', path, reason: 'local deleted while deletes are disabled' }];
    }

    if (local.kind !== remote.kind) {
        return [{ type: 'hold', path, reason: 'local and remote types differ' }];
    }

    if (!stored) {
        if (local.kind === 'dir') {
            return [{ type: 'link_only', path }];
        }
        if (sameFile(local, remote, options.mtimeToleranceMs)) {
            return [{ type: 'link_only', path }];
        }
        return conflictActions(path, remote, 'upload_new', options.now);
    }

    const localChanged = hasLocalChanged(local, stored, options);
    const remoteChanged = hasRemoteChanged(remote, stored, options);

    if (!localChanged && !remoteChanged) {
        return [];
    }
    if (local.kind === 'dir') {
        return [];
    }
    if (localChanged && !remoteChanged) {
        return [{ type: 'upload_revision', path, remoteUid: remote.uid }];
    }
    if (!localChanged && remoteChanged) {
        return [{ type: 'download_update', path, remoteUid: remote.uid }];
    }
    return conflictActions(path, remote, 'upload_revision', options.now);
}

function conflictActions(
    path: string,
    remote: RemoteSide,
    uploadType: 'upload_new' | 'upload_revision',
    now: Date = new Date()
): Action[] {
    const conflictPath = conflictName(path, now);
    const upload: Action =
        uploadType === 'upload_revision'
            ? { type: 'upload_revision', path, remoteUid: remote.uid }
            : { type: 'upload_new', path };
    return [
        { type: 'download_conflict', path, remoteUid: remote.uid, conflictPath },
        upload,
    ];
}

function sameFile(local: LocalSide, remote: RemoteSide, toleranceMs: number): boolean {
    if (local.kind !== 'file' || remote.kind !== 'file') {
        return false;
    }
    if (local.size !== remote.size) {
        return false;
    }
    if (remote.mtimeMs === undefined) {
        return true;
    }
    return Math.abs(local.mtimeMs - remote.mtimeMs) <= toleranceMs;
}

function hasLocalChanged(
    local: LocalSide,
    stored: StoredSide,
    options: PlanOptions
): boolean {
    if (!stored.local) {
        return true;
    }
    if (local.kind !== stored.local.kind) {
        return true;
    }
    if (local.kind === 'dir') {
        return false;
    }
    return (
        local.size !== stored.local.size ||
        Math.abs(local.mtimeMs - stored.local.mtimeMs) > options.mtimeToleranceMs
    );
}

function hasRemoteChanged(
    remote: RemoteSide,
    stored: StoredSide,
    options: PlanOptions
): boolean {
    if (!stored.remote) {
        return true;
    }
    if (remote.uid !== stored.remote.uid || remote.kind !== stored.remote.kind) {
        return true;
    }
    if (remote.kind === 'dir') {
        return false;
    }
    if (remote.size !== stored.remote.size) {
        return true;
    }
    if (remote.mtimeMs === undefined || stored.remote.mtimeMs === undefined) {
        return false;
    }
    return Math.abs(remote.mtimeMs - stored.remote.mtimeMs) > options.mtimeToleranceMs;
}

export function conflictName(path: string, now: Date): string {
    const slash = path.lastIndexOf('/');
    const directory = slash >= 0 ? path.slice(0, slash + 1) : '';
    const fileName = slash >= 0 ? path.slice(slash + 1) : path;
    const dot = fileName.lastIndexOf('.');
    const hasExtension = dot > 0;
    const base = hasExtension ? fileName.slice(0, dot) : fileName;
    const extension = hasExtension ? fileName.slice(dot) : '';
    const stamp = [
        now.getUTCFullYear(),
        String(now.getUTCMonth() + 1).padStart(2, '0'),
        String(now.getUTCDate()).padStart(2, '0'),
        String(now.getUTCHours()).padStart(2, '0'),
        String(now.getUTCMinutes()).padStart(2, '0'),
        String(now.getUTCSeconds()).padStart(2, '0'),
    ];
    const stampText = `${stamp[0]}-${stamp[1]}-${stamp[2]} ${stamp[3]}${stamp[4]}${stamp[5]}`;
    return `${directory}${base} (conflict ${stampText})${extension}`;
}

export type MassDeleteGuardInput = {
    localCount: number;
    remoteCount: number;
    trackedCount: number;
    deleteCount: number;
    thresholdCount?: number;
    thresholdRatio?: number;
};

/**
 * Returns a hold reason when a sync cycle looks like it is about to delete far
 * too much, or when one side unexpectedly looks empty.
 */
export function massDeleteGuard(input: MassDeleteGuardInput): string | null {
    if (input.deleteCount === 0) {
        return null;
    }
    if (input.trackedCount > 0 && input.localCount === 0) {
        return 'local tree is empty but sync state has tracked items';
    }
    if (input.trackedCount > 0 && input.remoteCount === 0) {
        return 'remote tree is empty but sync state has tracked items';
    }
    const thresholdCount = input.thresholdCount ?? 10;
    const thresholdRatio = input.thresholdRatio ?? 0.25;
    if (
        input.deleteCount > thresholdCount &&
        input.deleteCount / Math.max(input.trackedCount, 1) > thresholdRatio
    ) {
        return 'mass delete threshold exceeded';
    }
    return null;
}
