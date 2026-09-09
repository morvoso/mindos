"""Shared local state. Secrets never appear in status responses or argv."""
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile
import time

DATA = Path(os.environ.get('XDG_DATA_HOME', Path.home() / '.local/share')) / 'mindos/play'
CONFIG = Path(os.environ.get('XDG_CONFIG_HOME', Path.home() / '.config')) / 'mindos/play.json'


def read(path, default=None):
    try:
        return json.loads(Path(path).read_text())
    except FileNotFoundError:
        return {} if default is None else default
    except (OSError, ValueError) as e:
        raise ValueError(f'Cannot read {Path(path).name}: {e}') from e


def write(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix='.play-')
    try:
        with os.fdopen(fd, 'w') as out:
            json.dump(value, out)
            out.flush()
            os.fsync(out.fileno())
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


@contextlib.contextmanager
def lock(name='state'):
    DATA.mkdir(parents=True, exist_ok=True, mode=0o700)
    with (DATA / (name + '.lock')).open('a') as out:
        fcntl.flock(out, fcntl.LOCK_EX)
        yield


def run(argv, timeout=15, check=True, **kw):
    p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, **kw)
    if check and p.returncode:
        raise ValueError(p.stderr.strip() or f'{argv[0]} failed ({p.returncode})')
    return p


def key(gid):
    if not isinstance(gid, str) or not gid or len(gid) > 300:
        raise ValueError('Invalid game ID')
    return hashlib.sha256(gid.encode()).hexdigest()[:24]


def config():
    return read(CONFIG)


def games():
    script = Path(__file__).resolve().parents[1] / 'mindos-games'
    if not script.exists():
        script = Path('/usr/bin/mindos-games')
    return runpy.run_path(str(script))['scan']()['games']


def game(gid):
    result = next((g for g in games() if g['id'] == gid), None)
    if not result:
        raise ValueError('Game is not installed; refresh the library.')
    return result


def safe_dir(value):
    if not isinstance(value, str) or not value.strip():
        raise ValueError('Choose an absolute folder path')
    p = Path(value).expanduser()
    if not p.is_absolute() or not p.is_dir() or p.resolve() == Path('/'):
        raise ValueError('Folder must exist and must not be the filesystem root')
    return p.resolve()


def update_config(p):
    allowed = {'cloud_folder', 'cold_folder'}
    if set(p) - allowed:
        raise ValueError('Unknown connection setting')
    with lock():
        cfg = config()
        for k, v in p.items():
            if k in ('cloud_folder', 'cold_folder') and v:
                v = str(safe_dir(v))
            cfg[k] = v
        write(CONFIG, cfg)
    return public_config()


def public_config():
    cfg = config()
    return {k: v for k, v in cfg.items() if k not in ('steam_key', 'steam_id', 'obs_password', 'obs_port')}


def audit(action, **data):
    DATA.mkdir(parents=True, exist_ok=True, mode=0o700)
    with (DATA / 'events.jsonl').open('a') as out:
        out.write(json.dumps(dict(time=time.time(), action=action, **data)) + '\n')
