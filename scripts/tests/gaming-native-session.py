#!/usr/bin/env python3
"""Run INSIDE a disposable QA VM as its desktop user."""
import json, os, pathlib, subprocess, tempfile, time
with tempfile.TemporaryDirectory(prefix='mindos-session-test-') as tmp:
    env=dict(os.environ, XDG_DATA_HOME=tmp+'/data', XDG_CONFIG_HOME=tmp+'/config')
    counter=pathlib.Path(tmp)/'counter'
    def call(action,**kw):
        result=subprocess.run(['mindos-play'],input=json.dumps(dict(action=action,**kw)),text=True,capture_output=True,env=env,timeout=20)
        assert result.returncode==0,(result.stdout,result.stderr)
        return json.loads(result.stdout)
    worker=subprocess.Popen(['mindos-play','run','qa:session-freeze','--','python3','-c',f'import time,pathlib\np=pathlib.Path({str(counter)!r})\nfor i in range(1000):\n p.write_text(str(i));time.sleep(.05)'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
    try:
        for i in range(80):
            if counter.exists():break
            time.sleep(.1)
        assert counter.exists(),worker.stderr.read().decode() if worker.poll() is not None else 'worker did not start'
        assert call('sessions')[0]['active']
        frozen=call('session.suspend',game='qa:session-freeze')
        before=counter.read_text();time.sleep(.35);assert counter.read_text()==before,'counter advanced while frozen'
        resumed=call('session.resume',game='qa:session-freeze')
        time.sleep(.2);assert counter.read_text()!=before,'counter did not resume'
        print(json.dumps(dict(test='native systemd game scope freeze/thaw',held_ms=frozen['transition_ms'],resumed_ms=resumed['transition_ms'],passed=True)))
    finally:
        import hashlib
        unit='mindos-game-'+hashlib.sha256(b'qa:session-freeze').hexdigest()[:24]+'.scope'
        subprocess.run(['systemctl','--user','thaw',unit],capture_output=True)
        subprocess.run(['systemctl','--user','stop',unit],capture_output=True)
        worker.wait(timeout=10)
