#!/bin/bash
# Run a command inside the MindOS build box (Docker). The repository is mounted
# at /work. Use `scripts/buildbox.sh --root <cmd>` for steps that need root
# (mkarchiso). Build the image with `scripts/buildbox.sh --build`.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="mindos-buildbox"
if [[ "${1:-}" == "--build" ]]; then
  exec docker build -t "$image" "$here/scripts/buildbox"
fi
args=(--rm -v "$here:/work" -w /work -e TERM="${TERM:-xterm}")
if [[ -t 0 ]]; then args+=(-it); fi
if [[ "${1:-}" == "--root" ]]; then
  shift
  args+=(--privileged -e MINDOS_ROOT=1)
fi
exec docker run "${args[@]}" "$image" "$@"
