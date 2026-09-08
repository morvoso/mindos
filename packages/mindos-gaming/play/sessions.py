"""Suspend only game processes launched in MindOS-owned systemd scopes."""
import os
from pathlib import Path
import shutil
import subprocess
import time
from .common import DATA, audit, game, key, lock, read, run, write


def unit(gid):
    return 'mindos-game-' + key(gid) + '.scope'


def state(gid):
    p = run(['systemctl', '--user', 'show', unit(gid), '--property=ActiveState,FreezerState,ControlGroup'], check=False)
    props = dict(line.split('=', 1) for line in p.stdout.splitlines() if '=' in line)
    active = props.get('ActiveState') == 'active'
    return dict(active=active, suspended=active and props.get('FreezerState') == 'frozen', managed=active)


def list_sessions():
    result = []
    for path in sorted((DATA / 'sessions').glob('*.json'), key=lambda p: p.stat().st_mtime, reverse=True)[:100]:
        item = read(path)
        if not item.get('ended'):
            item.update(state(item['game']))
        else:
            item.update(active=False, suspended=False, managed=True)
        result.append(item)
    return result


def change(gid, suspend):
    with lock('session-' + key(gid)):
        before = state(gid)
        if not before['active']:
            raise ValueError('This game is not in a managed session. Set the launch command shown in Session setup, then restart the game.')
        start = time.monotonic()
        run(['systemctl', '--user', 'freeze' if suspend else 'thaw', unit(gid)])
        after = state(gid)
        if after['suspended'] != suspend:
            raise ValueError('The game did not reach the requested session state')
        after['transition_ms'] = round((time.monotonic() - start) * 1000)
        audit('suspend' if suspend else 'resume', game=gid, transition_ms=after['transition_ms'])
        return after


def execute(gid, argv):
    if not argv:
        raise ValueError('Pass the actual game command after --')
    if state(gid)['active']:
        raise ValueError('A managed session for this game is already running')
    session = f'{key(gid)}-{time.time_ns()}'
    directory = DATA / 'traces' / session
    directory.mkdir(parents=True, mode=0o700)
    record = DATA / 'sessions' / (session + '.json')
    env = dict(os.environ)
    profile = read(DATA / 'games' / (key(gid) + '.json'))
    env['MANGOHUD_CONFIG'] = ','.join(filter(None, [env.get('MANGOHUD_CONFIG', ''),
        f'autostart_log=1,log_interval=100,output_folder={directory},fps_limit={profile.get("fps_limit",0)}']))
    env['MANGOHUD'] = '1'
    env['MINDOS_GAME_ID'] = gid
    command = list(argv)
    if shutil.which('mangohud'):
        command.insert(0, 'mangohud')
    if shutil.which('gamemoderun'):
        command.insert(0, 'gamemoderun')
    data = dict(id=session, game=gid, started=time.time(), trace=str(directory), unit=unit(gid), fps_limit=profile.get('fps_limit',0))
    write(record, data)
    audit('game-start', game=gid, session=session)
    try:
        code = subprocess.call(['systemd-run', '--user', '--scope', '--collect', '--quiet',
                                '--property=PartOf=mindos-session.target',
                                '--unit=' + unit(gid), '--', *command], env=env)
        data.update(ended=time.time(), exit_code=code)
        return code
    finally:
        data.setdefault('ended', time.time())
        write(record, data)
        audit('game-end', game=gid, session=session)


def setup(gid):
    import shlex
    prefix = f'mindos-play run {shlex.quote(gid)} --'
    # Steam %command% is expanded by Steam, not by this helper.
    if gid.startswith('steam:'):
        command = prefix + ' %command%'
        instructions = 'Steam: Properties → General → Launch Options. Keep existing game options after --. Flatpak Steam needs a compatible host-access wrapper.'
    elif gid.startswith('lutris:'):
        command = prefix
        instructions = 'Lutris: System options → Command prefix. Paste this prefix so it wraps the actual game executable.'
    else:
        command = prefix
        instructions = 'Run this prefix followed by the game executable and its arguments, or use your launcher’s command-prefix setting. Launching an already-running client does not manage its games.'
    return {'command': command, 'instructions': instructions, 'unit': unit(gid)}
