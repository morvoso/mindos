#!/usr/bin/env python3
"""Isolated gaming backend tests: no launchers, accounts or real game files."""
import base64
import hashlib
import importlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'packages/mindos-gaming'))
from play import common, sessions, storage, telemetry, providers, media, service

class PlayTest(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(); self.root=Path(self.temp.name)
        self.patches=[]
        for module in (common,sessions,storage,telemetry,providers,media,service):
            if hasattr(module,'DATA'): self.patches.append(patch.object(module,'DATA',self.root/'data'))
            if hasattr(module,'CONFIG'): self.patches.append(patch.object(module,'CONFIG',self.root/'config.json'))
        self.patches += [patch.object(storage,'stopped'),patch.object(storage,'game',lambda gid:dict(id=gid,path=str(self.root/'game')))]
        for p in self.patches:p.start()
        self.src=self.root/'game';self.src.mkdir();(self.src/'empty').mkdir();(self.src/'game.bin').write_bytes(b'game\0data'*100)
        (self.root/'cold').mkdir();(self.root/'cloud').mkdir()
        common.update_config(dict(cold_folder=str(self.root/'cold'),cloud_folder=str(self.root/'cloud')))
    def tearDown(self):
        for p in reversed(self.patches):p.stop()
        self.temp.cleanup()
    def test_config_secrets_private_and_not_returned(self):
        for secret in ('steam_key','steam_id','obs_password'):
            with self.assertRaises(ValueError): common.update_config({secret:'value'})
        self.assertEqual(common.CONFIG.stat().st_mode & 0o777,0o600)
        # Credentials left behind by an older version never appear in a response.
        common.write(common.CONFIG,{**common.config(),'steam_key':'a'*32,'steam_id':'76561198000000000','obs_password':'x','obs_port':4455})
        r=common.public_config()
        for k in ('steam_key','steam_id','obs_password','obs_port'): self.assertNotIn(k,r)
        self.assertEqual(r['cold_folder'],str(self.root/'cold'))
        for action in ('obs','captures','friends','achievements','config.disconnect'):
            with self.assertRaises(ValueError): service.dispatch(dict(action=action))
        with self.assertRaises(ValueError):common.update_config({'cold_folder':'/'})
    def test_native_game_progress_survives_without_launcher_manifest(self):
        gid='desktop:org.example.Game.desktop'
        service.dispatch(dict(action='metadata.set',game=gid,settings=dict(completion=42,notes='Checkpoint reached')))
        with patch.object(service,'games',return_value=[]), patch.object(sessions,'list_sessions',return_value=[]):
            state=service.dispatch(dict(action='library.state',games=[gid]))
            self.assertEqual(state['metadata'][gid]['completion'],42)
            self.assertEqual(state['metadata'][gid]['notes'],'Checkpoint reached')
            with self.assertRaises(ValueError):service.dispatch(dict(action='library.state',games=['/etc/passwd']))
    def test_storage_roundtrip_preserves_symlinks_empty_dirs_bytes(self):
        (self.src/'link').symlink_to('game.bin');before=storage.manifest(self.src)
        p=storage.relocate('steam:10');self.assertTrue(self.src.is_symlink());self.assertEqual(before,storage.manifest(self.src));self.assertTrue(Path(p['destination']).is_dir())
        storage.relocate('steam:10',True);self.assertFalse(self.src.is_symlink());self.assertEqual(before,storage.manifest(self.src));self.assertFalse(storage.index())
    def test_storage_refuses_existing_destination(self):
        p=storage.plan('steam:10');Path(p['destination']).mkdir()
        with self.assertRaises(ValueError):storage.relocate('steam:10')
        self.assertTrue((self.src/'game.bin').exists())
    def test_storage_copy_failure_keeps_original(self):
        with patch.object(storage.shutil,'copytree',side_effect=OSError('disk disconnected')):
            with self.assertRaises(OSError):storage.relocate('steam:10')
        self.assertFalse(self.src.is_symlink());self.assertTrue((self.src/'game.bin').exists());self.assertFalse(storage.index())
    def test_storage_changed_files_abort(self):
        real=storage.shutil.copytree
        def copy(src,dst,*args,**kw):
            result=real(src,dst,*args,**kw)
            if Path(src)==self.src: (self.src/'game.bin').write_text('changed')
            return result
        with patch.object(storage.shutil,'copytree',side_effect=copy):
            with self.assertRaisesRegex(ValueError,'changed'):storage.relocate('steam:10')
        self.assertFalse(self.src.is_symlink());self.assertEqual((self.src/'game.bin').read_text(),'changed')
    def test_save_restore_local_and_cloud_preserves_current(self):
        service.metadata('steam:10',{'save_path':str(self.src)})
        a=storage.snapshot('steam:10');storage.snapshot('steam:10',True)
        self.assertEqual(len(storage.save_list('steam:10')),2)
        (self.src/'game.bin').write_text('new save')
        result=storage.restore_save('steam:10',a['id'])
        self.assertEqual((Path(result['previous'])/'game.bin').read_text(),'new save')
        self.assertEqual((self.src/'game.bin').read_bytes(),b'game\0data'*100)
    def test_damaged_backup_refused(self):
        service.metadata('steam:10',{'save_path':str(self.src)})
        a=storage.snapshot('steam:10');(Path(a['path'])/'game.bin').write_text('corrupt')
        with self.assertRaisesRegex(ValueError,'damaged'):storage.restore_save('steam:10',a['id'])
        self.assertEqual((self.src/'game.bin').read_bytes(),b'game\0data'*100)
    def test_nested_save_backup_refused(self):
        service.metadata('steam:10',{'save_path':str(self.root)})
        with self.assertRaises(ValueError):storage.snapshot('steam:10')
    def test_session_freeze_only_owned_unit(self):
        with patch.object(sessions,'state',side_effect=[dict(active=True),dict(active=True,suspended=True)]),patch.object(sessions,'run') as run:
            result=sessions.change('steam:10',True)
            self.assertIn('transition_ms',result);self.assertEqual(run.call_args.args[0],['systemctl','--user','freeze',sessions.unit('steam:10')])
        with patch.object(sessions,'state',return_value=dict(active=False)),patch.object(sessions,'run') as run:
            with self.assertRaises(ValueError):sessions.change('steam:10',True)
            run.assert_not_called()
    def test_trace_skips_metadata_nan_and_computes_recorded_metrics(self):
        p=self.root/'trace.csv';p.write_text('os,gpu\nLinux,Test\nfps,frametime,gpu_temp\n100,10,60\n100,10,62\n20,50,64\nnan,nan,60\n')
        r=telemetry.parse_trace(p);self.assertEqual(r['samples'],3);self.assertEqual(r['avg_fps'],73.3);self.assertEqual(r['p99_ms'],50);self.assertEqual(r['stutters'],1)
    def test_providers_only_read_local_manifests(self):
        for name in ('steam_api','friends','achievements','urllib'): self.assertFalse(hasattr(providers,name))
        with patch.object(Path,'home',return_value=self.root):
            self.assertEqual(providers.downloads()['items'],[])
    def test_library_state_uses_the_listed_games_without_scanning(self):
        service.dispatch(dict(action='metadata.set',game='steam:10',settings=dict(completion=7)))
        with patch.object(service,'games',side_effect=AssertionError('the library was scanned')),patch.object(sessions,'list_sessions',return_value=[]):
            state=service.dispatch(dict(action='library.state',games=['steam:10','desktop:a.desktop'],scanned=True))
            self.assertEqual(set(state['metadata']),{'steam:10','desktop:a.desktop'})
            self.assertEqual(state['metadata']['steam:10']['completion'],7);self.assertEqual(state['metadata']['desktop:a.desktop'],{})
            self.assertEqual(service.dispatch(dict(action='library.state',games=[],scanned=True))['metadata'],{})
            for bad in (dict(games=['steam:10'],scanned='yes'),dict(games=['steam:10'],scanned=1),dict(games=['steam:10']),dict(games='steam:10',scanned=True),
                        dict(games=[1],scanned=True),dict(games=['x'*513],scanned=True),dict(games=['a']*2001,scanned=True)):
                with self.assertRaises(ValueError):service.dispatch(dict(action='library.state',**bad))
        # Without "scanned", the listed ids are desktop games added to a scan made here.
        with patch.object(service,'games',return_value=[dict(id='steam:10')]),patch.object(sessions,'list_sessions',return_value=[]):
            state=service.dispatch(dict(action='library.state',games=['desktop:a.desktop']))
            self.assertEqual(set(state['metadata']),{'steam:10','desktop:a.desktop'})
    def test_sessions_ask_systemctl_once_for_every_running_session(self):
        folder=self.root/'data/sessions'
        for i,(gid,ended) in enumerate([('steam:1',None),('steam:2',None),('steam:1',5.0),('steam:3',None)]):
            common.write(folder/f's{i}.json',dict(id=f's{i}',game=gid,started=1.0,**({'ended':ended} if ended else {})))
        u1,u2,u3=(sessions.unit(g) for g in ('steam:1','steam:2','steam:3'))
        output=f'Id={u2}\nActiveState=active\nFreezerState=frozen\nControlGroup=/x\n\nId={u1}\nActiveState=active\nFreezerState=running\n\nId={u3}\nActiveState=inactive\n'
        with patch.object(sessions,'run',return_value=subprocess.CompletedProcess([],0,output,'')) as run:
            result={s['id']:s for s in sessions.list_sessions()}
        self.assertEqual(run.call_count,1)
        argv=run.call_args.args[0]
        self.assertEqual(argv[:3],['systemctl','--user','show']);self.assertEqual(sorted(argv[3:6]),sorted([u1,u2,u3]))
        self.assertEqual(argv[6:],['--property=Id,ActiveState,FreezerState,ControlGroup'])
        self.assertEqual((result['s0']['active'],result['s0']['suspended']),(True,False))
        self.assertEqual((result['s1']['active'],result['s1']['suspended']),(True,True))
        self.assertEqual((result['s2']['active'],result['s2']['managed']),(False,True))
        self.assertFalse(result['s3']['active'])
        with patch.object(sessions,'run',return_value=subprocess.CompletedProcess([],0,'','')) as run:
            self.assertFalse(any(s['active'] for s in sessions.list_sessions()))
    def test_history_reuses_the_summary_kept_at_game_end_until_the_csv_changes(self):
        trace=self.root/'data/traces/s1';trace.mkdir(parents=True)
        recording=trace/'log.csv';recording.write_text('fps,frametime,gpu_temp\n100,10,60\n100,10,62\n20,50,64\n')
        fresh=telemetry.parse_trace(recording)
        common.write(self.root/'data/sessions/s1.json',dict(id='s1',game='steam:1',started=1.0,ended=2.0,telemetry=telemetry.summarize(trace)))
        with patch.object(telemetry,'parse_trace',side_effect=AssertionError('parsed again')):
            history,=telemetry.history()
            self.assertEqual(telemetry.recommendations('steam:1')['evidence'][0]['avg_fps'],fresh['avg_fps'])
        self.assertEqual(json.dumps(history['stats']),json.dumps(fresh))
        self.assertNotIn('telemetry',history);self.assertNotIn('telemetry',sessions.list_sessions()[0])
        recording.write_text('fps,frametime\n50,20\n50,20\n')
        history,=telemetry.history()
        self.assertEqual(history['stats'],telemetry.parse_trace(recording));self.assertEqual(history['stats']['avg_fps'],50)
    def test_game_end_keeps_the_recording_summary_in_the_record(self):
        def game(argv,env):
            folder=env['MANGOHUD_CONFIG'].split('output_folder=')[1].split(',')[0]
            (Path(folder)/'game.csv').write_text('fps,frametime\n60,16.67\n60,16.67\n')
            return 3
        with patch.object(sessions,'state',return_value=dict(active=False)),patch.object(sessions.subprocess,'call',side_effect=game),patch.object(sessions.shutil,'which',return_value=None):
            self.assertEqual(sessions.execute('steam:1',['game']),3)
        record,=(self.root/'data/sessions').glob('*.json')
        data=common.read(record)
        self.assertEqual(data['exit_code'],3);self.assertEqual(data['telemetry']['stats']['avg_fps'],60)
        self.assertEqual([t[0] for t in data['telemetry']['traces']],['game.csv'])
        with patch.object(telemetry,'parse_trace',side_effect=AssertionError('parsed again')):
            history,=telemetry.history('steam:1')
        self.assertEqual(history['stats']['samples'],2);self.assertNotIn('telemetry',history)
    def test_duck_is_relative_renews_lease_and_restores_pid(self):
        current=dict(id=2,pid='123',volume=80,name='Game')
        with patch.object(media,'streams',return_value=[current]),patch.object(media,'volume') as volume,patch.object(media.subprocess,'Popen'):
            media.duck(2,True);volume.assert_called_with(2,48)
            self.assertGreater(common.read(media.DATA/'duck.json')['2']['expires'],time.time())
            media.duck(2,False);volume.assert_called_with(2,80)
            media.duck(2,True);current['pid']='456';volume.reset_mock();media.duck(2,False);volume.assert_not_called()
    def test_metadata_validation(self):
        with self.assertRaises(ValueError):service.metadata('steam:10',dict(completion=101))
        with self.assertRaises(ValueError):service.dispatch(dict(action='exec',command='false'))

if __name__=='__main__': unittest.main()
