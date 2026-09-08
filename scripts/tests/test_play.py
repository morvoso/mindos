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
        r=common.update_config(dict(steam_key='a'*32,steam_id='76561198000000000'))
        self.assertTrue(r['steam_connected']);self.assertNotIn('steam_key',r);self.assertNotIn('obs_password',r)
        self.assertEqual(common.CONFIG.stat().st_mode & 0o777,0o600)
        common.update_config(dict(steam_key=''));self.assertEqual(common.config()['steam_key'],'a'*32)
        with self.assertRaises(ValueError): common.update_config(dict(obs_password='removed'))
        for action in ('obs', 'captures'):
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
    def test_steam_summary_uses_v2_and_redacts_network_errors(self):
        common.update_config(dict(steam_key='a'*32,steam_id='76561198000000000'))
        import urllib.error
        with patch.object(providers.urllib.request,'urlopen',side_effect=urllib.error.URLError('secret')) as req:
            with self.assertRaisesRegex(ValueError,'Steam could not'):providers.steam_api('ISteamUser','GetPlayerSummaries',steamids='1')
            self.assertIn('/v0002/',req.call_args.args[0])
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
