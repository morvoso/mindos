#!/bin/bash
# Screenshot the preview page with headless Chromium: shot.sh [outdir]
# Needs `chromium` (or set CHROMIUM=/path/to/chrome).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="${1:-${MINDSHELL_SHOT_DIR:-$here/shots}}"
chromium="${CHROMIUM:-chromium}"
mkdir -p "$out"
url="file://$here/dist/index.html"
shot() {
  local size="${3:-1920,1080}"
  "$chromium" --headless --disable-gpu --hide-scrollbars --no-sandbox "--window-size=$size" \
    --virtual-time-budget=4000 --screenshot="$out/$1.png" "$url?$2" >/dev/null 2>&1
  echo "$out/$1.png"
}
shot preview             'kind=preview'
shot preview-edit        'kind=preview&edit=1&demo=1'
shot preview-layout-mode 'kind=preview&popup=layout-mode&mode=dwindle'
shot preview-popups      'kind=preview&popup=calendar,context-menu'
shot preview-audio       'kind=preview&popup=audio,tray-menu'
shot preview-catalog     'kind=preview&edit=1&popup=widget-catalog'
shot preview-settings    'kind=preview&edit=1&popup=widget-settings'
shot preview-vertical    'kind=preview&vertical=1&labels=1&battery=1'
shot preview-app         'kind=preview&app=settings&page=displays'
# The apps at their default window size.
shot app-overview        'kind=app&id=settings' 1440,1000
shot app-settings        'kind=app&id=settings&arg=%7B%22page%22%3A%22mind%22%7D' 1040,700
shot app-wallpaper       'kind=app&id=settings&arg=%7B%22page%22%3A%22wallpaper%22%7D' 1040,700
shot app-displays        'kind=app&id=settings&arg=%7B%22page%22%3A%22displays%22%7D' 1040,700
shot app-desktop         'kind=app&id=settings&arg=%7B%22page%22%3A%22shell%22%7D' 1040,700
shot app-updates         'kind=app&id=settings&arg=%7B%22page%22%3A%22updates%22%7D' 1040,760
shot app-performance     'kind=app&id=settings&arg=%7B%22page%22%3A%22performance%22%7D' 1040,760
shot app-games           'kind=app&id=settings&arg=%7B%22page%22%3A%22games%22%7D' 1040,760
shot app-software        'kind=app&id=settings&arg=%7B%22page%22%3A%22software%22%7D' 1040,760
shot app-input           'kind=app&id=settings&arg=%7B%22page%22%3A%22input%22%7D' 1040,760
shot preview-notifications 'kind=preview&popup=notifications'
shot preview-perf        'kind=preview&popup=perf'
shot preview-auth        'kind=preview&popup=auth'
shot toast               'kind=toast&id=toast&output=Virtual-1' 400,300
# The login screen (kind=greeter) on a full output.
shot greeter             'kind=greeter&arg=%7B%22primary%22%3Atrue%7D'
# The lock screen and a screensaver or two (kind=lock).
shot lock                'kind=lock&arg=%7B%22primary%22%3Atrue%7D&saver=breaker'
shot saver-serpent       'kind=lock&arg=%7B%22primary%22%3Afalse%7D&stage=screensaver&saver=serpent'
shot saver-volley        'kind=lock&arg=%7B%22primary%22%3Afalse%7D&stage=screensaver&saver=volley'
shot saver-breaker       'kind=lock&arg=%7B%22primary%22%3Afalse%7D&stage=screensaver&saver=breaker'
shot saver-drift         'kind=lock&arg=%7B%22primary%22%3Afalse%7D&stage=screensaver&saver=drift'
shot saver-wave          'kind=lock&arg=%7B%22primary%22%3Afalse%7D&stage=screensaver&saver=wave'
shot saver-lander        'kind=lock&arg=%7B%22primary%22%3Afalse%7D&stage=screensaver&saver=lander'
shot saver-starfield     'kind=lock&arg=%7B%22primary%22%3Afalse%7D&stage=screensaver&saver=starfield'
shot app-screen          'kind=app&id=settings&arg=%7B%22page%22%3A%22screen%22%7D' 1040,760
