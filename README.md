# pin-ci

Pin container image references in GitLab CI files to immutable digests.

## What it does

M1 pins image references in GitLab CI files: every image or service
reference is resolved to its current digest and rewritten in place as
`original-as-written@sha256:<digest>` (an existing `@sha256:<old>` has its
digest replaced; tags are never changed). Edits are byte-exact and
idempotent. M1 started Docker-Hub-only — references on other hosts were
`skipped (registry not enabled)`; M4a removes that skip entirely (see
[Multi-registry support (M4a)](#multi-registry-support-m4a)).

M2 adds `--update`: before pinning, each reference's tag is bumped to the
latest eligible stable tag (see [Tag updates](#tag-updates--update)).
M3 extends the same run to GitLab **components** and **classic includes**
(see [Components & includes (M3)](#components--includes-m3)).

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
- `--min-age-days` — with `--update`, skip bump candidates younger than N
  days by Docker Hub push timestamp (Docker Hub refs only; other hosts are
  a no-op; default `0` = off; recommended `14`)
- `--gitlab-host` — GitLab instance host for component/include resolution
  (default: `gitlab.com`)

Example:

```console
$ pin-ci --update --update-major ci/.gitlab-ci.yml
ci/.gitlab-ci.yml:1  updated  alpine:3.20 -> alpine:3.24@sha256:25b0e91ad7048476c484f3a5128e2b5d9a4f2c1e8b7a6d5c4f3e2d1a0b9c8d7e  # alpine:3.20
ci/.gitlab-ci.yml:3  updated  ubuntu:24.04 -> ubuntu:26.04@sha256:8083fbb2465d13d61f77932a49d1a1fde3c2b1a09f8e7d6c5b4a39281706f5e4  # ubuntu:24.04
pinned: 0, ok: 0, updated: 2, skipped: 0, unresolved: 0
```

## Tag updates (`--update`)

For every image reference (any registry host), the tag list is fetched
from its registry and the tag is bumped to the **highest stable tag
compatible with the current one** (docker versioning: pre-release/beta/nightly tags are
ignored). Compatibility means the same dash-suffix (e.g. `-alpine`) and the same
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
- **Release-age guard is opt-in.** Renovate's config sets
  `minimumReleaseAge: 14d`; by default pin-ci has no such guard and will
  pick a just-published stable tag — `--min-age-days 14` restores parity
  (see [Release age guard (M4b-1)](#release-age-guard-m4b1)). With the
  guard off, the M2 oracle accepts a tag divergence from the old pipeline
  only as a versioning-policy difference (both tags crane-verified; e.g.
  renovate's `ubuntu` versioning treats odd-minor interludes as unstable).

## Release age guard (M4b-1)

`--min-age-days <N>` (default `0` = off) makes `--update` wait for newly
published tags: a bump candidate must have been pushed at least N days
before the run, per its Docker Hub push timestamp (`tag_last_pushed`,
fallback `last_updated`).

- **Default off.** Without the flag, pin/update behavior is exactly M2 —
  the newest stable tag is picked even if it was pushed minutes ago.
- **Docker Hub refs only.** The guard is active only for `docker.io`
  references (including the no-slash default). References on any other
  host are a documented no-op: M2 update behavior, unchanged.
- **Recommended: `14`.** The old pipeline (`pin-ci-images.sh`, renovate)
  sets `minimumReleaseAge: 14d` in its config — but that rule is scoped to
  `matchUpdateTypes: ['pinDigest','digest']`, so it did not apply to tag
  bumps (proven in the oracle run log); `--min-age-days 14` gates bumps
  too, matching renovate's documented intent.
- **No timestamp = excluded.** A tag missing from the Hub timestamp map
  (neither `tag_last_pushed` nor `last_updated`) is treated as NOT old
  enough (conservative; parity with renovate's `timestamp-required`).
- **No eligible candidate → skip.** When bump candidates exist but none is
  old enough, the reference is left byte-identical and reported as
  `skipped (release age: no candidate ≥ N d)`, exit `0`.
- **Hub API failures are loud.** A 404 repo yields an empty timestamp map
  (→ the skip above); other Hub errors (network, 429, malformed page) make
  the reference `unresolved (hub api: …)`, exit `1`.

## Components & includes (M3)

M3 resolves and pins, in the same run as image pinning:

- **component references** — a top-level (unindented)
  `component: <host>/<group…/project>/<name>@<version>` line. The version
  is resolved to a commit SHA (tag first, then branch) and rewritten as
  `@<40-hex sha>`, with the original version kept as a trailing comment:

  ```yaml
  component: gitlab.com/acme/widget@1.0.0
  # →
  component: gitlab.com/acme/widget@0d7a3b1c2e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b  # 1.0.0
  ```

- **classic includes with an explicit `ref`** — a top-level `include:`
  mapping (block or flow, any key order) carrying both `project` and
  `ref`. The ref is resolved to a commit SHA and replaced, with the
  original ref kept as a trailing comment:

  ```yaml
  include:
    project: gitlab-org/example
    ref: v1.2.3
  # →
  include:
    project: gitlab-org/example
    ref: 0d7a3b1c2e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b  # v1.2.3
  ```

  In a flow include the comment goes **after the closing bracket** (a
  `#` inside `{ … }` would end the flow mapping):

  ```yaml
  include: { project: gitlab-org/example, ref: main }
  # →
  include: { project: gitlab-org/example, ref: 0d7a3b1c2e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b }  # main
  ```

  A line that already has a trailing comment is left as-is (no second
  comment is added). Edits are byte-exact and idempotent — a second run
  reports everything `ok` and writes nothing.

### What gets skipped

Skipped targets are left untouched and keep the exit code `0`:

- **host not enabled** — a component reference whose host differs from
  `--gitlab-host` (default `gitlab.com`): `skipped (host not enabled: <host>)`.
  A classic include's `project` is a project path on the same instance,
  so includes are always eligible (there is no host to compare).
- **catalog-managed versions** — component versions that GitLab's CI/CD
  Catalog resolves at pipeline time and that have no tag/branch to pin
  against: versions starting with `~` (e.g. `~latest`), containing a
  `.x` wildcard (e.g. `1.0.x`), or a **partial** numeric version (all
  digits, fewer than 3 dot-separated components: `1`, `1.0`):
  `skipped (catalog-managed version)`. **Full versions resolve as tags** —
  including full numeric semvers like `1.0.0`; if no such tag (or branch)
  exists the reference is `unresolved` (fail loud). Documented edges:
  `2024` (one numeric component) is catalog-managed; `1.2.3.4` is treated
  as a tag name.
- **templated references** — a ref value containing `$` (e.g.
  `ref: ${CI_REF}`): `skipped (templated reference)`.
- **unparseable component references** — no `@`, too few path segments,
  or an empty version: `skipped (unparseable reference)`.

Never targets at all (not even reported): include mappings with
`project` but no `ref` (bare `{project}`); include items that are plain
local templates (`"local-template.yml"`), `remote:` URLs, or `template:`
names; indented (job-level) `component:` / `include:`.

### GitLab auth

- `GITLAB_TOKEN` — sent as `PRIVATE-TOKEN`; takes precedence.
- `CI_JOB_TOKEN` — sent as `JOB-TOKEN` (for runs inside a pipeline).
  When a job token is the active token, branch resolution uses the list
  endpoint `GET /projects/:id/repository/branches?search=<name>` plus an
  exact-name match (the single-branch endpoint is not in the job-token
  allowlist); PAT/anonymous use the single-branch endpoint.
- Neither set — anonymous; works for public projects.

### Self-hosted instances

`--gitlab-host <host>` (default `gitlab.com`) points component/include
resolution at a self-hosted GitLab. Only component references whose host
equals the configured host are pinned; references on other hosts are
skipped (`host not enabled`).

### Errors & exit codes

Resolution failures are reported per reference and never abort the run —
other targets in the file are still written:

- no tag and no branch for the ref:
  `unresolved (<project>:<ref> not found (no tag or branch))`
- `401`/`403`:
  `unresolved (gitlab api: <status> unauthorized (check GITLAB_TOKEN/CI_JOB_TOKEN))`
- `429`: `unresolved (gitlab api: rate limited (429), retry later)` —
  **no retry** day one; run again later
- other API/network failures: `unresolved (gitlab api: …)`

The exit code is `1` iff at least one reference is `unresolved`; skipped
targets keep exit `0` (M1/M2 exit-code rules unchanged).

## Multi-registry support (M4a)

M4a extends image pinning to **every docker-v2 registry host** —
`ghcr.io`, GCP Artifact Registry, ECR (public and private), JFrog
Artifactory, and any self-hosted registry. The M1 status
`skipped (registry not enabled)` no longer occurs for images.

### Per-host logins

| Registry host | How to authenticate |
|---|---|
| `docker.io` | `DOCKERHUB_USER` + `DOCKERHUB_TOKEN` (env, used first) or `~/.docker/config.json` (M1 auths keys) |
| `ghcr.io` | `GITHUB_TOKEN` (a PAT with `read:packages`), sent as Bearer on the token request, or config.json |
| GCP Artifact Registry (`<region>-docker.pkg.dev`) | `gcloud auth configure-docker <host>` — writes an `identitytoken` entry to config.json, sent as Bearer |
| ECR public (`public.ecr.aws`) | none — anonymous |
| ECR private (`registry-…amazonaws.com`) | `docker login -u AWS -p "$(aws ecr get-login-password …)" registry-…` — lands in config.json. SigV4 credential minting is deliberately **not** in the tool |
| JFrog Artifactory / self-hosted | `docker login <host>` — lands in config.json; both `Basic` and `Bearer` challenges are handled |

config.json entries resolve per host: `username` + `password` → Basic
credentials, otherwise `identitytoken` → Bearer token. Precedence is
per host: env vars first, then config.json. A missing or malformed
config.json is never fatal — the host simply has no credentials.

### Fail loud (no silent skips)

When a registry demands auth — anonymous 401 with the token request
denied, or a `Basic` challenge — and no credentials are configured for
that host, the reference is `unresolved` with a login hint and the exit
code is `1`:

- `ghcr.io`: `unresolved (ghcr.io: token denied — set GITHUB_TOKEN (PAT with read:packages) or docker login ghcr.io)`
- other hosts: `unresolved (<host>: token denied — docker login <host>)`; a `Basic` challenge with no (or Bearer-only) credentials reports `unresolved (<host>: registry requires basic login — docker login <host>)`

Everything else in the file is still resolved and written (partial-write
semantics, M1/M2/M3 rules). `--gitlab-host` (M3) is unaffected: it only
selects the GitLab instance for component/include resolution.

## Environment

Example:

```console
$ pin-ci ci/.gitlab-ci.yml
ci/.gitlab-ci.yml:1  pinned  alpine:3.20 -> alpine:3.20@sha256:25b0e91ad7048476c484f3a5128e2b5d9a4f2c1e8b7a6d5c4f3e2d1a0b9c8d7e  # alpine:3.20
ci/.gitlab-ci.yml:3  pinned  ubuntu:24.04 -> ubuntu:24.04@sha256:8083fbb2465d13d61f77932a49d1a1fde3c2b1a09f8e7d6c5b4a39281706f5e4  # ubuntu:24.04
pinned: 2, ok: 0, skipped: 0, unresolved: 0
```

## Environment

- `DOCKERHUB_USER` / `DOCKERHUB_TOKEN` — `docker.io` credentials, used
  first.
- `GITHUB_TOKEN` — `ghcr.io`, sent as Bearer on the token request.
- Fallback: `~/.docker/config.json`, per host — `auths` keys in order
  `https://index.docker.io/v1/`, `https://docker.io`,
  `index.docker.io/v1/`, `docker.io` for Docker Hub; `ghcr.io` /
  `https://ghcr.io` for GitHub; `{host}` / `https://{host}` for anything
  else. An entry's `username` + `password` are Basic credentials; an
  `identitytoken` is a Bearer token. A missing or malformed config.json
  is never fatal (no credentials for that host).

Nothing secret is written to files or logs.

## Exit codes

- `0` — nothing was `unresolved` (and no IO error).
- `1` — at least one reference was `unresolved`, or an IO error occurred.

## Tests

- `cargo test` — unit, mock-registry, and end-to-end tests. Always runs
  without network, registry credentials, or external tools.
- `PIN_CI_LIVE=1 cargo test --test live` — opt-in live tests against real
  registries: Docker Hub digest resolution for `alpine:3.20` /
  `ubuntu:24.04` plus a full `--update --update-major` run whose tag
  choice is checked against an independent `list_tags` +
  `pick_latest_stable` recomputation and whose digests are checked
  against `crane digest` (a second run must change nothing); a guarded
  `--update --min-age-days 14` run on `alpine:3.19` that must update to a
  ≥ 14-days-old 3.x stable tag (learned live, not frozen) and pin its
  digest, cross-checked against the real Hub push timestamps and `crane
  digest` (skips with a hint when `crane` is missing); ECR public
  anonymous resolution of `public.ecr.aws/nginx/nginx:latest`,
  cross-checked against `crane digest` (skips with a hint when `crane`
  is missing); and a GHCR public-package resolution gated on
  `GITHUB_TOKEN` (skips with a hint otherwise), likewise cross-checked
  against `crane digest`.
- `PIN_CI_ORACLE=1 cargo test --test oracle` — opt-in differential tests
  against the old pipeline, which runs `pin-ci-images.sh` as-is on a temp
  checkout:
  - M1 (pin-only): the new tool must pin exactly the same
    `name@sha256:...` digests the old pipeline wrote.
  - M2 (`--update --update-major` vs. the old pipeline, which bumps tags by
    default): per original reference position (line + repo), equal tags
    need byte-identical digests; a differing tag passes only as the
    age-explained divergence — both tags parse-stable-compatible and the
    new tool's strictly greater, AND the real Hub push timestamps show the
    old pick ≥ 14 days old while the new pick is younger (renovate's
    `minimumReleaseAge: 14d`) — with both digests sanity-checked via
    `crane digest`. A missing Hub timestamp fails loudly (a data bug,
    never a silent pass); anything else fails.

  Both require `renovate`, `crane`, `python3` and `git` on `PATH` (and
  outbound network); without the env flag or the tools they print a SKIP
  notice and pass.

## Old pipeline

`pin-ci-images.sh`, `extract-docker-pins.py`, and `config.js` are kept
unmodified at the repo root and serve as the correctness oracle (see
`PIN_CI_ORACLE=1` above). Their retirement is a later decision.
