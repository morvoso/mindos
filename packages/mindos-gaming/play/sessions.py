"""Suspend only game processes launched in MindOS-owned systemd scopes."""
import os
from pathlib import Path
import shutil
import subprocess
import time
from .common import DATA, audit, game, key, lock, read, run, write


def unit(gid):
    return 'mindos-game-' + key(gid) + '.scope'


def unit_state(props):
    active = props.get('ActiveState') == 'active'
    return dict(active=active, suspended=active and props.get('FreezerState') == 'frozen', managed=active)


def state(gid):
    p = run(['systemctl', '--user', 'show', unit(gid), '--property=ActiveState,FreezerState,ControlGroup'], check=False)
    props = dict(line.split('=', 1) for line in p.stdout.splitlines() if '=' in line)
    return unit_state(props)


def states(gids):
    """state() for several games from one systemctl call, keyed by game."""
    gids = list(dict.fromkeys(gids))
    if len(gids) < 2:
        return {gid: state(gid) for gid in gids}
    units = [unit(gid) for gid in gids]
    p = run(['systemctl', '--user', 'show', *units, '--property=Id,ActiveState,FreezerState,ControlGroup'], check=False)
    blocks = [dict(line.split('=', 1) for line in block.splitlines() if '=' in line) for block in p.stdout.split('\n\n')]
    by_id = {props['Id']: props for props in blocks if 'Id' in props}
    result = {}
    for i, (gid, name) in enumerate(zip(gids, units)):
        props = by_id.get(name)
        if props is None and len(blocks) == len(units):
            props = blocks[i]  # printed in the order asked
        result[gid] = unit_state(props or {})
    return result


def records():
    """Session records newest first, each with the recording summary kept at game end (never part of a response)."""
    result = []
    for path in sorted((DATA / 'sessions').glob('*.json'), key=lambda p: p.stat().st_mtime, reverse=True)[:100]:
        item = read(path)
        result.append((item, item.pop('telemetry', None)))
    return result


def load_sessions():
    """Records with their live state; one systemctl call covers every session not marked ended."""
    loaded = records()
    live = states([item['game'] for item, _ in loaded if not item.get('ended')])
    for item, _ in loaded:
        if not item.get('ended'):
            item.update(live[item['game']])
        else:
            item.update(active=False, suspended=False, managed=True)
    return loaded


def list_sessions():
    return [item for item, _ in load_sessions()]


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
        remember_recording(record, data, directory)


def remember_recording(record, data, directory):
    """Keep the recording's summary in the record, so History reads it instead of parsing the CSV again."""
    try:
        from .telemetry import summarize
        data['telemetry'] = summarize(directory)
        write(record, data)
    except Exception:  # a summary that cannot be kept now is computed from the CSV later
        pass


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
