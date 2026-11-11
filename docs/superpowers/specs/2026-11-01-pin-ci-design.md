# pin-ci — GitLab CI image & component pinning CLI

> **Superseded** by [2026-11-09-pin-toolkit-design.md](./2026-11-09-pin-toolkit-design.md)
> (pin-toolkit). Kept as a historical record of the original `pin-ci`
> design; the tool it describes has been replaced by pin-toolkit.

Status: draft — superseded, see banner above
Date: 2026-11-01

## Intent and why

GitLab CI files reference container images and CI/CD components by mutable
tags or branches. Rebuilding the same commit can silently pull a different
image or component. `pin-ci` resolves each reference to an immutable hash —
image digest for containers, commit SHA for components — and rewrites the CI
file in place, keeping the original tag/ref visible.

It replaces the current pipeline in this repo (`pin-ci-images.sh` +
`extract-docker-pins.py` + `crane` + `sed`), which works for Docker images but:

- does not handle components at all — Renovate's gitlabci manager does not
  extract the classic `include: { project, ref }` form (verified against the
  Renovate source; it only handles the new-style
  `include: <host>/<group>/<project>/<file>@<version>` form);
- drags in Node + the full Renovate stack for what is essentially two
  registry API calls and one GitLab API call;
- parses Renovate's log output with regex and rewrites files with `sed` —
  brittle against anything but a narrow file shape.

Users: developers running the CLI locally, or a CI job (scheduled or on merge).
CLI only: no daemon, no service, no configuration server.

## Goals

- G1: Pin image references — top-level `image`, `default: { image, services }`,
  per-job `image`, `services` (string and map forms) — to `tag@sha256:<digest>`. Day one: images hosted on
  Docker Hub. References on other registry hosts are detected and reported as
  skipped, never silently ignored.
- G2: Pin `include` components to the full commit SHA, original ref preserved
  as a comment. Day one: classic `include` with an explicit ref —
  `{ project, ref }` and `{ project, ref, file }`, any key order — and
  new-style `include: <host>/<group>/<project>/<file>@<version>`, against
  gitlab.com (host overridable for self-hosted instances). Bare `{ project }`
  (implicit default branch) is reported `skipped`, not resolved, day one.
- G3: `--update` mode: resolve to a newer tag first, then pin (policy below).
- G4: Pin-only is the default: tags unchanged, digests/SHAs (re-)resolved.
- G5: Line-level, idempotent edits: comments, formatting, ordering preserved.
  A second run on an unchanged repo makes zero changes (except re-pinning a
  digest whose upstream tag was re-pushed).
- G6: Human-readable report + `--json` output; CI-friendly exit codes.
- G7: Correctness for images proven by differential testing against the
  existing pipeline (the oracle the owner wants kept) plus mock-registry tests.

## Non-goals

- Version bumps that aren't requested via `--update` (never implicit).
- Editing non-GitLab-CI files; remote-URL `include:` entries; local file
  `include:` entries (reported as out of scope, not modified).
- Per-platform (single-arch) digests — we pin the manifest-list digest, which
  is what registries return and what pulls use (same behavior as `crane` and
  Renovate).
- Day one: GHCR, GCP Artifact Registry, ECR images; self-hosted GitLab is
  addressable via config but day-one tests cover gitlab.com only.
- Scheduling, git hooks, web UI, Renovate plugin.

## Anti-goals (failure states)

- Any reformat, reordering, or comment loss in the CI file: diffs must touch
  only the pinned lines.
- Silently skipping an unresolvable reference — every reference ends in a
  reported state (pinned / skipped-with-reason / unresolved).
- Pulling in a runtime (Node) or a heavy dependency tree.

## Constraints

- Single static Rust binary, no runtime dependency, runs on a dev machine or
  in any CI image. (Owner decision.)
- Must produce digests identical to the existing pipeline for the same tags —
  the oracle requirement.
- Credentials come from the environment or `~/.docker/config.json`; nothing
  secret is ever written to the CI file.
- CI files are comment-rich: line-level edits only, no full YAML
  parse-and-rewrite.

## Approaches

- **A. Purpose-built Rust CLI (chosen, owner pick).** Own YAML-target
  discovery, a small registry client, a GitLab API client, one binary for
  images and components. The registry surface is two endpoints
  (`HEAD /v2/<repo>/manifests/<tag>` → `Docker-Content-Digest` header;
  `GET /v2/<repo>/tags/list` with Link pagination) plus the
  WWW-Authenticate bearer-token dance — ~200 lines against `reqwest` +
  `serde`, risk covered by the differential oracle.
- **B. Thin orchestrator over `crane` + `git ls-remote` (current stack).**
  Fastest start, but `--update` needs tag listing regardless, and shell-based
  editing becomes fragile once multi-line `include:` mappings are in scope.
- **C. Keep Renovate as the resolver backend.** Rejected: heavy, and it has
  exactly the component gap we are filling.

Why A: one language and one binary for both use cases; the parts we must get
right are small enough to validate against the oracle.

## Decided (owner)

- Rust, single CLI, one tool for images and components.
- Pin-only by default; `--update` flag for tag bumps.
- Day-one scope: Docker Hub images + gitlab.com components.
- The existing pipeline stays in the repo and serves as the correctness
  oracle in tests.

## Decided (my calls — flag any you disagree with)

- **CLI surface**
  `pin-ci [FILES...]` (default `ci/.gitlab-ci.yml`), `--update`, `--dry-run`,
  `--json`, `--gitlab-host <host>` (default `gitlab.com`),
  `--concurrency <n>` (default 8).
  Exit codes: `0` every reference ends in `pinned` or `skipped(reason)`
  (skipped = expected, e.g. registry not enabled day one); `1` on error or
  any `unresolved` reference (all reported by name). `--dry-run` prints the
  planned rewrites and changes nothing.
- **Auth.** Anonymous probe first (all day-one hosts allow anonymous pulls
  and public-project reads). On 401: parse `WWW-Authenticate: Bearer
  realm=...`, fetch a token with scope `repository:<repo>:pull`
  (Docker Hub: `auth.docker.io`, token in `~/.docker/config.json` or
  `DOCKERHUB_USER`/`DOCKERHUB_TOKEN`; GitLab: `GITLAB_TOKEN` or `CI_JOB_TOKEN`).
  No registry credentials → references requiring them are reported unresolved.
- **Pin format.**
  Images: `alpine:3.21@sha256:...` — the tag stays in the value, no comment
  (matches the current tool's output). Tagless `image: alpine` is rewritten
  to `alpine@sha256:...` (no explicit `:latest`).
  Components: `ref: <40-hex-sha>  # was <original ref>`.
  New-style `include: ...@<version>`: version replaced by the commit SHA,
  original version in a comment. (Builder to verify GitLab accepts a commit
  SHA as the `@version` in new-style includes; if not, leave the new-style
  form untouched and report it — flagged as a verify item, not day-one
  promise.)
- **Re-pin semantics.** An already-pinned `tag@digest` whose current registry
  digest differs (tag re-pushed) is re-pinned in place, tag unchanged,
  immediately — following a re-push has no release-age guard.
  Already-pinned components are re-resolved and updated if the ref moved.
- **Update policy (`--update`).** Highest *stable* tag under docker
  versioning (semver-like; pre-releases/betas/nightlies ignored), restricted
  to the same major as the current tag by default; `--update-major` lifts the
  restriction. Non-semver tags (`latest`, `main`, `rolling`) never bump —
  digest/SHA re-pin only. Branch refs on components never bump. Whether
  component *tag* refs bump is open (see Open below).
- **Editor.** Discovery via a hand-rolled line scanner over top-level/job
  `image`, `services` entries (incl. `name:` in map form) and, from M3,
  `include` string and mapping forms (incl. multi-line `include: { project,
  ref }`), then line-anchored rewrites keyed on byte spans. Mechanism
  refined from the originally proposed position-reporting YAML parse: same
  outcome, no parser dependency, less risk. Multi-line mapping forms get the
  comment on the `ref:` line. Untouched lines are byte-identical.
- **Concurrency.** Resolutions run in parallel (tokio); file writes are
  sequential, one file at a time.
- **Report.** One line per reference: `file:line  target  old → new  status`
  (status: `pinned`, `skipped(<registry not enabled>)`, `unresolved(<reason>)`),
  plus a summary line. `--json` emits the same data machine-readable.
- **Repo layout.** Single cargo crate in this repo, binary named `pin-ci`.
  Old pipeline files stay untouched (oracle). New modules: `registry`
  (client), `gitlab` (client), `versioning`, `edit`, `report`, `cli`.
- **Config.** Environment variables only in day one: `DOCKERHUB_USER`,
  `DOCKERHUB_TOKEN`, `GITLAB_TOKEN` (or `CI_JOB_TOKEN`), plus CLI flags.
  No config file.
- **Builder-check settlements (my calls, 2026-11-01).** `default:` block in
  scope (G1); explicit-ref includes pinned in any key order incl. `file`
  (G2); tagless-image normalization (pin format); `skipped` exits 0, only
  `unresolved`/error exits 1 (CLI surface); re-pin follows immediately
  (re-pin semantics).

## Open

- **Component tag refs under `--update`.** Resolved 2026-11-05 (controller,
  owner may override): M3 is **pin-only** for components — mirroring the M1/M2
  image split (pin first, `--update` as a separate later step). Bumping
  component versions under `--update` is deferred to M4 follow-ups.
- **Minimum release age.** Decided 2026-11-01: NOT implemented day one.
  Follow-up (M4): the old config's `minimumReleaseAge: 14d` would need tag
  timestamps, which Docker Hub's v2 API does not expose (third API, Hub API,
  with `tag_last_pushed`).

## Left to the builder

- Rust library choices within the stack above (reqwest vs hyper; YAML
  parser: needs per-node line positions — e.g. a position-reporting parser
  such as `yaml-nrust`-style marks, or a hybrid hand-rolled scanner for the
  narrow key set; validate against the fixtures in Testing).
- Docker versioning rules: port the minimal subset of Renovate's docker
  versioning (validity, stability, major/minor/patch comparison, edge tags)
  into `versioning` with tests — do not vendor the full scheme.
- Exact endpoint behaviors against live public instances (Docker Hub,
  gitlab.com) — the design quotes Renovate's code paths; confirm during
  implementation, especially Docker Hub's token host and `Link` pagination,
  and GitLab's `repository/commits/:ref` resolving both tags and branches.
- Graceful handling of YAML edge cases (anchors, multi-doc files): report,
  don't guess.

## Testing

- **Unit.** Versioning (stable/highest/same-major selection, edge tags);
  editor fixtures (comments, anchors, multi-line includes, `services` map
  form, already-pinned, stale digest, byte-identical untouched lines);
  registry client and GitLab client against a mock HTTP server (digest
  header, Link pagination, 401→bearer→retry, tag/branch→SHA, 404).
- **Integration (opt-in, network).** Public Docker Hub images (e.g. `alpine`,
  `ubuntu`) and a public gitlab.com component include.
- **Differential oracle (opt-in; needs node + renovate + crane + network).**
  Same fixture CI file → old pipeline vs `pin-ci` → digests must match
  exactly. This is the owner's required correctness check before the old
  tool is retired.
- **Idempotency.** Running twice: the second run reports zero changes.

## Milestones (small steps, per owner 2026-11-01)

- **M1 — Docker Hub image pinning (pin-only).** Scope: registry client
  (anonymous + bearer), discovery (top-level `image`, `default:` block,
  per-job `image`, `services`), line editor, report / `--json` / `--dry-run`,
  exit codes, idempotency, mock-HTTP tests, opt-in live tests, differential
  oracle against the old pipeline. No `--update`; non-Docker-Hub hosts are
  detected and reported `skipped`. Plan: `docs/superpowers/plans/2026-11-01-pin-ci-m1-docker-hub-pinning.md`.
- **M2 — `--update` for images.** Docker-versioning module (minimal
  Renovate subset), same-major stable bump, `--update-major`.
- **M3 — gitlab.com components (pin-only).** GitLab client (ref→commit for
  tags and branches), classic includes (explicit `ref`, any key order, incl.
  `file`; bare `{project}` stays out) and new-style component references
  (`gitlab.com/<group>/<project>/<file>@<version>`), pinned to the commit
  SHA, comment format; pin-only (see Open above).
- **M1 — DONE** (merged ad42721).
- **M2 — DONE** (merged 543d56a).
- **M3 — DONE** (merged ea43e2e; branch m3-components, 8 commits 5379740..a7cad28).
- **M4a — multi-registry support (in progress, 2026-11-07).** Every
  docker-v2 host resolves by default (the M1 `skipped (registry not
  enabled)` image status is gone): GHCR, GCP Artifact Registry, ECR
  public (anonymous-verified), ECR private via `docker login` config
  (SigV4 minting deferred), **JFrog Artifactory** and any self-hosted
  registry (docker v2; client handles both `Bearer` and `Basic`
  WWW-Authenticate — JFrog commonly sends Basic). Per-host credentials:
  `DOCKERHUB_USER`/`DOCKERHUB_TOKEN` (hub), `GITHUB_TOKEN` (ghcr, Bearer
  on token request), `~/.docker/config.json` for all hosts (username/
  password → Basic; identitytoken → Bearer, the gcloud path). Host
  demanding auth with no configured credentials → `unresolved` with a
  login hint, exit 1 (fail-loud ruling 2026-11-07). `--update` works for
  all hosts (shared auth path). Design:
  `.superpowers/sdd/2026-11-07-pin-ci-m4a-multi-registry/design.md`.
- **M4b-1 — minimum release age guard (in progress, 2026-11-09).**
  `--min-age-days <N>` (default 0 = off; only meaningful with
  `--update`): a `--update` bump candidate must be at least N days old.
  Timestamps come from the Docker Hub API
  (`hub.docker.com/v2/repositories/{ns}/{repo}/tags` — `last_updated` /
  `tag_last_pushed` per tag, `next` pagination, 180 req/h per IP —
  probe-verified 2026-11-09); the guard applies to **docker.io refs only**
  (the only host with push timestamps; other hosts: documented no-op,
  per owner's small-steps ruling — overridable). No eligible candidate →
  `skipped (release age: no candidate ≥ {N}d)`, exit 0. Ruling: default
  OFF (M1-M4a contract: pin is default, update is opt-in; the old
  pipeline's 14d is documented as the recommended value, not a default).
  Oracle: same-day differential vs the old pipeline (config.js
  `minimumReleaseAge: '14 days'`, `timestamp-required`) — the M2
  documented divergence (ubuntu 26.04 vs 26.10) becomes a matching pick
  when both see the same ages. Design:
  `.superpowers/sdd/2026-11-09-pin-ci-m4b1-release-age/design.md`.
- **M4b-2 — `--update` for gitlab.com components/includes (later, own
  plan).** Bump `@<tag>` component versions and tag-valued include refs
  to the same-major latest stable tag (GitLab tags list,
  `order_by=version`), re-resolve to commit SHA; branch refs and
  non-semver tags are not bump targets.
- **Deferred (recorded 2026-11-09): self-hosted GitLab live test
  coverage.** No self-hosted instance is available in this
  environment; the wiremock suite covers the API contract and
  `--gitlab-host` makes the host switch a one-liner. Revisit when a
  real instance exists.

## Future (out of scope, noted 2026-11-05)

- **Helm chart pinning — separate tool.** Owner wants a *separate* tool
  later to pin Helm chart references (chart URLs / OCI chart digests,
  `Chart.yaml` dependencies, `requirements`). Not part of pin-ci; scope and
  plan it separately when picked up.

The old pipeline is retired only after the differential oracle has passed on
real files and the owner says so.
