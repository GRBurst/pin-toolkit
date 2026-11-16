#!/usr/bin/env bash
# The Nix gate: every flake check for this system, then evaluation of the
# other systems' checks (aarch64-linux and aarch64-darwin build only in CI).
# Nix sees git-tracked files only: refuse untracked, not-ignored files anywhere
# in the repository (no per-source path list to keep in step with the checks).
set -euo pipefail
cd "$(dirname "$0")/.."
untracked=$(git ls-files --others --exclude-standard)
if [ -n "$untracked" ]; then
  echo "nix-check: untracked files are invisible to Nix; git add them first:" >&2
  echo "$untracked" >&2
  exit 1
fi
nix flake check -L "$@"
nix flake check --no-build --all-systems
