#!/bin/bash
# Entry point: refresh the package database once (the image can be days old),
# then run the requested command as the builder user unless asked for root.
set -e
if [[ "${MINDOS_ROOT:-0}" == "1" ]]; then
  exec "$@"
fi
exec sudo -u builder -E HOME=/home/builder "$@"
