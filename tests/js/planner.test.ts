import { describe, expect, it } from 'vitest';

import {
    conflictName,
    massDeleteGuard,
    planPath,
    type LocalSide,
    type RemoteSide,
    type StoredSide,
} from '../../scripts/assets/sync-planner';

const now = new Date('2026-10-24T14:30:12Z');

function localFile(size = 10, mtimeMs = 1_000): LocalSide {
    return { kind: 'file', size, mtimeMs };
}

function remoteFile(uid = 'remote-1', size = 10, mtimeMs = 1_000): RemoteSide {
    return { uid, kind: 'file', size, mtimeMs };
}

function localDir(): LocalSide {
    return { kind: 'dir', size: 0, mtimeMs: 0 };
}

function remoteDir(uid = 'remote-dir'): RemoteSide {
    return { uid, kind: 'dir' };
}

function stored(
    local: LocalSide | undefined,
    remote: RemoteSide | undefined,
    state = 'synced'
): StoredSide {
    return { state, local, remote };
}

const options = { deleteEnabled: true, mtimeToleranceMs: 2_000, now };

describe('planPath', () => {
    it('uploads a local-only file', () => {
        expect(planPath('a.txt', localFile(), undefined, undefined, options)).toEqual([
            { type: 'upload_new', path: 'a.txt' },
        ]);
    });

    it('downloads a remote-only file', () => {
        expect(planPath('a.txt', undefined, remoteFile(), undefined, options)).toEqual([
            { type: 'download_new', path: 'a.txt', remoteUid: 'remote-1' },
        ]);
    });

    it('links a first-sync file with matching size and mtime', () => {
        expect(planPath('a.txt', localFile(), remoteFile(), undefined, options)).toEqual([
            { type: 'link_only', path: 'a.txt' },
        ]);
    });

    it('treats mtime within tolerance as unchanged', () => {
        expect(
            planPath('a.txt', localFile(10, 1_000), remoteFile('r', 10, 2_500), undefined, options)
        ).toEqual([{ type: 'link_only', path: 'a.txt' }]);
    });

    it('keeps both versions on a first-sync conflict', () => {
        expect(
            planPath('dir/file.txt', localFile(10, 1_000), remoteFile('r', 20, 1_000), undefined, options)
        ).toEqual([
            {
                type: 'download_conflict',
                path: 'dir/file.txt',
                remoteUid: 'r',
                conflictPath: 'dir/file (conflict 2026-10-24 143012).txt',
            },
            { type: 'upload_new', path: 'dir/file.txt' },
        ]);
    });

    it('uploads a revision when only the local file changed', () => {
        const state = stored(localFile(), remoteFile());
        expect(planPath('a.txt', localFile(11), remoteFile(), state, options)).toEqual([
            { type: 'upload_revision', path: 'a.txt', remoteUid: 'remote-1' },
        ]);
    });

    it('downloads when only the remote file changed', () => {
        const state = stored(localFile(), remoteFile());
        expect(planPath('a.txt', localFile(), remoteFile('remote-1', 11), state, options)).toEqual([
            { type: 'download_update', path: 'a.txt', remoteUid: 'remote-1' },
        ]);
    });

    it('downloads when the remote UID changed', () => {
        const state = stored(localFile(), remoteFile());
        expect(planPath('a.txt', localFile(), remoteFile('new-uid'), state, options)).toEqual([
            { type: 'download_update', path: 'a.txt', remoteUid: 'new-uid' },
        ]);
    });

    it('keeps both when both files changed', () => {
        const state = stored(localFile(), remoteFile());
        expect(planPath('a.txt', localFile(12), remoteFile('remote-1', 13), state, options)).toEqual([
            {
                type: 'download_conflict',
                path: 'a.txt',
                remoteUid: 'remote-1',
                conflictPath: 'a (conflict 2026-10-24 143012).txt',
            },
            { type: 'upload_revision', path: 'a.txt', remoteUid: 'remote-1' },
        ]);
    });

    it('does nothing when neither side changed', () => {
        const state = stored(localFile(), remoteFile());
        expect(planPath('a.txt', localFile(), remoteFile(), state, options)).toEqual([]);
    });

    it('trashes the remote file when the local copy was deleted and remote is unchanged', () => {
        const state = stored(localFile(), remoteFile());
        expect(planPath('a.txt', undefined, remoteFile(), state, options)).toEqual([
            { type: 'trash_remote', path: 'a.txt', remoteUid: 'remote-1' },
        ]);
    });

    it('downloads the remote file when local was deleted but remote changed', () => {
        const state = stored(localFile(), remoteFile());
        expect(planPath('a.txt', undefined, remoteFile('remote-1', 20), state, options)).toEqual([
            { type: 'download_new', path: 'a.txt', remoteUid: 'remote-1' },
        ]);
    });

    it('trashes the local file when the remote copy was deleted and local is unchanged', () => {
        const state = stored(localFile(), remoteFile());
        expect(planPath('a.txt', localFile(), undefined, state, options)).toEqual([
            { type: 'trash_local', path: 'a.txt' },
        ]);
    });

    it('uploads again when local was edited while the remote copy was deleted', () => {
        const state = stored(localFile(), remoteFile());
        expect(planPath('a.txt', localFile(20), undefined, state, options)).toEqual([
            { type: 'upload_new', path: 'a.txt' },
        ]);
    });

    it('removes stale state when both sides are gone', () => {
        const state = stored(localFile(), remoteFile());
        expect(planPath('a.txt', undefined, undefined, state, options)).toEqual([
            { type: 'remove_state', path: 'a.txt' },
        ]);
    });

    it('holds deletes when deletes are disabled', () => {
        const state = stored(localFile(), remoteFile());
        const noDeletes = { ...options, deleteEnabled: false };
        expect(planPath('a.txt', undefined, remoteFile(), state, noDeletes)).toEqual([
            { type: 'hold', path: 'a.txt', reason: 'local deleted while deletes are disabled' },
        ]);
        expect(planPath('a.txt', localFile(), undefined, state, noDeletes)).toEqual([
            { type: 'hold', path: 'a.txt', reason: 'remote deleted while deletes are disabled' },
        ]);
    });

    it('holds a file/directory type mismatch', () => {
        expect(planPath('a', localFile(), remoteDir(), undefined, options)).toEqual([
            { type: 'hold', path: 'a', reason: 'local and remote types differ' },
        ]);
    });

    it('creates directories on the missing side', () => {
        expect(planPath('empty', localDir(), undefined, undefined, options)).toEqual([
            { type: 'mkdir_remote', path: 'empty' },
        ]);
        expect(planPath('empty', undefined, remoteDir(), undefined, options)).toEqual([
            { type: 'mkdir_local', path: 'empty' },
        ]);
        expect(planPath('empty', localDir(), remoteDir(), undefined, options)).toEqual([
            { type: 'link_only', path: 'empty' },
        ]);
    });

    it('does not delete a local directory just because its remote copy is missing', () => {
        const state = stored(localDir(), remoteDir());
        expect(planPath('empty', localDir(), undefined, state, options)).toEqual([
            { type: 'mkdir_remote', path: 'empty' },
        ]);
    });
});

describe('conflictName', () => {
    it('inserts a timestamp before the extension', () => {
        expect(conflictName('dir/file.txt', now)).toBe(
            'dir/file (conflict 2026-10-24 143012).txt'
        );
    });

    it('handles compound extensions and dotfiles', () => {
        expect(conflictName('archive.tar.gz', now)).toBe(
            'archive.tar (conflict 2026-10-24 143012).gz'
        );
        expect(conflictName('.bashrc', now)).toBe(
            '.bashrc (conflict 2026-10-24 143012)'
        );
    });
});

describe('massDeleteGuard', () => {
    it('allows small deletes', () => {
        expect(
            massDeleteGuard({
                localCount: 20,
                remoteCount: 20,
                trackedCount: 20,
                deleteCount: 2,
            })
        ).toBeNull();
    });

    it('holds when the local tree looks empty', () => {
        expect(
            massDeleteGuard({
                localCount: 0,
                remoteCount: 20,
                trackedCount: 20,
                deleteCount: 20,
            })
        ).toContain('local tree is empty');
    });

    it('holds when the remote tree looks empty', () => {
        expect(
            massDeleteGuard({
                localCount: 20,
                remoteCount: 0,
                trackedCount: 20,
                deleteCount: 20,
            })
        ).toContain('remote tree is empty');
    });

    it('holds a large proportional delete', () => {
        expect(
            massDeleteGuard({
                localCount: 20,
                remoteCount: 20,
                trackedCount: 20,
                deleteCount: 11,
            })
        ).toContain('mass delete');
    });
});
