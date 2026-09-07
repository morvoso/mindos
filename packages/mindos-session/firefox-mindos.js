// MindOS defaults for Firefox (/usr/lib/firefox/defaults/pref/mindos.js).
// Dark everything: the browser chrome follows a dark system theme, and pages
// see prefers-color-scheme: dark. Users can still change these in about:config
// or pick a theme in about:addons.
pref("ui.systemUsesDarkTheme", 1);
pref("browser.theme.content-theme", 0);
pref("browser.theme.toolbar-theme", 0);
pref("layout.css.prefers-color-scheme.content-override", 0);
// GTK never asks the compositor for a title bar on Wayland, so Firefox draws
// its own: tabs in the title bar, with the buttons from gtk-decoration-layout.
pref("browser.tabs.inTitlebar", 1);
pref("widget.gtk.overlay-scrollbars.enabled", true);
