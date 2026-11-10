#!/usr/bin/env bash
# The repository gate: run before every commit (and in CI once it exists).
# Uses the `cargo` on PATH (devenv has no Rust by design; export the
# toolchain's bin dir and, optionally, CARGO_HOME before calling).
# Logs go to .tmp/gate-*.log; exit status 0 only when every step passed.
set -u
cd "$(dirname "$0")/.."
mkdir -p .tmp
fail=0
step() { # step <name> <log> <cmd…>: run, report ok/FAIL, keep going
  local name=$1 log=$2; shift 2
  if "$@" > ".tmp/gate-$log.log" 2>&1; then echo "ok    $name"; else
    echo "FAIL  $name (.tmp/gate-$log.log)"; tail -20 ".tmp/gate-$log.log"; fail=1; fi
}

# Tests, all features; snapshots must match (no .snap.new written).
INSTA_UPDATE=no cargo test --workspace --all-features > .tmp/gate-test.log 2>&1 || fail=1
grep -E '^test result' .tmp/gate-test.log | awk '{p+=$4; f+=$6} END {print "tests passed", p, "failed", f}'
grep -E '^test .* FAILED|^error' .tmp/gate-test.log | head -20

step "fmt" fmt cargo fmt --all -- --check
step "clippy (workspace, all features)" clippy \
  cargo clippy --workspace --all-targets --all-features -- -D warnings

# pin-core feature matrix.
for f in "" "oci" "gitlab" "yaml" "oci,gitlab,test-support"; do
  step "clippy pin-core [${f:-no features}]" "core-${f//,/-}" \
    cargo clippy -p pin-core --no-default-features ${f:+--features "$f"} --all-targets -- -D warnings
done

# pin-toolkit without plugins.
step "clippy pin-toolkit --no-default-features" nd-clippy \
  cargo clippy -p pin-toolkit --no-default-features --all-targets -- -D warnings
cargo test -p pin-toolkit --no-default-features > .tmp/gate-nd-test.log 2>&1 || fail=1
grep -E '^test result' .tmp/gate-nd-test.log | awk '{p+=$4; f+=$6} END {print "tests pin-toolkit --no-default-features passed", p, "failed", f}'

# D38: raw registry/GitLab clients unreachable from plugin and binary code
# (dev-dependency features are not unified by `cargo check -p`), and no
# normal/build dependency enables pin-core/test-support.
step "D38 cargo check -p pin-gitlab-ci -p pin-toolkit" d38-check \
  cargo check -p pin-gitlab-ci -p pin-toolkit
for p in pin-gitlab-ci pin-toolkit; do
  if cargo tree -p "$p" -e normal,build -f '{p} {f}' | grep -q 'pin-core.*test-support'; then
    echo "FAIL  D38 cargo tree [$p]: a normal dependency enables pin-core/test-support"; fail=1
  else echo "ok    D38 cargo tree [$p]"; fi
done

step "rustdoc -D warnings" doc env RUSTDOCFLAGS="-D warnings" \
  cargo doc --workspace --all-features --no-deps

exit $fail
