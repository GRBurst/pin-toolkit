# Plan: df1-gitlab-smoke
<!-- task-router plan v1. Edit only checkboxes; change the plan with amend_plan. -->
Request: Implement DF1 part 1 of docs/superpowers/specs/2026-11-09-pin-toolkit-design.md (S10 DF1 row, S8.4): GitLab smoke pipelines on the default branch and v* tags that run the released ghcr.io/grburst/pin-toolkit image inside GitLab CI. The design with every decision, the full file texts (verified locally) and the GitLab doc citations is <main checkout>/.superpowers/sdd/2026-11-12-df1-gitlab/design.md (not tracked by git: read it from the main checkout, /home/pallon/projects/pin-ci-images, also from a worktree) — READ IT FIRST; copy its §2 files, do not redesign them. M2 stacks on M1; the owner merges each by hand and pushes to gitlab.com himself (no push, no tag in this plan).
Invariants:
  - No product change: no Rust, Cargo, snapshot or test edits; ci/.gitlab-ci.yml, pin-ci-images.sh, extract-docker-pins.py, config.js (oracle inputs) and .github/workflows/ci.yml (security-reviewed release path) are byte-identical at the end (`git diff --quiet b63c04b -- ci/.gitlab-ci.yml pin-ci-images.sh extract-docker-pins.py config.js crates .github`)
  - A pin-toolkit bug found while doing this is reported to the orchestrator (file, command, output), never fixed here (brief decision 5); known and reported only: `include: - component:` is not a target before G1 (spec C3b)
  - Every image in .gitlab-ci.yml is digest-pinned by pin-toolkit itself (write the tag, run `$PT .gitlab-ci.yml`, never hand-copy a digest), except release-image's interpolated image (design §3: it is the artefact under test; reported `skipped (unparseable reference)`)
  - ci/smoke/pins.yml stays unpinned in git; only copies under smoke-out/ or .tmp/ are pinned; no task check may leave it modified (`git diff --quiet -- ci/smoke/pins.yml`)
  - Task checks run locally (no GitLab); the network ones resolve anonymously against ghcr.io, public.ecr.aws and gitlab.com. No CI/CD variables, tokens or secrets are introduced anywhere
  - Every new check is seen RED once with its planted violation (listed in the task) before it is accepted; note "planted: <what> -> red seen" in the commit body; revert the plant
  - Nix sees git-tracked files only: a nix check stages only its own task's paths first (`git add -A -- <paths>`, never a bare `git add -A`) and ends with `git diff --cached --quiet`; nix commands use `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache`; never evaluate the repo as `path:.`
  - Shell in .gitlab-ci.yml and ci/smoke/smoke.sh is POSIX sh for busybox ash (the image has no bash): `shellcheck -s sh` clean
  - bash scripts/gate.sh is untouched and stays green; each milestone diff <= 400 changed lines

## M1: GitLab smoke pipeline: .pin-toolkit.toml (dogfood scope), ci/smoke/{pins.yml,smoke.sh}, root .gitlab-ci.yml (dogfood, smoke, delayed release-image via pipeline input), offline flake check gitlab-ci (~190 lines). (branch: milestone; depends: —)
Acceptance: repo-wide `pin-toolkit --check` walks only .gitlab-ci.yml (ci/.gitlab-ci.yml excluded by `files`, which replaces the defaults) and exits 0 with summary pinned 0, ok 2, skipped 1, unresolved 0; ci/smoke/smoke.sh pins a copy of ci/smoke/pins.yml (4 pinned: ECR image digest, foreign component SHA, own include main == current main SHA, v0.1.0 == b63c04bea5c4376d34f9597cd357271bc654fb30) and a second --check run is clean (4 ok); workflow: only push pipelines of the default branch and v* tags; dogfood/smoke on the branch, release-image only on v* tags as a delayed job (45 minutes); checks.<system>.gitlab-ci (offline) green and seen red; scripts/nix-check.sh green
- [x] M1.T1: Create .pin-toolkit.toml exactly as design §2.1 (`version = 1`, `[plugins.gitlab-ci] files = [".gitlab-ci.yml", ".gitlab/ci/**/*.yml"]`, comment naming ci/.gitlab-ci.yml as the oracle input). Planted: delete the file -> `--check` exit 1 (ci/.gitlab-ci.yml rows, ghcr.io/docker/buildx unresolved).
  files: .pin-toolkit.toml (new) · forbidden: `ignore` patterns (they are per reference, not per file); a .ignore file (hides ci/ from ripgrep/fd too); editing or moving ci/.gitlab-ci.yml; keys beyond version and plugins.gitlab-ci.files · check: `mkdir -p .tmp && export CARGO_HOME="$(git rev-parse --path-format=absolute --git-common-dir)/../.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH" && cargo build -q --locked -p pin-toolkit && PT="$PWD/target/debug/pin-toolkit" && $PT config show | grep -qxF 'plugins.gitlab-ci.files = [".gitlab-ci.yml", ".gitlab/ci/**/*.yml"]' && $PT --check --json > .tmp/df1-t1.json && jq -e '[.results[].file, .plugins[].errors[].file] | all(. == ".gitlab-ci.yml" or startswith(".gitlab/ci/"))' .tmp/df1-t1.json > /dev/null` · depends: —
- [x] M1.T2: Create ci/smoke/pins.yml and ci/smoke/smoke.sh verbatim from design §2.3/§2.4 (copy from the design, keep the v0.1.0 SHA literal and the two summary literals). Planted: (a) `alpine:3.20` -> `alpine:0.0.0-nope` in pins.yml -> `smoke: FAIL: pin-toolkit gitlab-ci … exited 1`; (b) CI_COMMIT_SHA=0000000000000000000000000000000000000000 -> `smoke: FAIL: no line matching /    ref: 000…0  # main/`; (c) unquoted "$f" in smoke.sh -> shellcheck SC2086 red.
  files: ci/smoke/pins.yml (new), ci/smoke/smoke.sh (new) · forbidden: Docker Hub images in the fixture (anonymous rate limit); a fixture name matching `*.gitlab-ci.yml` or a path under .gitlab/ci/ (the repo-wide run would see it); bash-isms, jq or curl in smoke.sh (the image has busybox sh, grep only); pinning ci/smoke/pins.yml in place · check: `shellcheck -s sh ci/smoke/smoke.sh && export CARGO_HOME="$(git rev-parse --path-format=absolute --git-common-dir)/../.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH" && cargo build -q --locked -p pin-toolkit && PT="$PWD/target/debug/pin-toolkit" && rm -rf .tmp/df1-smoke && PATH="$PWD/target/debug:$PATH" CI_COMMIT_BRANCH=main CI_COMMIT_SHA=$(git ls-remote https://gitlab.com/GRBurst/pin-toolkit.git refs/heads/main | cut -f1) sh ci/smoke/smoke.sh .tmp/df1-smoke | tail -1 | grep -qx 'smoke: OK' && git diff --quiet -- ci/smoke/pins.yml && $PT --check --json | jq -e '[.results[].file] | all(startswith("ci/") | not)' > /dev/null` · depends: M1.T1
- [x] M1.T3: Create .gitlab-ci.yml from design §2.2: write both branch-job images as `ghcr.io/grburst/pin-toolkit:0.1.0`, then run `$PT .gitlab-ci.yml` to pin them (dogfood; expect `pinned: 2, … skipped: 1`); keep the spec header (input release-tag, default $CI_COMMIT_TAG), workflow rules (push && default branch; push && tag =~ /^v[0-9]/), auto_cancel interruptible, default interruptible + timeout 5m, GIT_DEPTH 1, jobs dogfood/smoke (rules if $CI_COMMIT_BRANCH) and release-image (image ghcr.io/grburst/pin-toolkit:$[[ inputs.release-tag | expand_vars | truncate(1,64) ]], rule if tag, when delayed, start_in 45 minutes, interruptible false), artifacts when always / 30 days. Planted: (a) strip one @sha256 digest -> `--check` exit 1; (b) `&&` -> `||` in a workflow rule -> yq false.
  files: .gitlab-ci.yml (new) · forbidden: hand-copied digests; MR, schedule, web or other-branch pipelines (decision 1); `when`/`start_in` at job level next to rules (GitLab rejects it); polling loops or sleep for the release race; CI/CD variables named GITLAB_TOKEN/GITHUB_TOKEN/PIN_TOOLKIT_*; Docker Hub images; an `ignore` entry for the release-image line (it is skipped, exit 0) · check: `export CARGO_HOME="$(git rev-parse --path-format=absolute --git-common-dir)/../.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH" && cargo build -q --locked -p pin-toolkit && PT="$PWD/target/debug/pin-toolkit" && mkdir -p .tmp/df1 && yq "select(document_index == 0)" .gitlab-ci.yml > .tmp/df1/head.yml && yq "select(document_index == 1)" .gitlab-ci.yml > .tmp/df1/jobs.yml && yq -e '.spec.inputs.release-tag.default == "$CI_COMMIT_TAG"' .tmp/df1/head.yml > /dev/null && yq -e '(.workflow.rules | length) == 2 and ([.workflow.rules[].if | test("^\$CI_PIPELINE_SOURCE == \"push\" && ")] | all)' .tmp/df1/jobs.yml > /dev/null && yq -e '.release-image.rules[0] | ((.["if"] == "$CI_COMMIT_TAG =~ /^v[0-9]/") and (.when == "delayed") and (.start_in == "45 minutes"))' .tmp/df1/jobs.yml > /dev/null && [ "$(grep -cE '^  image: ghcr\.io/grburst/pin-toolkit:[0-9][^@ ]*@sha256:[0-9a-f]{64}$' .gitlab-ci.yml)" = 2 ] && for j in dogfood smoke release-image; do yq ".\"$j\".script | join(\"\n\")" .tmp/df1/jobs.yml > .tmp/df1/$j.sh && shellcheck -s sh .tmp/df1/$j.sh || exit 1; done && $PT --check --json > .tmp/df1-t3.json && jq -e '.summary == {"pinned":0,"ok":2,"updated":0,"skipped":1,"unresolved":0} and ([.results[].file] | unique) == [".gitlab-ci.yml"]' .tmp/df1-t3.json > /dev/null` · depends: M1.T2
- [ ] M1.T4: nix/checks.nix: add `gitlab-ci` (runCommand; nativeBuildInputs shellcheck yq-go; src = lib.fileset.toSource { root = ../.; fileset = lib.fileset.unions [ ../.gitlab-ci.yml ../.pin-toolkit.toml ../ci/smoke ]; }) running, offline, the T3 yq assertions (split documents into $TMPDIR first), the grep count of 2 digest-pinned images, shellcheck -s sh of every `script:` block (`yq '.[] | select(tag == "!!map" and has("script")) | key'`) and of ci/smoke/smoke.sh, and `HOME=$TMPDIR ${pkg}/bin/pin-toolkit --config .pin-toolkit.toml config show | grep -qxF 'plugins.gitlab-ci.files = [...]'`. Planted: `start_in: 1 hour` -> red; an unpinned branch image -> red; `cat $f` in smoke.sh -> red.
  files: nix/checks.nix · forbidden: network access in the check (pin-toolkit --check, smoke.sh stay out of Nix); including more than the three source paths in the fileset; changing existing checks · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- .gitlab-ci.yml .pin-toolkit.toml ci/smoke nix/checks.nix && nix build --no-link -L .#checks.x86_64-linux.gitlab-ci && bash scripts/nix-check.sh && git diff --cached --quiet` · depends: M1.T3

## M2: Docs and close-out: README CI image section points at the real .gitlab-ci.yml (GHCR public now), spec DF1 row + S8.4 as built (DF1) + D60/D61 + owner steps (~60 lines). (branch: milestone; depends: M1)
Acceptance: README CI image section links .gitlab-ci.yml as this repository's working example, says the image is public, and names ci/smoke/smoke.sh; readme anchor check green; spec: D60 (dogfood scope via repo files globs) and D61 (tag job: pipeline input + expand_vars/truncate, delayed 45 min; fallback tarball in pinned alpine) rows; S8.4 'As built (DF1)' bullet; DF1 row says part 1 done and what remains; owner steps from design §7
- [ ] M2.T1: README.md `## CI image`: replace `The image is public once the owner has set the GHCR package to public.` with: the image is public; this repository's own [.gitlab-ci.yml](.gitlab-ci.yml) is a working example (dogfood --check with a JSON report, ci/smoke/smoke.sh end-to-end pin + idempotence, delayed release-image on v* tags whose image cannot carry a digest yet); `.pin-toolkit.toml` keeps ci/.gitlab-ci.yml (oracle sample) out of the repo-wide run. Tests section: one line naming ci/smoke/smoke.sh and how to run it locally.
  files: README.md · forbidden: rewriting unrelated README sections; `--format json` (the flag is --json); claiming the job token was needed (public projects resolve anonymously too) · check: `bash scripts/check-readme-anchors.sh && ! grep -q 'The image is public once' README.md && grep -q '(\.gitlab-ci\.yml)' README.md && grep -q 'ci/smoke/smoke.sh' README.md && ! grep -q -- '--format json' README.md` · depends: —
- [ ] M2.T2: Spec docs/superpowers/specs/2026-11-09-pin-toolkit-design.md: S8.4 bullet 'As built (DF1 part 1)' (files, jobs, exception for release-image, what the smoke proves/does not prove: design §4); D60 and D61 rows (design §1 fact 2, §3); DF1 row: 'part 1 (GitLab smoke pipelines) done in <branch>; owner-driven dogfooding continues'; S13/O-steps: owner steps design §7 (CI/CD + instance runners enabled, config path default, project stays public, no token variables, optional CI Lint dry run, fallback B if GitLab rejects the input); S14: the G1 gap observed in DF1 (`include: - component:` not pinned).
  files: docs/superpowers/specs/2026-11-09-pin-toolkit-design.md · forbidden: rewriting unrelated spec sections; renumbering existing decisions; marking DF1 as done (it stays open, owner-driven) · check: `S=docs/superpowers/specs/2026-11-09-pin-toolkit-design.md && grep -q '^| D60 ' $S && grep -q '^| D61 ' $S && grep -q 'As built (DF1' $S && grep -E '^\| DF1 ' $S | grep -q 'part 1' && bash scripts/check-readme-anchors.sh` · depends: M2.T1

## GitLab-only verification (after the owner merges and pushes to gitlab.com; not router tasks)
See design §4, §5 (CI Lint dry run), §7. Owner first: CI/CD and instance runners enabled, CI/CD configuration file empty (default). Optional before pushing: CI Lint API dry run for ref `main` (jobs dogfood + smoke) and ref `v0.1.0` (one job release-image, when delayed; merged YAML shows `ghcr.io/grburst/pin-toolkit:0.1.0`) — this is the first proof that `$[[ inputs.release-tag | expand_vars | truncate(1,64) ]]` is accepted and yields the tag without `v`. On the first main push: pipeline `push` with dogfood + smoke green; pin-report.json artifact summary pinned 0, ok 2, skipped 1, unresolved 0; smoke log shows `smoke: OK` and the `main` include pinned to the pipeline's CI_COMMIT_SHA (job token sent to gitlab.com: accepted on the tags and branches-list endpoints). On the next v* tag (owner): release-image shown as delayed (45 min), then green with `pin-toolkit <ver>`; if it fails at image pull because GitHub is still releasing, Retry. If GitLab rejects the spec header/interpolation (lint error, or the tag job pulls `…:CI_COMMIT_TAG`/a v-prefixed tag): switch release-image to fallback B (design §3: tarball + .sha256 in the pinned ECR alpine image; drop the spec header) as a follow-up commit; ci.yml stays untouched. Usage quotas page after a week: per-job minutes vs. the ≤ 1 min estimate (design §6).

<details><summary>skeleton</summary>

```json
{
  "invariants": [
    "No product change: no Rust, Cargo, snapshot or test edits; ci/.gitlab-ci.yml, pin-ci-images.sh, extract-docker-pins.py, config.js (oracle inputs) and .github/workflows/ci.yml (security-reviewed release path) are byte-identical at the end (`git diff --quiet b63c04b -- ci/.gitlab-ci.yml pin-ci-images.sh extract-docker-pins.py config.js crates .github`)",
    "A pin-toolkit bug found while doing this is reported to the orchestrator (file, command, output), never fixed here (brief decision 5); known and reported only: `include: - component:` is not a target before G1 (spec C3b)",
    "Every image in .gitlab-ci.yml is digest-pinned by pin-toolkit itself (write the tag, run `$PT .gitlab-ci.yml`, never hand-copy a digest), except release-image's interpolated image (design §3: it is the artefact under test; reported `skipped (unparseable reference)`)",
    "ci/smoke/pins.yml stays unpinned in git; only copies under smoke-out/ or .tmp/ are pinned; no task check may leave it modified (`git diff --quiet -- ci/smoke/pins.yml`)",
    "Task checks run locally (no GitLab); the network ones resolve anonymously against ghcr.io, public.ecr.aws and gitlab.com. No CI/CD variables, tokens or secrets are introduced anywhere",
    "Every new check is seen RED once with its planted violation (listed in the task) before it is accepted; note \"planted: <what> -> red seen\" in the commit body; revert the plant",
    "Nix sees git-tracked files only: a nix check stages only its own task's paths first (`git add -A -- <paths>`, never a bare `git add -A`) and ends with `git diff --cached --quiet`; nix commands use `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache`; never evaluate the repo as `path:.`",
    "Shell in .gitlab-ci.yml and ci/smoke/smoke.sh is POSIX sh for busybox ash (the image has no bash): `shellcheck -s sh` clean",
    "bash scripts/gate.sh is untouched and stays green; each milestone diff <= 400 changed lines"
  ],
  "milestones": [
    {
      "acceptance": [
        "repo-wide `pin-toolkit --check` walks only .gitlab-ci.yml (ci/.gitlab-ci.yml excluded by `files`, which replaces the defaults) and exits 0 with summary pinned 0, ok 2, skipped 1, unresolved 0",
        "ci/smoke/smoke.sh pins a copy of ci/smoke/pins.yml (4 pinned: ECR image digest, foreign component SHA, own include main == current main SHA, v0.1.0 == b63c04bea5c4376d34f9597cd357271bc654fb30) and a second --check run is clean (4 ok)",
        "workflow: only push pipelines of the default branch and v* tags; dogfood/smoke on the branch, release-image only on v* tags as a delayed job (45 minutes)",
        "checks.<system>.gitlab-ci (offline) green and seen red; scripts/nix-check.sh green"
      ],
      "branch": "milestone",
      "dependsOn": [],
      "goal": "GitLab smoke pipeline: .pin-toolkit.toml (dogfood scope), ci/smoke/{pins.yml,smoke.sh}, root .gitlab-ci.yml (dogfood, smoke, delayed release-image via pipeline input), offline flake check gitlab-ci (~190 lines).",
      "id": "M1",
      "tasks": [
        {
          "check": "mkdir -p .tmp && export CARGO_HOME=\"$(git rev-parse --path-format=absolute --git-common-dir)/../.cargo-home\" PATH=\"/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH\" && cargo build -q --locked -p pin-toolkit && PT=\"$PWD/target/debug/pin-toolkit\" && $PT config show | grep -qxF 'plugins.gitlab-ci.files = [\".gitlab-ci.yml\", \".gitlab/ci/**/*.yml\"]' && $PT --check --json > .tmp/df1-t1.json && jq -e '[.results[].file, .plugins[].errors[].file] | all(. == \".gitlab-ci.yml\" or startswith(\".gitlab/ci/\"))' .tmp/df1-t1.json > /dev/null",
          "create": [
            ".pin-toolkit.toml"
          ],
          "dependsOn": [],
          "files": [
            ".pin-toolkit.toml"
          ],
          "forbidden": [
            "`ignore` patterns (they are per reference, not per file)",
            "a .ignore file (hides ci/ from ripgrep/fd too)",
            "editing or moving ci/.gitlab-ci.yml",
            "keys beyond version and plugins.gitlab-ci.files"
          ],
          "goal": "Create .pin-toolkit.toml exactly as design §2.1 (`version = 1`, `[plugins.gitlab-ci] files = [\".gitlab-ci.yml\", \".gitlab/ci/**/*.yml\"]`, comment naming ci/.gitlab-ci.yml as the oracle input). Planted: delete the file -> `--check` exit 1 (ci/.gitlab-ci.yml rows, ghcr.io/docker/buildx unresolved).",
          "id": "M1.T1"
        },
        {
          "check": "shellcheck -s sh ci/smoke/smoke.sh && export CARGO_HOME=\"$(git rev-parse --path-format=absolute --git-common-dir)/../.cargo-home\" PATH=\"/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH\" && cargo build -q --locked -p pin-toolkit && PT=\"$PWD/target/debug/pin-toolkit\" && rm -rf .tmp/df1-smoke && PATH=\"$PWD/target/debug:$PATH\" CI_COMMIT_BRANCH=main CI_COMMIT_SHA=$(git ls-remote https://gitlab.com/GRBurst/pin-toolkit.git refs/heads/main | cut -f1) sh ci/smoke/smoke.sh .tmp/df1-smoke | tail -1 | grep -qx 'smoke: OK' && git diff --quiet -- ci/smoke/pins.yml && $PT --check --json | jq -e '[.results[].file] | all(startswith(\"ci/\") | not)' > /dev/null",
          "create": [
            "ci/smoke/pins.yml",
            "ci/smoke/smoke.sh"
          ],
          "dependsOn": [
            "M1.T1"
          ],
          "files": [
            "ci/smoke/pins.yml",
            "ci/smoke/smoke.sh"
          ],
          "forbidden": [
            "Docker Hub images in the fixture (anonymous rate limit)",
            "a fixture name matching `*.gitlab-ci.yml` or a path under .gitlab/ci/ (the repo-wide run would see it)",
            "bash-isms, jq or curl in smoke.sh (the image has busybox sh, grep only)",
            "pinning ci/smoke/pins.yml in place"
          ],
          "goal": "Create ci/smoke/pins.yml and ci/smoke/smoke.sh verbatim from design §2.3/§2.4 (copy from the design, keep the v0.1.0 SHA literal and the two summary literals). Planted: (a) `alpine:3.20` -> `alpine:0.0.0-nope` in pins.yml -> `smoke: FAIL: pin-toolkit gitlab-ci … exited 1`; (b) CI_COMMIT_SHA=0000000000000000000000000000000000000000 -> `smoke: FAIL: no line matching /    ref: 000…0  # main/`; (c) unquoted \"$f\" in smoke.sh -> shellcheck SC2086 red.",
          "id": "M1.T2"
        },
        {
          "check": "export CARGO_HOME=\"$(git rev-parse --path-format=absolute --git-common-dir)/../.cargo-home\" PATH=\"/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH\" && cargo build -q --locked -p pin-toolkit && PT=\"$PWD/target/debug/pin-toolkit\" && mkdir -p .tmp/df1 && yq \"select(document_index == 0)\" .gitlab-ci.yml > .tmp/df1/head.yml && yq \"select(document_index == 1)\" .gitlab-ci.yml > .tmp/df1/jobs.yml && yq -e '.spec.inputs.release-tag.default == \"$CI_COMMIT_TAG\"' .tmp/df1/head.yml > /dev/null && yq -e '(.workflow.rules | length) == 2 and ([.workflow.rules[].if | test(\"^\\$CI_PIPELINE_SOURCE == \\\"push\\\" && \")] | all)' .tmp/df1/jobs.yml > /dev/null && yq -e '.release-image.rules[0] | ((.[\"if\"] == \"$CI_COMMIT_TAG =~ /^v[0-9]/\") and (.when == \"delayed\") and (.start_in == \"45 minutes\"))' .tmp/df1/jobs.yml > /dev/null && [ \"$(grep -cE '^  image: ghcr\\.io/grburst/pin-toolkit:[0-9][^@ ]*@sha256:[0-9a-f]{64}$' .gitlab-ci.yml)\" = 2 ] && for j in dogfood smoke release-image; do yq \".\\\"$j\\\".script | join(\\\"\\n\\\")\" .tmp/df1/jobs.yml > .tmp/df1/$j.sh && shellcheck -s sh .tmp/df1/$j.sh || exit 1; done && $PT --check --json > .tmp/df1-t3.json && jq -e '.summary == {\"pinned\":0,\"ok\":2,\"updated\":0,\"skipped\":1,\"unresolved\":0} and ([.results[].file] | unique) == [\".gitlab-ci.yml\"]' .tmp/df1-t3.json > /dev/null",
          "create": [
            ".gitlab-ci.yml"
          ],
          "dependsOn": [
            "M1.T2"
          ],
          "files": [
            ".gitlab-ci.yml"
          ],
          "forbidden": [
            "hand-copied digests",
            "MR, schedule, web or other-branch pipelines (decision 1)",
            "`when`/`start_in` at job level next to rules (GitLab rejects it)",
            "polling loops or sleep for the release race",
            "CI/CD variables named GITLAB_TOKEN/GITHUB_TOKEN/PIN_TOOLKIT_*",
            "Docker Hub images",
            "an `ignore` entry for the release-image line (it is skipped, exit 0)"
          ],
          "goal": "Create .gitlab-ci.yml from design §2.2: write both branch-job images as `ghcr.io/grburst/pin-toolkit:0.1.0`, then run `$PT .gitlab-ci.yml` to pin them (dogfood; expect `pinned: 2, … skipped: 1`); keep the spec header (input release-tag, default $CI_COMMIT_TAG), workflow rules (push && default branch; push && tag =~ /^v[0-9]/), auto_cancel interruptible, default interruptible + timeout 5m, GIT_DEPTH 1, jobs dogfood/smoke (rules if $CI_COMMIT_BRANCH) and release-image (image ghcr.io/grburst/pin-toolkit:$[[ inputs.release-tag | expand_vars | truncate(1,64) ]], rule if tag, when delayed, start_in 45 minutes, interruptible false), artifacts when always / 30 days. Planted: (a) strip one @sha256 digest -> `--check` exit 1; (b) `&&` -> `||` in a workflow rule -> yq false.",
          "id": "M1.T3"
        },
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- .gitlab-ci.yml .pin-toolkit.toml ci/smoke nix/checks.nix && nix build --no-link -L .#checks.x86_64-linux.gitlab-ci && bash scripts/nix-check.sh && git diff --cached --quiet",
          "create": [],
          "dependsOn": [
            "M1.T3"
          ],
          "files": [
            "nix/checks.nix"
          ],
          "forbidden": [
            "network access in the check (pin-toolkit --check, smoke.sh stay out of Nix)",
            "including more than the three source paths in the fileset",
            "changing existing checks"
          ],
          "goal": "nix/checks.nix: add `gitlab-ci` (runCommand; nativeBuildInputs shellcheck yq-go; src = lib.fileset.toSource { root = ../.; fileset = lib.fileset.unions [ ../.gitlab-ci.yml ../.pin-toolkit.toml ../ci/smoke ]; }) running, offline, the T3 yq assertions (split documents into $TMPDIR first), the grep count of 2 digest-pinned images, shellcheck -s sh of every `script:` block (`yq '.[] | select(tag == \"!!map\" and has(\"script\")) | key'`) and of ci/smoke/smoke.sh, and `HOME=$TMPDIR ${pkg}/bin/pin-toolkit --config .pin-toolkit.toml config show | grep -qxF 'plugins.gitlab-ci.files = [...]'`. Planted: `start_in: 1 hour` -> red; an unpinned branch image -> red; `cat $f` in smoke.sh -> red.",
          "id": "M1.T4"
        }
      ]
    },
    {
      "acceptance": [
        "README CI image section links .gitlab-ci.yml as this repository's working example, says the image is public, and names ci/smoke/smoke.sh; readme anchor check green",
        "spec: D60 (dogfood scope via repo files globs) and D61 (tag job: pipeline input + expand_vars/truncate, delayed 45 min; fallback tarball in pinned alpine) rows; S8.4 'As built (DF1)' bullet; DF1 row says part 1 done and what remains; owner steps from design §7"
      ],
      "branch": "milestone",
      "dependsOn": [
        "M1"
      ],
      "goal": "Docs and close-out: README CI image section points at the real .gitlab-ci.yml (GHCR public now), spec DF1 row + S8.4 as built (DF1) + D60/D61 + owner steps (~60 lines).",
      "id": "M2",
      "tasks": [
        {
          "check": "bash scripts/check-readme-anchors.sh && ! grep -q 'The image is public once' README.md && grep -q '(\\.gitlab-ci\\.yml)' README.md && grep -q 'ci/smoke/smoke.sh' README.md && ! grep -q -- '--format json' README.md",
          "create": [],
          "dependsOn": [],
          "files": [
            "README.md"
          ],
          "forbidden": [
            "rewriting unrelated README sections",
            "`--format json` (the flag is --json)",
            "claiming the job token was needed (public projects resolve anonymously too)"
          ],
          "goal": "README.md `## CI image`: replace `The image is public once the owner has set the GHCR package to public.` with: the image is public; this repository's own [.gitlab-ci.yml](.gitlab-ci.yml) is a working example (dogfood --check with a JSON report, ci/smoke/smoke.sh end-to-end pin + idempotence, delayed release-image on v* tags whose image cannot carry a digest yet); `.pin-toolkit.toml` keeps ci/.gitlab-ci.yml (oracle sample) out of the repo-wide run. Tests section: one line naming ci/smoke/smoke.sh and how to run it locally.",
          "id": "M2.T1"
        },
        {
          "check": "S=docs/superpowers/specs/2026-11-09-pin-toolkit-design.md && grep -q '^| D60 ' $S && grep -q '^| D61 ' $S && grep -q 'As built (DF1' $S && grep -E '^\\| DF1 ' $S | grep -q 'part 1' && bash scripts/check-readme-anchors.sh",
          "create": [],
          "dependsOn": [
            "M2.T1"
          ],
          "files": [
            "docs/superpowers/specs/2026-11-09-pin-toolkit-design.md"
          ],
          "forbidden": [
            "rewriting unrelated spec sections",
            "renumbering existing decisions",
            "marking DF1 as done (it stays open, owner-driven)"
          ],
          "goal": "Spec docs/superpowers/specs/2026-11-09-pin-toolkit-design.md: S8.4 bullet 'As built (DF1 part 1)' (files, jobs, exception for release-image, what the smoke proves/does not prove: design §4); D60 and D61 rows (design §1 fact 2, §3); DF1 row: 'part 1 (GitLab smoke pipelines) done in <branch>; owner-driven dogfooding continues'; S13/O-steps: owner steps design §7 (CI/CD + instance runners enabled, config path default, project stays public, no token variables, optional CI Lint dry run, fallback B if GitLab rejects the input); S14: the G1 gap observed in DF1 (`include: - component:` not pinned).",
          "id": "M2.T2"
        }
      ]
    }
  ]
}
```

</details>
