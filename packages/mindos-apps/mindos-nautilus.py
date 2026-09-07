# MindOS entries in the Files (Nautilus) context menus: "Open in Terminal" on a
# folder or the folder background (Nautilus only offers its own for GNOME
# Console). "Set as Background" needs nothing here: the shell implements the
# Wallpaper portal. Loaded by nautilus-python from
# /usr/share/nautilus-python/extensions/.
import os
import subprocess
from urllib.parse import unquote, urlparse

from gi.repository import GObject, Nautilus

def local_path(info):
    if info.get_uri_scheme() != "file":
        return None
    return unquote(urlparse(info.get_uri()).path)


def spawn(argv, cwd=None):
    subprocess.Popen(argv, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)


class MindOSMenu(GObject.GObject, Nautilus.MenuProvider):
    def get_file_items(self, files):
        if len(files) != 1:
            return []
        info = files[0]
        path = local_path(info)
        if path is None:
            return []
        if info.is_directory():
            item = Nautilus.MenuItem(name="MindOS::terminal", label="Open in Terminal", tip="Open a terminal in this folder")
            item.connect("activate", lambda _item, p: spawn(["foot"], cwd=p), path)
            return [item]
        return []

    def get_background_items(self, folder):
        path = local_path(folder)
        if path is None or not os.path.isdir(path):
            return []
        item = Nautilus.MenuItem(name="MindOS::terminal-here", label="Open in Terminal", tip="Open a terminal in this folder")
        item.connect("activate", lambda _item, p: spawn(["foot"], cwd=p), path)
        return [item]
