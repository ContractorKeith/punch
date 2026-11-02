#!/usr/bin/env bash
# Run checks with a disposable store and fail-closed external CLI shims.
set -euo pipefail
scratch=$(mktemp -d "${TMPDIR:-/tmp}/punch-check.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/bin" "$scratch/state"
for tool in gh basecamp; do
  printf '#!/bin/sh\necho "unexpected external tool call during checks" >&2\nexit 97\n' > "$scratch/bin/$tool"
  chmod +x "$scratch/bin/$tool"
done
export PATH="$scratch/bin:$PATH"
export PUNCH_STATE_DIR="$scratch/state"
for tool in gh basecamp; do
  test "$(command -v "$tool")" = "$scratch/bin/$tool"
done
"$@"
