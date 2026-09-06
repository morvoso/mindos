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
shot app-settings        'kind=app&id=settings&arg=%7B%22page%22%3A%22mind%22%7D' 1040,700
shot app-wallpaper       'kind=app&id=settings&arg=%7B%22page%22%3A%22wallpaper%22%7D' 1040,700
shot app-displays        'kind=app&id=settings&arg=%7B%22page%22%3A%22displays%22%7D' 1040,700
shot app-desktop         'kind=app&id=settings&arg=%7B%22page%22%3A%22shell%22%7D' 1040,700
shot app-files           'kind=app&id=files' 1040,700
shot app-files-list      'kind=app&id=files&arg=%7B%22arg%22%3A%22%2Fhome%2Fmorvoso%2FDownloads%22%7D&view=list' 1040,700
