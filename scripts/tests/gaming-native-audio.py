#!/usr/bin/env python3
"""QA VM only, as desktop user: duck a disposable silent stream and expire lease."""
import json,os,subprocess,tempfile,time
with tempfile.TemporaryDirectory(prefix='mindos-audio-test-') as tmp:
    env=dict(os.environ,XDG_DATA_HOME=tmp+'/data',XDG_CONFIG_HOME=tmp+'/config')
    player=subprocess.Popen(['pacat','--raw','--playback','--rate=48000','--channels=2','--format=s16le','--client-name=MindOS-QA-audio','--stream-name=QA-silence'],stdin=open('/dev/zero','rb'),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    def call(action,**kw):
        p=subprocess.run(['mindos-play'],input=json.dumps(dict(action=action,**kw)),env=env,text=True,capture_output=True,timeout=10)
        assert p.returncode==0,p.stdout
        return json.loads(p.stdout)
    try:
        for _ in range(40):
            stream=next((s for s in call('audio.streams') if s['pid']==str(player.pid)),None)
            if stream:break
            time.sleep(.1)
        assert stream,'QA stream did not appear'
        call('audio.volume',stream=stream['id'],percent=80)
        call('audio.duck',stream=stream['id'],enabled=True)
        current=next(s for s in call('audio.streams') if s['id']==stream['id'])
        assert current['volume']==48,current
        for _ in range(34):
            time.sleep(.5)
            current=next(s for s in call('audio.streams') if s['id']==stream['id'])
            if current['volume']==80:break
        assert current['volume']==80,current
        print('PASS native audio: 80% → 48%, lease expired → 80% without companion cleanup')
    finally:
        player.terminate();player.wait(timeout=5)
