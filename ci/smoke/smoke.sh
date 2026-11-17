#!/bin/sh
# DF1 smoke: pin a copy of ci/smoke/pins.yml, assert every pin, then assert
# that a second run is a clean --check (idempotence). POSIX sh (busybox in the
# CI image). Usage: sh ci/smoke/smoke.sh [OUT_DIR]   (default: smoke-out)
set -eu
here=$(dirname "$0")
out=${1:-smoke-out}
f=$out/pins.yml
mkdir -p "$out"
cp "$here/pins.yml" "$f"

fail() { echo "smoke: FAIL: $*" >&2; exit 1; }
has() { grep -Eqx -- "$1" "$f" || fail "no line matching /$1/ in $f"; }
run() { # run <report> <args...>: exit code must be 0; show the report either way
  r=$1; shift
  rc=0; pin-toolkit gitlab-ci "$@" "$f" > "$r" || rc=$?
  cat "$r"
  [ "$rc" -eq 0 ] || fail "pin-toolkit gitlab-ci${*:+ $*} $f exited $rc"
}

run "$out/pin.txt"
grep -qx 'pinned: 4, ok: 0, updated: 0, skipped: 0, unresolved: 0' "$out/pin.txt" \
  || fail "first run: expected 4 pinned"
has 'image: public\.ecr\.aws/docker/library/alpine:3\.20@sha256:[0-9a-f]{64}'
has 'component: gitlab\.com/components/secret-detection/secret-detection@[0-9a-f]{40}  # 2\.4\.0'
has '    ref: [0-9a-f]{40}  # main'
has '    ref: b63c04bea5c4376d34f9597cd357271bc654fb30  # v0\.1\.0'
# branch pipelines: main resolves to the commit this pipeline runs for
# (a newer push auto-cancels this interruptible job first)
if [ -n "${CI_COMMIT_BRANCH:-}" ]; then has "    ref: ${CI_COMMIT_SHA}  # main"; fi

run "$out/check.txt" --check
grep -qx 'pinned: 0, ok: 4, updated: 0, skipped: 0, unresolved: 0' "$out/check.txt" \
  || fail "second run: expected a clean --check with 4 ok"
echo "smoke: OK"
