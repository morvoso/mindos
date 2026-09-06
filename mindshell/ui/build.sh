#!/bin/bash
# Build the mindshell UI bundle: build.sh [outdir]   (default: ./dist)
# Produces <outdir>/index.html, app.js, app.css and fonts/. Needs esbuild from
# PATH (Arch: pacman -S esbuild), from ./node_modules (npm install) or, as a
# last resort, npx (network).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="${1:-$here/dist}"
mkdir -p "$out/fonts"
if command -v esbuild >/dev/null 2>&1; then
  esb=(esbuild)
elif [[ -x "$here/node_modules/.bin/esbuild" ]]; then
  esb=("$here/node_modules/.bin/esbuild")
else
  esb=(npx --yes esbuild)
fi
"${esb[@]}" "$here/src/main.ts" --bundle --format=iife --target=es2022 --minify \
  --legal-comments=none --log-level=warning --outfile="$out/app.js"
"${esb[@]}" "$here/src/app.css" --minify --log-level=warning --outfile="$out/app.css"
cp "$here/index.html" "$out/index.html"
cp "$here"/fonts/*.ttf "$out/fonts/"
printf 'mindshell ui: %s (%s bytes app.js, %s bytes app.css)\n' "$out" "$(stat -c %s "$out/app.js")" "$(stat -c %s "$out/app.css")"
