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
# run_path hands back a copy; the scanner's functions look names up here.
G = helper['scan'].__globals__


def parsed_again(*args, **kwargs):
    raise AssertionError('the library was parsed again')


def without_parsing():
    """Fail any scan that reads a manifest, launcher JSON or the Lutris database."""
    return patch.dict(G, {'fields': parsed_again, 'read_json': parsed_again,
                          'sqlite3': type('sqlite3', (), {'connect': staticmethod(parsed_again), 'Error': sqlite3.Error})})


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

    def full_library(self):
        self.manifest()
        epic = self.home / 'Epic game'
        epic.mkdir()
        self.write(self.home / '.config/legendary/installed.json', json.dumps({'ep&ic': {'title': 'Epic', 'install_path': str(epic)}}))
        self.write(self.home / '.config/heroic/gog_store/installed.json', json.dumps({'installed': [{'appName': '123', 'install_path': str(epic)}]}))
        db = self.home / '.local/share/lutris/pga.db'
        db.parent.mkdir(parents=True)
        with sqlite3.connect(db) as con:
            con.execute('CREATE TABLE games (id INTEGER, name TEXT, directory TEXT, installed INTEGER)')
            con.execute('INSERT INTO games VALUES (7, "Lutris game", ?, 1)', (str(self.home),))
        return db

    def test_unchanged_library_is_served_from_the_cache_without_parsing(self):
        db = self.full_library()
        first = json.dumps(helper['scan']())
        self.assertEqual({g['source'] for g in json.loads(first)['games']}, {'steam', 'heroic', 'gog', 'lutris'})
        self.assertTrue((self.home / '.cache/mindos/games.json').is_file())
        with without_parsing():
            self.assertEqual(json.dumps(helper['scan']()), first)
        # Every input the scan depends on is watched: manifests, art, install
        # folders, launcher JSON and the Lutris database.
        self.manifest(name='Game', flags='4')
        self.write(self.steam / 'steamapps/appmanifest_42.acf', (self.steam / 'steamapps/appmanifest_42.acf').read_text().replace('1234', '5678'))
        self.assertEqual(next(g['lastPlayed'] for g in self.scan() if g['id'] == 'steam:42'), 5678)
        art = self.steam / 'appcache/librarycache/42/header.jpg'
        self.write(art, 'cover')
        self.assertEqual(next(g['art'] for g in self.scan() if g['id'] == 'steam:42'), str(art))
        (self.home / 'Epic game').rmdir()
        self.assertEqual({g['source'] for g in self.scan()}, {'steam', 'lutris'})
        with sqlite3.connect(db) as con:
            con.execute('INSERT INTO games VALUES (8, "Second", ?, 1)', (str(self.home),))
        self.assertIn('lutris:8', {g['id'] for g in self.scan()})
        self.write(self.home / '.config/legendary/installed.json', '{}')
        (self.steam / 'steamapps/appmanifest_42.acf').unlink()
        self.assertEqual({g['id'] for g in self.scan()}, {'lutris:7', 'lutris:8'})
        with without_parsing():
            self.assertEqual({g['id'] for g in self.scan()}, {'lutris:7', 'lutris:8'})

    def test_cache_location_damage_and_unreadable_launchers(self):
        cache = self.home / 'elsewhere/mindos/games.json'
        with patch.dict(os.environ, {'XDG_CACHE_HOME': str(self.home / 'elsewhere')}):
            self.manifest()
            expected = json.dumps(helper['scan']())
            self.assertTrue(cache.is_file())
            self.assertFalse((self.home / '.cache').exists())
            cache.write_text('{"context": [], "result": {"games": [{"id": "steam:1"}], "warnings": []}, "inputs": []}')
            self.assertEqual(json.dumps(helper['scan']()), expected)
            cache.write_text('not json')
            self.assertEqual(json.dumps(helper['scan']()), expected)
            with without_parsing():
                self.assertEqual(json.dumps(helper['scan']()), expected)
            # A launcher that could not be read is asked again next time, not remembered.
            self.write(self.home / '.local/share/lutris/pga.db', 'not a database')
            self.assertEqual(len(helper['scan']()['warnings']), 1)
            self.assertIsNone(helper['cached_library']())

    def test_launch_needs_no_scan_while_the_cached_library_holds_the_game(self):
        self.full_library()
        self.scan()
        with without_parsing(), patch('subprocess.run', return_value=subprocess.CompletedProcess([], 0, '', '')) as run:
            self.assertTrue(helper['launch']('steam:42')['dispatched'])
            self.assertEqual(run.call_args.args[0], ['gio', 'open', 'steam://rungameid/42'])
            helper['launch']('heroic:ep&ic')
            self.assertEqual(run.call_args.args[0], ['gio', 'open', 'heroic://launch?appName=ep%26ic&runner=legendary'])
            helper['launch']('lutris:7')
            self.assertEqual(run.call_args.args[0], ['gio', 'open', 'lutris:rungameid/7'])
        # A game the cache does not know is looked for once more before it is refused.
        with patch('subprocess.run', return_value=subprocess.CompletedProcess([], 0, '', '')) as run:
            with self.assertRaisesRegex(ValueError, 'no longer installed'):
                helper['launch']('steam:7')
            run.assert_not_called()
            self.manifest('7', 'New game', directory='New')
            self.assertTrue(helper['launch']('steam:7')['dispatched'])
            (self.home / '.cache/mindos/games.json').unlink()
            self.assertTrue(helper['launch']('steam:42')['dispatched'])
            self.assertTrue((self.home / '.cache/mindos/games.json').is_file())


if __name__ == '__main__':
    unittest.main()
