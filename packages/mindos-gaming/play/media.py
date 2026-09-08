"""Per-stream PipeWire/PulseAudio mixing."""
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from .common import DATA, lock, read, run, write


def streams():
    result = json.loads(run(['pactl','--format=json','list','sink-inputs']).stdout)
    return [dict(id=s['index'], name=s.get('properties',{}).get('application.name','Audio stream'),
                 pid=str(s.get('properties',{}).get('application.process.id','')),
                 volume=round(sum(c.get('value',0) for c in s.get('volume',{}).values()) / max(1,len(s.get('volume',{}))) / 655.36),
                 muted=s.get('mute',False)) for s in result]


def volume(stream, percent):
    stream, percent = int(stream), int(percent)
    if not 0 <= percent <= 150: raise ValueError('Volume must be between 0 and 150%')
    if not any(s['id'] == stream for s in streams()): raise ValueError('Audio stream no longer exists')
    run(['pactl','set-sink-input-volume',str(stream),f'{percent}%'])
    return streams()


def duck(stream, enabled, percent=40):
    stream = int(stream)
    with lock('audio'):
        saved = read(DATA/'duck.json')
        current = next((s for s in streams() if s['id']==stream),None)
        old = saved.get(str(stream))
        if enabled:
            if not current: raise ValueError('Audio stream no longer exists')
            if old and old['pid'] != current['pid']:
                saved.pop(str(stream),None)
                old = None
            if not old:
                saved[str(stream)] = current
                write(DATA/'duck.json',saved)
            saved[str(stream)]['expires'] = time.time()+12
            volume(stream, round(saved[str(stream)]['volume'] * (100-max(0,min(100,int(percent))))/100))
        elif old:
            if current and old['pid'] == current['pid']:
                volume(stream,old['volume'])
            saved.pop(str(stream),None)
        write(DATA/'duck.json',saved)
    if enabled:
        subprocess.Popen([sys.executable, str(Path(sys.argv[0]).resolve()), 'audio-watch'],
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    return streams()


def restore_ducked():
    for stream in list(read(DATA/'duck.json')):
        duck(stream,False)


def watch_ducked():
    import fcntl
    DATA.mkdir(parents=True, exist_ok=True)
    with (DATA/'audio-watch.lock').open('a') as handle:
        try: fcntl.flock(handle, fcntl.LOCK_EX|fcntl.LOCK_NB)
        except BlockingIOError: return
        while True:
            saved = read(DATA/'duck.json')
            if not saved: return
            for stream, old in saved.items():
                if old.get('expires',0) <= time.time():
                    try: duck(stream, False)
                    except (OSError, ValueError): pass
            time.sleep(2)
