"""Verified game relocation and versioned saves. Every mutation is user requested."""
import hashlib
import os
from pathlib import Path
import shutil
import time
from .common import DATA, audit, config, game, key, lock, read, safe_dir, write
from .sessions import state


def stopped(gid):
    if state(gid)['active']:
        raise ValueError('Close the game before changing its files, including suspended sessions.')
    for p in Path('/proc').glob('[0-9]*/environ'):
        try:
            env = p.read_bytes().split(b'\0')
            if b'MINDOS_GAME_ID=' + gid.encode() in env or (gid.startswith('steam:') and b'SteamAppId=' + gid[6:].encode() in env):
                raise ValueError('This game has running processes. Close it before changing files.')
        except (OSError, PermissionError):
            continue


def manifest(path):
    result = {}
    for base, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            p = Path(base) / name
            rel = str(p.relative_to(path))
            if p.is_symlink():
                result[rel] = ['link', os.readlink(p)]
            elif p.is_file():
                h = hashlib.sha256()
                with p.open('rb') as src:
                    for block in iter(lambda: src.read(4 * 1024 * 1024), b''): h.update(block)
                result[rel] = [p.stat().st_size, h.hexdigest()]
            elif p.is_dir():
                result[rel] = ['directory']
            else:
                raise ValueError(f'Special file cannot be moved: {rel}')
    return result


def bytes_in(path):
    return sum(p.lstat().st_size for base, _, files in os.walk(path, followlinks=False)
               for p in (Path(base) / f for f in files) if not p.is_symlink())


def verified_copy(src, dst):
    if dst.exists() or dst.is_symlink(): raise ValueError('Destination already exists')
    before = manifest(src)
    shutil.copytree(src, dst, symlinks=True)
    if manifest(dst) != before or manifest(src) != before:
        raise ValueError('Files changed during copying or verification failed. The original is unchanged; the partial copy was retained for inspection.')
    return before


def index():
    return read(DATA / 'storage.json')


def plan(gid, restore=False):
    g = game(gid)
    entry = index().get(gid)
    if restore:
        if not entry: raise ValueError('Game is not in cold storage')
        src, dst = Path(entry['cold']), Path(entry['original'])
        if not dst.is_symlink() or dst.resolve() != src.resolve():
            raise ValueError('The launcher path has changed; refusing to replace it')
    else:
        src = Path(g['path'])
        resolved = src.resolve()
        if resolved == Path.home() or Path.home().is_relative_to(resolved) or DATA.resolve().is_relative_to(resolved):
            raise ValueError('The reported game folder contains user or system state; refusing to relocate it')
        if src.is_symlink(): raise ValueError('Game is already linked; use Restore for managed cold storage')
        base = safe_dir(config().get('cold_folder', ''))
        if base.is_relative_to(src.resolve()): raise ValueError('Cold storage cannot be inside the game')
        dst = base / ('mindos-' + key(gid))
        if dst.exists() or dst.is_symlink(): raise ValueError('Cold destination already exists')
    size = bytes_in(src)
    free = shutil.disk_usage(dst.parent).free
    return dict(game=gid, source=str(src), destination=str(dst), bytes=size, free=free,
                restore=restore, enough_space=free > size + 64*1024*1024,
                same_filesystem=src.stat().st_dev == dst.parent.stat().st_dev)


def relocate(gid, restore=False):
    with lock('storage'), lock('files-' + key(gid)):
        stopped(gid)
        p = plan(gid, restore)
        if not p['enough_space']: raise ValueError('Not enough free space at the destination')
        src, dst = Path(p['source']), Path(p['destination'])
        stage = dst.with_name(dst.name + '.mindos-' + str(time.time_ns()))
        verified_copy(src, stage)
        stopped(gid)
        records = index()
        if restore:
            # Keep the original cold copy until the verified warm copy is live.
            old_link = dst.with_name(dst.name + '.mindos-link')
            if old_link.exists() or old_link.is_symlink(): raise ValueError('A previous restore requires recovery')
            dst.rename(old_link)
            try: stage.rename(dst)
            except BaseException:
                old_link.rename(dst)
                raise
            old_link.unlink()
            records.pop(gid, None)
            write(DATA / 'storage.json', records)
            shutil.rmtree(src)
        else:
            stage.rename(dst)
            backup = src.with_name(src.name + '.mindos-original')
            if backup.exists() or backup.is_symlink(): raise ValueError('A previous move requires recovery')
            src.rename(backup)
            try: src.symlink_to(dst, target_is_directory=True)
            except BaseException:
                backup.rename(src)
                raise
            records[gid] = dict(original=str(src), cold=str(dst), time=time.time(), bytes=p['bytes'])
            write(DATA / 'storage.json', records)
            shutil.rmtree(backup)
        audit('restore-storage' if restore else 'archive-game', game=gid, bytes=p['bytes'])
        return p


def save_path(gid):
    meta = read(DATA / 'games' / (key(gid) + '.json'))
    return safe_dir(meta.get('save_path', ''))


def snapshot(gid, cloud=False):
    with lock('files-' + key(gid)):
        stopped(gid)
        src = save_path(gid)
        base = safe_dir(config().get('cloud_folder', '')) / 'MindOS-saves' if cloud else DATA / 'saves'
        if base.resolve().is_relative_to(src): raise ValueError('Backup folder cannot be inside the saves')
        name = f'{time.time_ns()}'
        dst = base / key(gid) / name
        dst.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        contents = verified_copy(src, dst)
        write(dst.parent / (name + '.json'), dict(id=name, game=gid, path=str(dst), source=str(src), time=time.time(), files=len(contents), cloud=cloud, manifest=contents))
        audit('cloud-save' if cloud else 'backup-saves', game=gid, revision=name)
        return {'id': name, 'files': len(contents), 'path': str(dst)}


def save_list(gid):
    bases = [DATA / 'saves']
    cloud = config().get('cloud_folder')
    if cloud and Path(cloud).is_dir(): bases.append(Path(cloud) / 'MindOS-saves')
    return sorted([read(p) for base in bases for p in (base / key(gid)).glob('*.json')], key=lambda x:x['time'], reverse=True)


def restore_save(gid, revision):
    stopped(gid)
    selected = next((s for s in save_list(gid) if s['id'] == revision), None)
    if not selected: raise ValueError('Save revision not found')
    if manifest(Path(selected['path'])) != selected.get('manifest'):
        raise ValueError('The saved revision has changed or is damaged; refusing to restore it')
    # Back up current saves before any replacement, even when restoring older saves.
    backup = snapshot(gid)
    with lock('files-' + key(gid)):
        stopped(gid)
        dst = save_path(gid)
        stage = dst.with_name(dst.name + '.restore-' + str(time.time_ns()))
        verified_copy(Path(selected['path']), stage)
        old = dst.with_name(dst.name + '.before-restore-' + str(time.time_ns()))
        dst.rename(old)
        try: stage.rename(dst)
        except BaseException:
            old.rename(dst)
            raise
        audit('restore-saves', game=gid, revision=revision, previous=str(old))
        return {'restored': revision, 'backup': backup, 'previous': str(old)}
