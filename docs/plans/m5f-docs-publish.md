# Plan: m5f-docs-publish
<!-- task-router plan v1. Edit only checkboxes; change the plan with amend_plan. -->
Request: M5f of docs/superpowers/specs/2026-11-09-pin-toolkit-design.md (M5f row, O3, O4): docs, license, GitHub publish. Three stacked branches: T1 LICENSE file (MIT OR Apache-2.0 dual text) + `license` key in the 4 workspace manifests (root [workspace.package] + 3 crates, via workspace = true). T2 README restructure into toolkit overview + per-plugin sections: new `## Plugins` overview section (gitlab-ci built; helm, github-actions known, not built), new `## Plugin: gitlab-ci` parent with the already-contiguous sections (Tag updates, Release age guard, Components & includes, Multi-registry support incl. their subsections) demoted under it, new `## Plugin: helm` and `## Plugin: github-actions` placeholder sections, new `## License` section; no heading renames (anchors are heading-text-derived, demotion keeps links). T3 old spec 2026-11-01-pin-ci-design.md gets a superseded-by banner pointing to the toolkit spec; toolkit spec M5f row marked done and O3 handle filled in (github.com/GRBurst/pin-toolkit, owner-confirmed 2026-11-11). Human follow-up (not a branch): owner creates the GitHub repo, then push main with full history (sandbox reaches github.com; try HTTPS push, fall back to owner push).
Invariants:
  - No heading text renames in README.md; only demotions and new sections (anchors are text-derived)
  - Cargo.lock unchanged; all cargo calls --locked; bash scripts/gate.sh passes on each branch
  - No code changes under crates/*/src or tests
  - Three stacked branches M1 → M2 → M3, each merges ≤ 150 lines
  - LICENSE keeps within budget: full MIT text + Apache-2.0 notice with URL, not the 200-line Apache text
  - GitHub repo creation and push of main with full history is a human follow-up after merge (try HTTPS push, else owner pushes); no task pushes

## M1: License: LICENSE file with MIT OR Apache-2.0 dual-license text and `license = "MIT OR Apache-2.0"` declared in [workspace.package], inherited by all 3 crates via `license.workspace = true`. (branch: milestone; depends: —)
Acceptance: LICENSE exists, states the project is dual-licensed MIT OR Apache-2.0, contains the full MIT text and the Apache-2.0 notice (Licensed under the Apache License, Version 2.0 + URL); root Cargo.toml [workspace.package] has license = "MIT OR Apache-2.0"; crates/pin-core, pin-gitlab-ci, pin-toolkit Cargo.toml have license.workspace = true; cargo metadata reports the license for all 3 packages; Cargo.lock unchanged; bash scripts/gate.sh passes on the branch
- [ ] M1.T1: Create LICENSE (dual-license header line 'MIT OR Apache-2.0', full MIT License text with copyright holder GRBurst 2026, Apache-2.0 notice block with URL; ≤ 55 lines) and add `license = "MIT OR Apache-2.0"` under [workspace.package] in root Cargo.toml.
  files: Cargo.toml, LICENSE (new) · forbidden: changing Cargo.lock; changing [workspace.dependencies]; pasting the full 200-line Apache text (line budget) · check: `test -f LICENSE && grep -q 'MIT License' LICENSE && grep -q 'Apache License, Version 2.0' LICENSE && grep -q 'MIT OR Apache-2.0' LICENSE && grep -qx 'license = "MIT OR Apache-2.0"' Cargo.toml` · depends: —
- [ ] M1.T2: Add `license.workspace = true` (after edition.workspace) to crates/pin-core/Cargo.toml and crates/pin-gitlab-ci/Cargo.toml.
  files: crates/pin-core/Cargo.toml, crates/pin-gitlab-ci/Cargo.toml · forbidden: changing dependencies or features; changing Cargo.lock · check: `grep -qx 'license.workspace = true' crates/pin-core/Cargo.toml && grep -qx 'license.workspace = true' crates/pin-gitlab-ci/Cargo.toml` · depends: M1.T1
- [ ] M1.T3: Add `license.workspace = true` (after edition.workspace) to crates/pin-toolkit/Cargo.toml; cargo metadata must show the license on all 3 packages.
  files: crates/pin-toolkit/Cargo.toml · forbidden: changing dependencies, features or [[bin]]; changing Cargo.lock · check: `grep -qx 'license.workspace = true' crates/pin-toolkit/Cargo.toml && [ "$(cargo metadata --locked --no-deps --format-version 1 | grep -o '"license":"MIT OR Apache-2.0"' | wc -l)" -eq 3 ]` · depends: M1.T2

## M2: README restructure into toolkit overview + per-plugin sections, with only heading demotions (no renames) so every internal anchor link still resolves; plus a License section. (branch: milestone; depends: M1)
Acceptance: ## Plugins overview (after What it does) lists gitlab-ci (built) and helm + github-actions (known, not built); ## Plugin: gitlab-ci parent directly above the demoted Tag updates / Release age guard / Components & includes / Multi-registry support sections (## → ###, their ### → ####); ## Plugin: helm and ## Plugin: github-actions placeholder sections after the gitlab-ci plugin section; ## License section stating MIT OR Apache-2.0 and pointing to LICENSE; scripts/check-readme-anchors.sh passes: every [..](#anchor) in README.md matches a heading slug; bash scripts/gate.sh passes on the branch
- [ ] M2.T1: Create scripts/check-readme-anchors.sh (bash ≤20 lines; GitHub slugs of README headings outside ``` fences; every ](#x) link must match; print misses, exit 1). README: add ## Plugins overview after What it does, ## Plugin: gitlab-ci parent, demote the 4 sections + subsections one level.
  files: README.md, scripts/check-readme-anchors.sh (new) · forbidden: renaming any heading text; demoting Usage, Configuration, Tests or Old pipeline; changing scripts/gate.sh · check: `test -f scripts/check-readme-anchors.sh && grep -qx '## Plugin: gitlab-ci' README.md && grep -qx '### Multi-registry support' README.md && grep -qx '#### Per-host logins' README.md && sed -n '/^## Plugins$/,/^## /p' README.md | grep -q github-actions && bash scripts/check-readme-anchors.sh` · depends: —
- [ ] M2.T2: In README.md add ## Plugin: helm and ## Plugin: github-actions placeholder sections (not built yet; `pin-toolkit plugins` shows state) right after the gitlab-ci plugin section, and a final ## License section (MIT OR Apache-2.0, see LICENSE).
  files: README.md · forbidden: renaming or re-leveling existing headings; documenting unbuilt behaviour as available · check: `grep -qx '## Plugin: helm' README.md && grep -qx '## Plugin: github-actions' README.md && sed -n '/^## License$/,$p' README.md | grep -q 'MIT OR Apache-2.0' && bash scripts/check-readme-anchors.sh` · depends: M2.T1

## M3: Spec bookkeeping: old pin-ci spec marked superseded by the toolkit spec; toolkit spec M5f marked done and O3 GitHub handle filled in (GRBurst, owner-confirmed 2026-11-11). (branch: milestone; depends: M2)
Acceptance: 2026-11-01-pin-ci-design.md has a superseded-by banner near the top linking 2026-11-09-pin-toolkit-design.md; toolkit spec §9 M5f row and §10 M5f row marked done; O3 names github.com/GRBurst/pin-toolkit (owner-confirmed 2026-11-11); 'to be filled in at M5f' gone; bash scripts/gate.sh passes on the branch
- [ ] M3.T1: Add a superseded-by banner (blockquote, ≤ 4 lines) right below the title of docs/superpowers/specs/2026-11-01-pin-ci-design.md, linking ./2026-11-09-pin-toolkit-design.md.
  files: docs/superpowers/specs/2026-11-01-pin-ci-design.md · forbidden: editing the old spec body · check: `head -8 docs/superpowers/specs/2026-11-01-pin-ci-design.md | grep -i 'superseded' | grep -q '2026-11-09-pin-toolkit-design.md'` · depends: —
- [ ] M3.T2: Toolkit spec: §9 M5f row starts '**Done** 2026-11-11 (...)' like M5d; §10 M5f row starts 'done (2026-11-11): '; O3: replace '<owner>'/'to be filled in at M5f' with github.com/GRBurst/pin-toolkit (owner-confirmed 2026-11-11); M5f ✓ in D34 order line.
  files: docs/superpowers/specs/2026-11-09-pin-toolkit-design.md · forbidden: changing other milestone rows or decisions; claiming the GitHub push already happened · check: `f=docs/superpowers/specs/2026-11-09-pin-toolkit-design.md; grep -qE '^\| M5f \| \*\*Done\*\*' $f && grep -qE '^\| M5f \| done' $f && grep -q 'github.com/GRBurst/pin-toolkit' $f && ! grep -q 'to be filled in at M5f' $f` · depends: —

<details><summary>skeleton</summary>

```json
{
  "invariants": [
    "No heading text renames in README.md; only demotions and new sections (anchors are text-derived)",
    "Cargo.lock unchanged; all cargo calls --locked; bash scripts/gate.sh passes on each branch",
    "No code changes under crates/*/src or tests",
    "Three stacked branches M1 → M2 → M3, each merges ≤ 150 lines",
    "LICENSE keeps within budget: full MIT text + Apache-2.0 notice with URL, not the 200-line Apache text",
    "GitHub repo creation and push of main with full history is a human follow-up after merge (try HTTPS push, else owner pushes); no task pushes"
  ],
  "milestones": [
    {
      "acceptance": [
        "LICENSE exists, states the project is dual-licensed MIT OR Apache-2.0, contains the full MIT text and the Apache-2.0 notice (Licensed under the Apache License, Version 2.0 + URL)",
        "root Cargo.toml [workspace.package] has license = \"MIT OR Apache-2.0\"",
        "crates/pin-core, pin-gitlab-ci, pin-toolkit Cargo.toml have license.workspace = true; cargo metadata reports the license for all 3 packages",
        "Cargo.lock unchanged; bash scripts/gate.sh passes on the branch"
      ],
      "branch": "milestone",
      "dependsOn": [],
      "goal": "License: LICENSE file with MIT OR Apache-2.0 dual-license text and `license = \"MIT OR Apache-2.0\"` declared in [workspace.package], inherited by all 3 crates via `license.workspace = true`.",
      "id": "M1",
      "tasks": [
        {
          "check": "test -f LICENSE && grep -q 'MIT License' LICENSE && grep -q 'Apache License, Version 2.0' LICENSE && grep -q 'MIT OR Apache-2.0' LICENSE && grep -qx 'license = \"MIT OR Apache-2.0\"' Cargo.toml",
          "create": [
            "LICENSE"
          ],
          "dependsOn": [],
          "files": [
            "Cargo.toml"
          ],
          "forbidden": [
            "changing Cargo.lock",
            "changing [workspace.dependencies]",
            "pasting the full 200-line Apache text (line budget)"
          ],
          "goal": "Create LICENSE (dual-license header line 'MIT OR Apache-2.0', full MIT License text with copyright holder GRBurst 2026, Apache-2.0 notice block with URL; ≤ 55 lines) and add `license = \"MIT OR Apache-2.0\"` under [workspace.package] in root Cargo.toml.",
          "id": "M1.T1"
        },
        {
          "check": "grep -qx 'license.workspace = true' crates/pin-core/Cargo.toml && grep -qx 'license.workspace = true' crates/pin-gitlab-ci/Cargo.toml",
          "dependsOn": [
            "M1.T1"
          ],
          "files": [
            "crates/pin-core/Cargo.toml",
            "crates/pin-gitlab-ci/Cargo.toml"
          ],
          "forbidden": [
            "changing dependencies or features",
            "changing Cargo.lock"
          ],
          "goal": "Add `license.workspace = true` (after edition.workspace) to crates/pin-core/Cargo.toml and crates/pin-gitlab-ci/Cargo.toml.",
          "id": "M1.T2"
        },
        {
          "check": "grep -qx 'license.workspace = true' crates/pin-toolkit/Cargo.toml && [ \"$(cargo metadata --locked --no-deps --format-version 1 | grep -o '\"license\":\"MIT OR Apache-2.0\"' | wc -l)\" -eq 3 ]",
          "dependsOn": [
            "M1.T2"
          ],
          "files": [
            "crates/pin-toolkit/Cargo.toml"
          ],
          "forbidden": [
            "changing dependencies, features or [[bin]]",
            "changing Cargo.lock"
          ],
          "goal": "Add `license.workspace = true` (after edition.workspace) to crates/pin-toolkit/Cargo.toml; cargo metadata must show the license on all 3 packages.",
          "id": "M1.T3"
        }
      ]
    },
    {
      "acceptance": [
        "## Plugins overview (after What it does) lists gitlab-ci (built) and helm + github-actions (known, not built)",
        "## Plugin: gitlab-ci parent directly above the demoted Tag updates / Release age guard / Components & includes / Multi-registry support sections (## → ###, their ### → ####)",
        "## Plugin: helm and ## Plugin: github-actions placeholder sections after the gitlab-ci plugin section",
        "## License section stating MIT OR Apache-2.0 and pointing to LICENSE",
        "scripts/check-readme-anchors.sh passes: every [..](#anchor) in README.md matches a heading slug",
        "bash scripts/gate.sh passes on the branch"
      ],
      "branch": "milestone",
      "dependsOn": [
        "M1"
      ],
      "goal": "README restructure into toolkit overview + per-plugin sections, with only heading demotions (no renames) so every internal anchor link still resolves; plus a License section.",
      "id": "M2",
      "tasks": [
        {
          "check": "test -f scripts/check-readme-anchors.sh && grep -qx '## Plugin: gitlab-ci' README.md && grep -qx '### Multi-registry support' README.md && grep -qx '#### Per-host logins' README.md && sed -n '/^## Plugins$/,/^## /p' README.md | grep -q github-actions && bash scripts/check-readme-anchors.sh",
          "checkCounterexample": "renaming 'Tag updates (`--update`)' breaks #tag-updates---update and the anchor script fails",
          "create": [
            "scripts/check-readme-anchors.sh"
          ],
          "dependsOn": [],
          "files": [
            "README.md"
          ],
          "forbidden": [
            "renaming any heading text",
            "demoting Usage, Configuration, Tests or Old pipeline",
            "changing scripts/gate.sh"
          ],
          "goal": "Create scripts/check-readme-anchors.sh (bash ≤20 lines; GitHub slugs of README headings outside ``` fences; every ](#x) link must match; print misses, exit 1). README: add ## Plugins overview after What it does, ## Plugin: gitlab-ci parent, demote the 4 sections + subsections one level.",
          "id": "M2.T1"
        },
        {
          "check": "grep -qx '## Plugin: helm' README.md && grep -qx '## Plugin: github-actions' README.md && sed -n '/^## License$/,$p' README.md | grep -q 'MIT OR Apache-2.0' && bash scripts/check-readme-anchors.sh",
          "dependsOn": [
            "M2.T1"
          ],
          "files": [
            "README.md"
          ],
          "forbidden": [
            "renaming or re-leveling existing headings",
            "documenting unbuilt behaviour as available"
          ],
          "goal": "In README.md add ## Plugin: helm and ## Plugin: github-actions placeholder sections (not built yet; `pin-toolkit plugins` shows state) right after the gitlab-ci plugin section, and a final ## License section (MIT OR Apache-2.0, see LICENSE).",
          "id": "M2.T2"
        }
      ]
    },
    {
      "acceptance": [
        "2026-11-01-pin-ci-design.md has a superseded-by banner near the top linking 2026-11-09-pin-toolkit-design.md",
        "toolkit spec §9 M5f row and §10 M5f row marked done",
        "O3 names github.com/GRBurst/pin-toolkit (owner-confirmed 2026-11-11); 'to be filled in at M5f' gone",
        "bash scripts/gate.sh passes on the branch"
      ],
      "branch": "milestone",
      "dependsOn": [
        "M2"
      ],
      "goal": "Spec bookkeeping: old pin-ci spec marked superseded by the toolkit spec; toolkit spec M5f marked done and O3 GitHub handle filled in (GRBurst, owner-confirmed 2026-11-11).",
      "id": "M3",
      "tasks": [
        {
          "check": "head -8 docs/superpowers/specs/2026-11-01-pin-ci-design.md | grep -i 'superseded' | grep -q '2026-11-09-pin-toolkit-design.md'",
          "dependsOn": [],
          "files": [
            "docs/superpowers/specs/2026-11-01-pin-ci-design.md"
          ],
          "forbidden": [
            "editing the old spec body"
          ],
          "goal": "Add a superseded-by banner (blockquote, ≤ 4 lines) right below the title of docs/superpowers/specs/2026-11-01-pin-ci-design.md, linking ./2026-11-09-pin-toolkit-design.md.",
          "id": "M3.T1"
        },
        {
          "check": "f=docs/superpowers/specs/2026-11-09-pin-toolkit-design.md; grep -qE '^\\| M5f \\| \\*\\*Done\\*\\*' $f && grep -qE '^\\| M5f \\| done' $f && grep -q 'github.com/GRBurst/pin-toolkit' $f && ! grep -q 'to be filled in at M5f' $f",
          "dependsOn": [],
          "files": [
            "docs/superpowers/specs/2026-11-09-pin-toolkit-design.md"
          ],
          "forbidden": [
            "changing other milestone rows or decisions",
            "claiming the GitHub push already happened"
          ],
          "goal": "Toolkit spec: §9 M5f row starts '**Done** 2026-11-11 (...)' like M5d; §10 M5f row starts 'done (2026-11-11): '; O3: replace '<owner>'/'to be filled in at M5f' with github.com/GRBurst/pin-toolkit (owner-confirmed 2026-11-11); M5f ✓ in D34 order line.",
          "id": "M3.T2"
        }
      ]
    }
  ]
}
```
</details>
