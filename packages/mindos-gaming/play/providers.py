"""Launcher download information from local Steam manifests."""
from pathlib import Path
import runpy
import time


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
