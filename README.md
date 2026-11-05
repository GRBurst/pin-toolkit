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

M2 adds `--update`: before pinning, each reference's tag is bumped to the
latest eligible stable tag (see [Tag updates](#tag-updates--update)).
Components (per-project `components` image definitions) are a later
milestone.

## Usage

```
pin-ci [OPTIONS] [FILES]...
```

- `FILES` — CI files to pin (default: `ci/.gitlab-ci.yml`)
- `--dry-run` — resolve and report without writing files
- `--json` — print the report as JSON instead of human-readable text
- `--concurrency` — max concurrent digest lookups per file (default: 8)
- `--update` — bump each reference's tag to the latest eligible stable tag
  before pinning it (same major as the current tag by default)
- `--update-major` — with `--update`, also allow major version bumps

Example:

```console
$ pin-ci --update --update-major ci/.gitlab-ci.yml
ci/.gitlab-ci.yml:1  updated  alpine:3.20 -> alpine:3.24@sha256:25b0e91ad7048476c484f3a5128e2b5d9a4f2c1e8b7a6d5c4f3e2d1a0b9c8d7e  # alpine:3.20
ci/.gitlab-ci.yml:3  updated  ubuntu:24.04 -> ubuntu:26.04@sha256:8083fbb2465d13d61f77932a49d1a1fde3c2b1a09f8e7d6c5b4a39281706f5e4  # ubuntu:24.04
pinned: 0, ok: 0, updated: 2, skipped: 0, unresolved: 0
```

## Tag updates (`--update`)

For every Docker Hub reference, the tag list is fetched from the registry
and the tag is bumped to the **highest stable tag compatible with the
current one** (docker versioning: pre-release/beta/nightly tags are
ignored). Compatibility means the same pre-release suffix and the same
number of release components (`3.20` ↔ `3.24`, `24.04` ↔ `26.04`; `24` is
not compatible with `24.0`). By default the major must not change;
`--update-major` lifts that restriction. The bumped tag is then pinned as
`repo:new-tag@sha256:<digest>` in the same rewrite.

A reference is never bumped (digest/SHA re-pin only) when:

- its tag is not semver-parseable (`latest`, `main`, `stable`, `rolling`),
- it has no tag (treated as `latest`),
- it is already pinned — an existing `@sha256:<old>` is only re-pinned to
  the digest of the tag it carries.

If the tag list cannot be fetched, the reference is reported as
`unresolved(tag list: <reason>)`, the tool exits `1`, and all other
references in the file are still written.

### Differences from the old pipeline

Two deliberate divergences from `pin-ci-images.sh` (renovate + crane):

- **Same major by default.** Renovate bumps to the newest tag of any major;
  pin-ci keeps the current major unless `--update-major` is given.
- **No release-age guard.** Renovate's config sets `minimumReleaseAge: 14d`;
  pin-ci has no such guard and will pick a just-published stable tag. The
  M2 oracle treats a strictly-newer stable tag as a documented divergence
  in exactly this case.

## Environment

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
- `PIN_CI_LIVE=1 cargo test --test live` — opt-in live tests against real
  Docker Hub: digest resolution for `alpine:3.20` / `ubuntu:24.04`, plus a
  full `--update --update-major` run whose tag choice is checked against an
  independent `list_tags` + `pick_latest_stable` recomputation and whose
  digests are checked against `crane digest`; a second run must change
  nothing.
- `PIN_CI_ORACLE=1 cargo test --test oracle` — opt-in differential tests
  against the old pipeline, which runs `pin-ci-images.sh` as-is on a temp
  checkout:
  - M1 (pin-only): the new tool must pin exactly the same
    `name@sha256:...` digests the old pipeline wrote.
  - M2 (`--update --update-major` vs. the old pipeline, which bumps tags by
    default): per original reference position (line + repo), equal tags
    need byte-identical digests; a differing tag passes only as the
    documented divergence — both tags parse-stable-compatible and the new
    tool's strictly greater (renovate's 14-day release-age guard) — with
    both digests sanity-checked via `crane digest`. Anything else fails.

  Both require `renovate`, `crane`, `python3` and `git` on `PATH` (and
  outbound network); without the env flag or the tools they print a SKIP
  notice and pass.

## Old pipeline

`pin-ci-images.sh`, `extract-docker-pins.py`, and `config.js` are kept
unmodified at the repo root and serve as the correctness oracle (see
`PIN_CI_ORACLE=1` above). Their retirement is a later decision.
