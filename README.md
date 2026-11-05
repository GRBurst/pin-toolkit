# pin-ci

Pin container image references in GitLab CI files to immutable digests.

## What it does

M1 pins Docker Hub image references in GitLab CI files: every image or
service reference whose host is Docker Hub (`None`/`docker.io`/
`index.docker.io`) is resolved to its current digest and rewritten in place
as `original-as-written@sha256:<digest>` (an existing `@sha256:<old>` has its
digest replaced; tags are never changed). Non-Docker Hub references are
reported as `skipped (registry not enabled)` and left untouched. Edits are
byte-exact and idempotent.

Components (per-project `components` image definitions) and `--update` are
later milestones.

## Usage

```
pin-ci [OPTIONS] [FILES]...
```

- `FILES` — CI files to pin (default: `ci/.gitlab-ci.yml`)
- `--dry-run` — resolve and report without writing files
- `--json` — print the report as JSON instead of human-readable text
- `--concurrency` — max concurrent digest lookups per file (default: 8)

Example:

```console
$ pin-ci ci/.gitlab-ci.yml
ci/.gitlab-ci.yml:1  pinned  alpine:3.20 -> alpine:3.20@sha256:25b0e91ad7048476c484f3a5128e2b5d9a4f2c1e8b7a6d5c4f3e2d1a0b9c8d7e  # alpine:3.20
ci/.gitlab-ci.yml:3  pinned  ubuntu:24.04 -> ubuntu:24.04@sha256:8083fbb2465d13d61f77932a49d1a1fde3c2b1a09f8e7d6c5b4a39281706f5e4  # ubuntu:24.04
pinned: 2, ok: 0, skipped: 0, unresolved: 0
```

## Environment

- `DOCKERHUB_USER` / `DOCKERHUB_TOKEN` — Docker Hub credentials, used first.
- Fallback: `~/.docker/config.json` (`auths` keys, in order:
  `https://index.docker.io/v1/`, `https://docker.io`,
  `index.docker.io/v1/`, `docker.io`).

Nothing secret is written to files or logs.

## Exit codes

- `0` — nothing was `unresolved` (and no IO error).
- `1` — at least one reference was `unresolved`, or an IO error occurred.

## Tests

- `cargo test` — unit, mock-registry, and end-to-end tests. Always runs
  without network, Docker Hub credentials, or external tools.
- `PIN_CI_LIVE=1 cargo test --test live` — opt-in live tests that resolve
  `alpine:3.20` and `ubuntu:24.04` against real Docker Hub.
- `PIN_CI_ORACLE=1 cargo test --test oracle` — opt-in differential test
  against the old pipeline: it runs `pin-ci-images.sh` as-is on a temp
  checkout and asserts the new tool pins exactly the same set of
  `name@sha256:...` digests. Requires `renovate` and `crane` on `PATH`
  (and outbound network); without the env flag or the binaries it prints a
  SKIP notice and passes.

## Old pipeline

`pin-ci-images.sh`, `extract-docker-pins.py`, and `config.js` are kept
unmodified at the repo root and serve as the correctness oracle (see
`PIN_CI_ORACLE=1` above). Their retirement is a later decision.
