import json
from pathlib import Path
import sys
import time
from . import media, providers, sessions, storage, telemetry
from .common import CONFIG, DATA, audit, config, game, games, key, lock, public_config, read, run, safe_dir, update_config, write


def metadata(gid, patch=None):
    path = DATA/'games'/(key(gid)+'.json')
    with lock('metadata'):
        value = read(path)
        if patch is not None:
            if set(patch)-{'save_path','completion','chapter','notes','wiki','video','fps_limit'}: raise ValueError('Unknown game setting')
            if 'fps_limit' in patch:
                patch['fps_limit']=int(patch['fps_limit'])
                if not 0<=patch['fps_limit']<=1000: raise ValueError('FPS limit must be between 0 (unlimited) and 1000')
            if 'save_path' in patch and patch['save_path']: patch['save_path']=str(safe_dir(patch['save_path']))
            if 'completion' in patch:
                patch['completion']=int(patch['completion'])
                if not 0<=patch['completion']<=100: raise ValueError('Completion must be between 0 and 100')
            for k in ('chapter','notes','wiki','video'):
                if k in patch and (not isinstance(patch[k],str) or len(patch[k])>8000): raise ValueError('Invalid game note or link')
            value.update(patch)
            write(path,value)
    return value


def metadata_all(ids):
    """metadata() for every game the Library shows, under one lock."""
    with lock('metadata'):
        return {gid:read(DATA/'games'/(key(gid)+'.json')) for gid in ids}


def boot():
    p = run(['systemd-analyze','--no-pager','time'],check=False)
    q = run(['systemd-analyze','--no-pager','blame'],check=False)
    return {'summary':p.stdout.strip(), 'services':q.stdout.splitlines()[:18], 'uptime':float(Path('/proc/uptime').read_text().split()[0])}


def dispatch(p):
    action=p.get('action')
    gid=p.get('game','')
    if action=='config.get': return public_config()
    if action=='config.set': return update_config(p.get('settings',{}))
    if action=='sessions': return sessions.list_sessions()
    if action=='library.state':
        # With "scanned": the caller already ran mindos-games and lists every game it shows;
        # without it, the ids are desktop games added to a scan made here.
        ids = p.get('games', [])
        scanned = p.get('scanned', False)
        if not isinstance(scanned, bool) or not isinstance(ids, list) or len(ids)>2000 or any(
                not isinstance(g, str) or len(g)>512 or (not scanned and not g.startswith('desktop:')) for g in ids):
            raise ValueError('Invalid game list' if scanned else 'Invalid desktop game list')
        ids = set(ids) if scanned else {g['id'] for g in games()} | set(ids)
        return dict(metadata=metadata_all(ids), storage=storage.index(), sessions=sessions.list_sessions())
    if action=='session.setup': return sessions.setup(gid)
    if action=='session.suspend': return sessions.change(gid,True)
    if action=='session.resume': return sessions.change(gid,False)
    if action=='metadata.get': return metadata(gid)
    if action=='metadata.set': return metadata(gid,p.get('settings',{}))
    if action=='history': return telemetry.history(gid or None)
    if action=='analyze': return telemetry.recommendations(gid)
    if action=='compare': return telemetry.compare(p.get('before'),p.get('after'))
    if action=='downloads': return providers.downloads()
    if action=='storage.list': return storage.index()
    if action=='storage.plan': return storage.plan(gid,bool(p.get('restore')))
    if action=='storage.move': return storage.relocate(gid,bool(p.get('restore')))
    if action=='saves.list': return storage.save_list(gid)
    if action=='saves.backup': return storage.snapshot(gid,bool(p.get('cloud')))
    if action=='saves.restore': return storage.restore_save(gid,p.get('revision'))
    if action=='audio.streams': return media.streams()
    if action=='audio.volume': return media.volume(p.get('stream'),p.get('percent'))
    if action=='audio.duck': return media.duck(p.get('stream'),bool(p.get('enabled')),p.get('percent',40))
    if action=='audio.restore':
        media.restore_ducked()
        return {}
    if action=='boot': return boot()
    if action=='activity':
        path=DATA/'events.jsonl'
        if not path.exists(): return []
        return [json.loads(s) for s in path.read_text().splitlines()[-40:]][::-1]
    raise ValueError('Unknown gaming request')


def main():
    if len(sys.argv)>1 and sys.argv[1]=='audio-watch':
        media.watch_ducked()
        return 0
    if len(sys.argv)>1 and sys.argv[1]=='run':
        if len(sys.argv)<5 or sys.argv[3]!='--': raise ValueError('Usage: mindos-play run GAME -- COMMAND [ARGS]')
        return sessions.execute(sys.argv[2],sys.argv[4:])
    raw=sys.stdin.read(65537)
    if len(raw)>65536: raise ValueError('Gaming request too large')
    p=json.loads(raw)
    if not isinstance(p,dict): raise ValueError('Expected a request object')
    print(json.dumps(dispatch(p)))
    return 0
