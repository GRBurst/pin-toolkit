# Builder check — 2026-11-01-pin-ci-design.md

Reviewed: design doc plus referenced files (`pin-ci-images.sh`,
`extract-docker-pins.py`, `config.js`). Repo state checked: no CI files exist
in this repo (no `ci/`, no `.gitlab-ci.yml`, no test fixtures); only the three
oracle files + design doc.

Verdict: **not rejectable** — the doc is plan-ready except for the questions
below. Ranked by how much a wrong guess would change the plan.

## Q1 (High) — Is GitLab's `default:` block in G1 scope?

G1 enumerates top-level `image`, per-job `image`, and `services`; the editor
spec repeats the same set. Both omit the `default:` block
(`default: { image: ..., services: ... }`), a common real-world pattern. The
oracle pipeline runs Renovate's gitlabci manager, which also extracts
`default.image`/`default.services` (not verifiable locally here — no renovate
installed — but that is my recollection of the manager's source). If the real
files the old pipeline ran against contain `default:`, the differential oracle
contradicts G1's enumeration.

Why it changes the plan: (a) target-set/discovery in `edit` grows a node type;
(b) oracle expectations — parity vs a documented, expected divergence; (c) if I
guess out-of-scope, references under `default:` are silently unpinned, which
is exactly the "never silently ignore" anti-goal. If I guess in-scope against
the owner's intent, it is easy scope creep to remove; the reverse is an oracle
failure at the owner's required gate.

## Q2 (High) — Should `--update` respect a minimum release age?

The decided update policy says "highest *stable* tag", no age guard. But the
referenced `config.js` carries `minimumReleaseAge: '14 days'` +
`timestamp-required` (inert there, since matching was pinDigest/digest only,
but it shows the owner cared about not chasing fresh pushes).

Why it changes the plan: an age guard requires tag *timestamps*, which Docker
Hub's v2 registry API (the two-endpoint surface the design commits to) does
not expose — the guard means integrating the Hub API (`hub.docker.com/v2/...`)
as a third backend, with its own rate limits and auth, plus "no timestamp →
refuse" semantics. Answer "no guard" keeps the client at two endpoints and the
policy as written; answer "guard like before" adds a module, auth path, and a
whole test surface.

## Q3 (Medium) — Which classic `include` shapes are day-one in scope?

G2 names `include: { project, ref }`. Legal classic forms also include
`{ project }` alone (resolves to the project's default branch — the GitLab
client would then need the project's default branch, another endpoint) and
`{ project, ref, file }` (`file` can be a string or a list).

Why it changes the plan: discovery rules in `edit` and the GitLab client
surface (default-branch lookup). If I guess too narrow, common real-world
includes surface as skipped/unresolved noise — unresolved drives exit 1, which
would red a day-one CI job on files the owner expects to pass. If I guess too
wide, I ship endpoints the owner didn't ask for.

## Q4 (Medium) — Does `--update` ever bump a component ref that is a tag?

The policy says branch refs never bump, but is silent on tag refs
(e.g. `ref: v1`). Read literally, tag refs could bump; read charitably,
components are re-pin-only.

Why it changes the plan: bumping means applying docker versioning to component
tags, listing tags of include projects via the GitLab API, and shipping an
auto-version-bump for include files — a meaningfully riskier behavior class
than re-pinning (a bumped include can break a pipeline; a re-pin cannot).
Wrong guess = shipping an unwanted auto-bump, or leaving a promised feature
out.

## Q5 (Low) — Tagless `image: alpine` (implicit `latest`): pin or leave?

Affects editor formatting rules (normalize to `alpine:latest@sha256:` as
Renovate would, or append a digest without adding `:latest`, or skip). Digest
identity for the oracle is unaffected either way; this only shapes the diff
and skip-reporting.

## Notes (not blocking; decided by builder)

- Repo has no CI files or fixtures: oracle/differential fixtures will be
  built synthetically from the shapes listed in Testing; the final
  "real files" gate at retirement presumably needs owner-provided files.
  Plan is unchanged by this.
- `skipped` vs exit code: doc's own three-state vocabulary (pinned /
  skipped / unresolved) and the exit-1 condition ("any reference
  *unresolved*") imply skipped ⇒ exit 0; I will plan on that reading.
