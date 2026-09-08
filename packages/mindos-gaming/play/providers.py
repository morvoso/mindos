"""Launcher downloads and opt-in Steam Web API data."""
import json
from pathlib import Path
import runpy
import time
import urllib.error
import urllib.parse
import urllib.request
from .common import DATA, config, key, read, write


def steam_api(interface, method, **params):
    cfg = config()
    if not cfg.get('steam_id') or not cfg.get('steam_key'):
        raise ValueError('Connect Steam with your SteamID64 and Web API key in Gaming connections.')
    # Fixed HTTPS origin, fixed methods, bounded response. Never return key URLs in errors.
    version = 'v0002' if method == 'GetPlayerSummaries' else 'v0001'
    url = f'https://api.steampowered.com/{interface}/{method}/{version}/?' + urllib.parse.urlencode(dict(key=cfg['steam_key'], **params))
    try:
        with urllib.request.urlopen(url, timeout=12) as response:
            raw = response.read(8*1024*1024 + 1)
            if len(raw) > 8*1024*1024: raise ValueError('Steam response too large')
            return json.loads(raw)
    except (urllib.error.URLError, ValueError):
        raise ValueError('Steam could not return this data. Check the API key, profile privacy and network connection.') from None


def friends(refresh=False):
    if not config().get('steam_key') or not config().get('steam_id'):
        raise ValueError('Connect Steam in Gaming connections to see friends.')
    path = DATA / 'steam-friends.json'
    cached = read(path)
    if not refresh and cached.get('at', 0) > time.time()-60: return cached
    cfg = config()
    result = steam_api('ISteamUser', 'GetFriendList', steamid=cfg.get('steam_id', ''), relationship='friend')
    ids = [f['steamid'] for f in result.get('friendslist', {}).get('friends', [])]
    people = []
    for offset in range(0, min(len(ids), 2000), 100):
        response = steam_api('ISteamUser', 'GetPlayerSummaries', steamids=','.join(ids[offset:offset+100]))
        for p in response.get('response', {}).get('players', []):
            people.append({k: p[k] for k in ('steamid', 'personaname', 'personastate', 'gameextrainfo', 'gameid', 'gameserverip', 'avatar') if k in p})
    people.sort(key=lambda p: (-bool(p.get('gameid')), -bool(p.get('personastate')), p.get('personaname', '').lower()))
    result = dict(at=time.time(), friends=people, source='Steam')
    write(path, result)
    return result


def achievements(gid):
    if not gid.startswith('steam:') or not gid[6:].isdigit():
        raise ValueError('Achievement import is available for Steam games; other games support manual completion tracking.')
    result = steam_api('ISteamUserStats', 'GetPlayerAchievements', steamid=config().get('steam_id'), appid=gid[6:], l='english')
    stats = result.get('playerstats', {})
    if not stats.get('success'): raise ValueError('Achievements are private or unavailable for this game')
    items = stats.get('achievements', [])
    return dict(items=items, unlocked=sum(bool(a.get('achieved')) for a in items), total=len(items))


def downloads():
    # Steam's manifest counters are durable, provider-owned byte counts. No guessed download rates.
    from .common import games
    script = Path(__file__).resolve().parents[1] / 'mindos-games'
    if not script.exists(): script = Path('/usr/bin/mindos-games')
    fields = runpy.run_path(str(script))['fields']
    seeds = [Path.home()/'.local/share/Steam', Path.home()/'.steam/root', Path.home()/'.var/app/com.valvesoftware.Steam/.local/share/Steam']
    import re
    libs = {s.resolve() for s in seeds if s.is_dir()}
    for s in list(libs):
        try:
            for p in re.findall(r'"path"\s+"([^"]+)"', (s/'steamapps/libraryfolders.vdf').read_text()): libs.add(Path(p).resolve())
        except OSError: pass
    result, seen = [], set()
    for lib in libs:
        for path in (lib/'steamapps').glob('appmanifest_*.acf'):
            f = fields(path)
            gid = f.get('appid', '')
            if gid in seen or not gid.isdigit(): continue
            seen.add(gid)
            def number(k):
                try: return max(0, int(f.get(k, 0)))
                except ValueError: return 0
            total, done = number('BytesToDownload'), number('BytesDownloaded')
            if total > done:
                result.append(dict(id='steam:'+gid, name=f.get('name',gid), source='Steam', downloaded=done, total=total,
                                   percent=round(done/total*100,1), path=str(path)))
    return {'items': result, 'at': time.time(), 'note': 'Steam manifest progress. Other launcher queues remain managed by their launchers.'}
