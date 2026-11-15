#!/usr/bin/env bash
# Runs `sh ARGS...` in a container, with /tmp shared and the acc-ui
# variables passed through. Used as ACC_UI_TEST_SH by scripts/test-shells.sh.
#
#   scripts/sh-in-container.sh IMAGE ARGS...
set -euo pipefail
IMAGE="$1"
shift
exec docker run -i --rm --network none --user "$(id -u):$(id -g)" -v /tmp:/tmp \
  -e ACC_UI_DIR -e ACC_UI_SESSION -e ACC_UI_ASK_TIMEOUT \
  "$IMAGE" sh "$@"
