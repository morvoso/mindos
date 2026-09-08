#!/usr/bin/env python3
"""Game library discovery and launcher handoff, with isolated local fixtures."""
import json
import os
from pathlib import Path
import runpy
import sqlite3
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
helper = runpy.run_path(str(ROOT / 'packages/mindos-gaming/mindos-games'))


class GamesTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.home = Path(self.tmp.name)
        env = patch.dict(os.environ, {'HOME': str(self.home), 'XDG_CONFIG_HOME': str(self.home / '.config'), 'XDG_DATA_HOME': str(self.home / '.local/share')})
        env.start()
        self.addCleanup(env.stop)
        self.steam = self.home / '.local/share/Steam'

    def write(self, path, data):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(data)

    def manifest(self, appid='42', name='Game', library=None, flags='4', directory='Game'):
        library = library or self.steam
        path = library / 'steamapps/common' / directory
        path.mkdir(parents=True, exist_ok=True)
        self.write(library / f'steamapps/appmanifest_{appid}.acf',
                   f'"AppState" {{ "appid" "{appid}" "name" "{name}" "installdir" "{directory}" "StateFlags" "{flags}" "LastPlayed" "1234" }}')
        return path

    def scan(self):
        return helper['scan']()['games']

    def test_steam_without_libraryfolders_and_cached_art(self):
        self.manifest()
        art = self.steam / 'appcache/librarycache/42/library_600x900.jpg'
        self.write(art, 'cached cover')
        game, = self.scan()
        self.assertEqual((game['id'], game['uri'], game['lastPlayed']), ('steam:42', 'steam://rungameid/42', 1234))
        self.assertEqual(game['art'], str(art))

    def test_multiple_libraries_and_aliases_are_deduplicated(self):
        self.manifest()
        secondary = self.home / 'Disk Two'
        self.manifest('99', 'Other Game', secondary)
        self.write(self.steam / 'steamapps/libraryfolders.vdf', f'"libraryfolders" {{ "1" {{ "path" "{secondary}" }} }}')
        (self.home / '.steam').mkdir()
        (self.home / '.steam/root').symlink_to(self.steam, target_is_directory=True)
        self.assertEqual({g['id'] for g in self.scan()}, {'steam:42', 'steam:99'})

    def test_tools_partial_downloads_and_path_escape_are_excluded(self):
        self.manifest('1', 'Proton Experimental')
        self.manifest('2', 'Still downloading', flags='2')
        self.manifest('3', 'Outside library', directory='../../outside')
        self.manifest('4', 'Installed')
        self.assertEqual([g['id'] for g in self.scan()], ['steam:4'])

    def test_heroic_flatpak_and_gog(self):
        game = self.home / 'Epic game'
        game.mkdir()
        base = self.home / '.var/app/com.heroicgameslauncher.hgl/config'
        slug = 'game&runner=unexpected;$(echo nope)'
        self.write(base / 'heroic/legendaryConfig/legendary/installed.json', json.dumps({slug: {'title': 'Epic Game', 'install_path': str(game)}}))
        self.write(self.home / '.config/heroic/gog_store/installed.json', json.dumps({'installed': [{'appName': '123', 'install_path': str(game)}]}))
        games = self.scan()
        self.assertEqual({g['source'] for g in games}, {'heroic', 'gog'})
        epic = next(g for g in games if g['source'] == 'heroic')
        self.assertIn('appName=game%26runner%3Dunexpected', epic['uri'])
        self.assertTrue(epic['uri'].endswith('&runner=legendary'))

    def test_lutris_uses_numeric_id_and_installed_flag(self):
        db = self.home / '.local/share/lutris/pga.db'
        db.parent.mkdir(parents=True)
        with sqlite3.connect(db) as con:
            con.execute('CREATE TABLE games (id INTEGER, name TEXT, directory TEXT, installed INTEGER)')
            con.executemany('INSERT INTO games VALUES (?, ?, ?, ?)', [(7, 'Game', str(self.home), 1), (8, 'Removed', str(self.home), 0)])
        game, = self.scan()
        self.assertEqual(game['uri'], 'lutris:rungameid/7')

    def test_broken_launcher_does_not_hide_steam(self):
        self.manifest()
        self.write(self.home / '.config/legendary/installed.json', '{broken')
        self.write(self.home / '.local/share/lutris/pga.db', 'not a database')
        result = helper['scan']()
        self.assertEqual(len(result['games']), 1)
        self.assertEqual(len(result['warnings']), 1)

    def test_launch_only_dispatches_discovered_game_as_arguments(self):
        self.manifest()
        with patch('subprocess.run', return_value=subprocess.CompletedProcess([], 0, '', '')) as run:
            self.assertTrue(helper['launch']('steam:42')['dispatched'])
            self.assertEqual(run.call_args.args[0], ['gio', 'open', 'steam://rungameid/42'])
        with self.assertRaises(ValueError):
            helper['launch']('steam:42; echo bad')

    def test_malformed_json_shapes_do_not_hide_other_libraries(self):
        self.manifest()
        self.write(self.home / '.config/legendary/installed.json', '{"bad":{"install_path":42}}')
        self.write(self.home / '.config/heroic/gog_store/installed.json', '{"installed":null}')
        self.write(self.home / '.config/heroic/store_cache/gog_library.json', '{"games":null}')
        self.assertEqual([g['id'] for g in self.scan()], ['steam:42'])

    def test_launch_errors_are_reported(self):
        self.manifest()
        with patch('subprocess.run', return_value=subprocess.CompletedProcess([], 1, '', 'No URI handler')):
            with self.assertRaisesRegex(ValueError, 'No URI handler'):
                helper['launch']('steam:42')


if __name__ == '__main__':
    unittest.main()
