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
# Explicit build choices must cross the container boundary. Without these,
# `MINDOS_CPU=generic make kernel` silently used the recipe's default instead.
for choice in MINDOS_CPU MINDOS_LTO MINDOS_JOBS MINDOS_MODULE_SIGN_KEY MINDOS_MODULE_SIGN_CERT; do
  if [[ -v $choice ]]; then args+=(-e "$choice"); fi
done
[[ ${MINDOS_CPU:-generic} =~ ^(generic|native)$ ]] || { echo 'MINDOS_CPU must be generic or native' >&2; exit 1; }
[[ ${MINDOS_LTO:-thin} =~ ^(thin|none)$ ]] || { echo 'MINDOS_LTO must be thin or none' >&2; exit 1; }
[[ ${MINDOS_JOBS:-1} =~ ^[1-9][0-9]*$ ]] || { echo 'MINDOS_JOBS must be a positive integer' >&2; exit 1; }
if [[ -t 0 ]]; then args+=(-it); fi
if [[ "${1:-}" == "--root" ]]; then
  shift
  args+=(--privileged -e MINDOS_ROOT=1)
fi
exec docker run "${args[@]}" "$image" "$@"
