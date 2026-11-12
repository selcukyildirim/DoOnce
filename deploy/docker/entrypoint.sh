#!/bin/sh
set -eu
umask 027
if [ "${DOONCE_MASTER_KEY:-}" = "" ] && [ ! -f "${DOONCE_HOME:-/data}/master.key" ]; then
  echo "warning: DOONCE_MASTER_KEY is not set; a local master.key will be created for single-node use" >&2
fi
exec /usr/local/bin/doonce "$@"
