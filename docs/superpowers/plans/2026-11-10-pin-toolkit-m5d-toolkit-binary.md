# M5d plan — minimal `pin-toolkit` binary, replaces `pin-ci` (pin-toolkit)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

- Date: 2026-11-10
- Source of truth: `docs/superpowers/specs/2026-11-09-pin-toolkit-design.md`: §6.2–§6.7 (core interface, one-pass run, report, exit codes, front-ends incl. the D33 `pin-ci` rule, memo), §6.8 (`release_age.unknown`), §6.9 (parse error → exit 2), §7.1–§7.3 (D35 layers, global-only keys, D37 token binding), §8.1/§8.2 (O1 "not in this build"), §9 (M5d row), §10, §11, §12 (D9–D19, D29–D38), §13 O1, §14.1 (carry-overs M3/D38, L2–L6, L8–L10, N5).
- Inputs: code review `.superpowers/sdd/2026-11-10-m5abc/code-review.md` (L2–L6, L8–L10, N5, M3); plans `2026-11-10-pin-toolkit-m5abc-workspace-core.md`, `2026-11-10-pin-toolkit-m5cy-yaml-discovery.md` (style, harness).
- Decided by main (2026-11-10, handshake): switch branches in place; every §6.6 option except `config show` (M5e'), each option = one config field, precedence in one place; keep `pin-ci` as **one argv[0] dispatch entry**, symlink in M6, README says so; explicit unclaimed file → exit 2 naming the file; known plugin ids from a static list in pin-toolkit, anything else exit 2; implement `ignore` (no row, no JSON `ignored` count: the spec has none); implement `release_age.unknown = "block"`; env vars `PIN_TOOLKIT_*` without compat (test gates `PIN_CI_LIVE`/`PIN_CI_ORACLE` keep their names); every snapshot diff named in its commit message; oracle/live switched off the deleted pin-ci API (reading in Open for main 11); `pin-toolkit --no-default-features` joins the matrix.
- Base: `main` @ c6a6ef3 (M5c-y merged). Branch: `m5d-toolkit-binary`.
- Out of scope: global config, `[hosts]` bindings, `config show [--origin]` (M5e'); nix symlinks `pin-<plugin>`/`pin-ci` (M6); toolkit README with per-plugin sections, LICENSE (M5f); helm/github-actions plugins (M7–M9); per-finding splice validation (§14.1 M1, before M8); `helm-oci` kind behaviour (accepted as a known kind, no targets until M7); the §7.3 `unresolved (<host>: not trusted for credentials — bind it in …)` message (M5e', T-D30, main).
- Tech stack: Rust 1.95, edition 2024, clap 4 (derive), tokio 1 (current-thread runtime in the binary), toml 0.9, ignore 0.4, globset 0.4, insta 1.49, wiremock 0.6.

## Goal

A new crate `crates/pin-toolkit` builds the multi-call binary `pin-toolkit` (D9). One run loads the repo config + env + CLI
(defaults < repo `.pin-toolkit.toml` < env < CLI, D35), walks the git root (or explicit paths), routes each file to exactly one
enabled plugin, plans and writes per file, keeps going after per-file errors, prints one combined report (human or JSON v1) and
exits 0/1/2. `pin-toolkit gitlab-ci …`, `pin-gitlab-ci` and the alias `pin-ci` run only gitlab-ci. The crate's `[[bin]] pin-ci`
is deleted. `CI_JOB_TOKEN` reaches only `CI_SERVER_HOST` (D37). The §14.1 carry-overs land on the way.

## Motivation (spec §6.3–§6.7, §7, §9 M5d, D33, D35, D37)

`pin-ci` is one plugin's CLI: it stops at the first bad file, exits 1 on a YAML error, sends `CI_JOB_TOKEN` to the default
gitlab.com, and has no config file. The toolkit is the product front-end every later plugin plugs into (M7–M9), so the run
loop, routing, report schema (stable from the first release, D15) and exit codes must be right before a second plugin exists.
The review carry-overs (raw clients public, stringly status equality, one `LineIndex` per comment, two GitLab host sources)
touch the same seams and are cheapest now.

## Approach (shape of the diff)

| Task | Diff shape |
|---|---|
| T1–T5 | pin-core/pin-gitlab-ci carry-overs, no behaviour change: L6 str-eq removed, L4/L5 `RegError: Clone` + `try_new`, L2 non-pinned edits dropped, L3 `Source` per file, L10 kind consts + N5 |
| T6 | `Ctx` memo (§6.7): `Memo<K,V>` over the four entry points, errors cached, in-flight lookups coalesced |
| T7–T8 | `GitlabEnv` + `GitlabAuth` (D31/D37), multi-host `GitLabHosts`, `GitLabClient::from_env` deleted, pin-core feature `test-support` (base-URL overrides); L9 `GitlabCi` unit struct + `Ctx::gitlab_enabled` |
| T9 | `AgePolicy { Allow, Block }` in `UpdatePolicy` (§6.8) |
| T10–T11 | pin-core report v1 model + `Report::exit_code`, `Selection` (kinds + `ignore` patterns) |
| T12 | pin-core `run::{Route, run}`: one pass over routes, per-file errors recorded, later files continue (§6.3); **review checkpoint 1** after T12 (core done) |
| T13–T19 | new crate `crates/pin-toolkit`: scaffold + `plugins`, `Layer`/`Effective::fold`, repo TOML, env, CLI + argv[0], config loading, enablement/"not in this build" |
| T20–T23 | walk + routing, run wiring (38 human/JSON snapshots `git mv`'d, byte-identical), exit-code table, credential wiremock (gate items) |
| T24–T25 | delete `[[bin]] pin-ci` (driver → `tests/common/driver.rs`), alias tests; D38 raw clients behind `test-support`; **review checkpoint 2** after T25 (toolkit before visible output changes) |
| T26 | snapshot metadata only: insta `source:`/`expression:` headers of the 38 moved snapshots rewritten, no content change (PR-M7) |
| T27–T29 | user-visible output changes, each with named snapshot diffs: human `plugin:kind` (19 files), JSON v1 (19 files), `service` kind (subset) |
| T30–T31 | toolkit live smoke (`PIN_CI_LIVE=1`); README + spec close-out |

Estimate: **31 commits** (T1–T31), ≈ +2 800 / −700 lines incl. tests. That is above §10's "M" (the carry-overs add ≈ 9 commits): see Open for main 12 (decided: one branch, two review checkpoints).

Finding labels: `PR-H1`, `PR-M1`…`PR-M10`, `PR-L1`…`PR-L14` are the plan-review findings (`.superpowers/sdd/2026-11-10-m5d/plan-review.md`, see "Review fixes applied" at the end). Unprefixed `L2`–`L10`, `M3`, `N5` remain the M5a–c code-review carry-overs (§14.1).

## Global constraints

1. **No behaviour change except** T7 (D37 token binding, D31 primary via `CI_SERVER_HOST`), T9 (new `block` value; `allow` = today), T12 (new `run`), T21+ (new binary), T24 (`pin-ci` binary gone), T27/T28/T29 (named snapshot diffs; T26 rewrites snapshot headers only). Proof per task: `INSTA_UPDATE=no` gate green and `git status` shows no `*.snap` change outside the task's list.
2. **Existing tests are preserved.** Allowed edits, each repeated in its task (sites verified by grep on 8f053e2, PR-M1):
   - T1: 19 status str-eq sites → `Status::…` (`tests/common/mod.rs` 4, `e2e.rs` 13, `e2e_components.rs` 1, `live.rs` 1), report unit test renamed.
   - T4: `trailing_comment` unit tests gain an index argument; test `Fake::resolve` signatures; 4 direct `resolve` calls in `tests/plugin.rs`.
   - T7: the env-mutating `from_env` unit test `env_precedence` in `gitlab.rs` (:392–440) is replaced by a pure table test (its subject is deleted); `ResolverEnv { gitlab_host: … }` literals → `ResolverEnv::default()` at `crates/pin-gitlab-ci/tests/common/mod.rs:434-437`, `crates/pin-gitlab-ci/tests/plugin.rs:16-21` (`fn env()`), `crates/pin-core/src/run.rs:420-425` (test-only `fn env()` in `resolver_tests`); `GitLabClient::from_env(GITLAB_API…)` at `crates/pin-core/tests/live_gitlab.rs:101,122,147` → `GitLabClient::try_new(GITLAB_API.into(), token, job)` with `(token, job) = ResolverEnv::from_process("gitlab.com").gitlab.auth_for("gitlab.com").into_parts()` (compiled under `--all-features`, so the gate fails to build without it). The `gitlab_host: "gitlab.com".into()` fields in `e2e.rs`, `e2e_components.rs`, `live.rs`, `oracle.rs` are `PinOptions` (driver) fields and stay.
   - T8: `tests/plugin.rs:67` (`gitlab_hosts` field gone).
   - T9: **one** `UpdatePolicy` literal gains `unknown_age` (`crates/pin-gitlab-ci/src/driver.rs:101`); `oci_decide.rs:58` and `common/mod.rs:430` use `..Default::default()` and compile unchanged.
   - T10: `Row { … }` literals gain `follow_up: None`: `crates/pin-core/src/report.rs:148,159,170,215` (unit tests), `crates/pin-gitlab-ci/src/driver.rs:178` (unit test `row()`), `crates/pin-gitlab-ci/tests/fixture_properties.rs:18`.
   - T11: test helpers pass a `Selection`: `run_pin_file.rs` `on()`, `tests/plugin.rs:171,282`, `common/mod.rs:442`, and the `pin_file` call in `crates/pin-core/src/run.rs:437` (`resolver_tests`).
   - T21: pin-gitlab-ci `fixture_snapshots.rs` drops its human/JSON asserts (the 38 snapshots move, R100, and are asserted by pin-toolkit), `common::fixtures()` path.
   - T24: imports of the moved driver; 3 `main.rs` unit tests deleted (superseded by T17); the driver unit test `outcome_prints_partial_rows_before_the_file_error` moves from `driver.rs` into `tests/e2e.rs` (PR-L2).
   - T26: the 38 moved snapshots' `source:`/`expression:` header lines (no content).
   - T27: `human_render_exact`; `crates/pin-gitlab-ci/tests/e2e.rs:308-313` (`"{}:2  skipped (unparseable reference)"` → `"{}:2  gitlab-ci:image  skipped (unparseable reference)"`); the moved `outcome_prints_partial_rows_before_the_file_error` in `e2e.rs` (`"a.yml:2  skipped (r)"` ×2 → `"a.yml:2  gitlab-ci:image  skipped (r)"`); pin-toolkit (review-2 M3, line numbers at 11f321d): `crates/pin-toolkit/tests/run.rs:34` (`".gitlab-ci.yml:1  pinned  …"` → `".gitlab-ci.yml:1  gitlab-ci:image  pinned  …"`), `crates/pin-toolkit/tests/exit_codes.rs:131` (row 4 `starts_with(".gitlab-ci.yml:1  unresolved (")` → `".gitlab-ci.yml:1  gitlab-ci:image  unresolved ("`), `exit_codes.rs:232` (row 14 exact stdout `".gitlab-ci.yml:1  skipped (unparseable reference)  …"` → `".gitlab-ci.yml:1  gitlab-ci:image  skipped (unparseable reference)  …"`).
   - T28: `json_render_roundtrip`; `crates/pin-toolkit/tests/exit_codes.rs` P-exit recomputation reads `plugins.*.errors` + new row 6j (PR-L8).
   - T29: kind lists in `tests/plugin.rs`, `discovery_parity.rs` `kind()`.
3. **Features (D9, D36, D38):** pin-toolkit `default = ["gitlab-ci"]`, `gitlab-ci = ["dep:pin-gitlab-ci"]`; pin-core gains `test-support` (T7, widened in T25) — never enabled by a non-dev dependency, checked by the `cargo tree` gate line from T25 (PR-M4). Integration tests of pin-toolkit that need gitlab-ci start with `#![cfg(feature = "gitlab-ci")]`.
4. **Texts users read are pinned literally** (list in Snippets → "User texts"); pin-ci texts that survive (row format, skip reasons) unchanged except T27.
5. **No process-env reads below the binary edge:** pin-toolkit reads env only in `Process::from_os()`; everything else gets `Process.env`. Registry credentials too (PR-M2, T7): `ResolverEnv::from_vars` captures `DOCKERHUB_USER`, `DOCKERHUB_TOKEN`, `GITHUB_TOKEN` from the map into `ResolverEnv.registry`; `Ctx::oci()` builds `Creds` via `ResolverEnv::creds()` = `Creds::load_from_env(home, |v| registry.get(v))` (made `pub(crate)`); the docker config under `home` is still read (file, unchanged). `Creds::load_from` (process env) stays only for driver/test helpers (`client_from_env`, `registry_hosts.rs`) and goes behind `test-support` with the registry module in T25.
6. **Net new crates** only from `toml 0.9`, `ignore 0.4`, `globset 0.4` and their dependencies; the T13 commit body lists `git diff Cargo.lock | grep '^+name'`. Licences MIT/Apache-2.0/Unlicense only.
7. Env/commits: `export CARGO_HOME="$PWD/.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH"`; unsigned commits `git -c commit.gpgsign=false -c core.excludesFile=/dev/null commit`; every `git add`/`status` with `-c core.excludesFile=/dev/null`; scratch only under `.tmp/` (crate-local `.tmp/` is ignored by the root `.gitignore`). Network to crates.io is needed once (T13) to lock `toml`/`ignore`/`globset`; afterwards `--offline` works.
8. Temp repos in tests live under `$CARGO_MANIFEST_DIR/.tmp/<test>-<pid>/` and **must contain a `.git/` directory**; without it the git-root walk-up finds this repository (and its config).

### Check (run after every task; "the gate")

```bash
export CARGO_HOME="$PWD/.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH"
INSTA_UPDATE=no cargo test --workspace --all-features > .tmp/m5d-test.log 2>&1; echo "cargo exit $?"
grep -E '^test result' .tmp/m5d-test.log | awk '{p+=$4; f+=$6} END {print "passed", p, "failed", f}'
cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings
for f in "" "--features oci" "--features gitlab" "--features yaml" "--features oci,gitlab,test-support"; do   # last leg from T7
  cargo clippy -p pin-core --no-default-features $f --all-targets -- -D warnings || echo "FAIL [$f]"; done
# from T13:
cargo clippy -p pin-toolkit --no-default-features --all-targets -- -D warnings && cargo test -p pin-toolkit --no-default-features 2>&1 | grep -E '^test result'
# from T25 (D38: raw clients unreachable from plugin/binary code; dev-dependency features are not unified here):
cargo check -p pin-gitlab-ci -p pin-toolkit
for p in pin-gitlab-ci pin-toolkit; do   # D38: no normal/build dependency enables pin-core/test-support (PR-M4)
  cargo tree -p $p -e normal,build -f '{p} {f}' | grep -q 'pin-core.*test-support' && echo "FAIL D38 [$p]"; done
git -c core.excludesFile=/dev/null status --porcelain     # only this task's files; never *.snap.new
```

Baseline (c6a6ef3, measured while planning): **passed 239 failed 0**. Expected passed after each task (all-features run):
T1 239 · T2 240 · T3 241 · T4 242 · T5 242 · T6 246 · T7 251 · T8 252 · T9 254 · T10 257 · T11 260 · T12 264 · T13 266 · T14 268 ·
T15 271 · T16 275 · T17 279 · T18 283 · T19 286 · T20 294 · T21 297 · T22 298 · T23 301 · T24 301 · T25 301 · T26 301 · T27 302 ·
T28 304 · T29 306 · T30 307 · T31 307 (**239 → 307**).
As built (review-2 M3): measured **308** at T25 (table + 7: review-1 fixes + 6, T21 extra `walked_conflicts_are_file_errors` + 1); the review-2 fixes (761c591..) add 5 (M1 + 2, L4 + 1, L3 + 2) → **313** before T26. Expected from there: T26 313 · T27 314 · T28 316 · T29 318 · T30 319 · T31 319. `--no-default-features` leg: 34 before T26.
`--no-default-features` pin-toolkit leg (PR-L3): T13 **2** (`compiled_plugins_are_known`, vacuous without plugins; `plugins_without_gitlab_ci`); from T14 on every pin-toolkit unit test and every integration test not gated `#[cfg(feature = "gitlab-ci")]` also runs there, so the rule is **≥ the previous task's count**; each commit body states the leg's count and names the tests it gated on `gitlab-ci`.
A lower number is a failure (silently skipped tests) unless the task explains it (T7: −1 +6; T24: −3 −1 +1 +3 = 0). A higher number is fine if the extra tests are named in the commit body.

## Decisions

| # | Decision | Rationale | Rejected |
|---|---|---|---|
| T-D1 | **New crate `crates/pin-toolkit`** (lib + `[[bin]] pin-toolkit`); lib exposes `execute(Process, tune) -> Output` | D12 names it; the binary depends on every plugin crate behind features (D9) — no plugin crate may own it; the lib makes the whole CLI testable in-process with fakes, the bin stays a 10-line shell | binary in pin-core (core would depend on plugins, breaks D10); binary in pin-gitlab-ci (keeps the coupling M5d removes) |
| T-D2 | **`config` and `cli` live in pin-toolkit**, not pin-core (§6.1 lists `config`, `cli` in pin-core) | the repo schema validates `[plugins.<id>]` for *known but not built* plugins (O1) from the static list in pin-toolkit (main); core must not know plugin ids (D10); no second front-end exists to share them | pin-core `config` with a plugin-registration API (speculative; revisit when a second front-end appears). Open for main 1 |
| T-D3 | `Process { args, env, cwd }` → `Output { stdout, stderr, code }`; env read once at the edge, **including registry credentials** (`DOCKERHUB_USER`/`DOCKERHUB_TOKEN`/`GITHUB_TOKEN` via `ResolverEnv.registry`, PR-M2) | tests pass env maps (no `set_var`, no races, no machine-dependent tokens); one place reads the process | reading `std::env` in config/resolvers (today's `GitLabClient::from_env`; unsafe in parallel tests) |
| T-D4 | **One precedence function** `Effective::fold(&[Layer])`; every source (defaults, repo, env, CLI) produces the same `Layer` (all `Option`) | main: one field per option, precedence in one place; adding the global layer (M5e') = one more element | per-source merging into a mutable struct (precedence spread over sources); figment (dependency, origin tracking not needed until `config show --origin`) |
| T-D5 | Merge rules in `fold`: scalars last-`Some` wins; `files` and `gitlab-hosts` replace; `ignore` concatenates; `kinds` per key last wins | §7.1 verbatim (`gitlab-hosts` is a list of hosts: replace, like `files`) | deep-merging lists |
| T-D6 | Repo file: `toml::Table` pre-pass (`version`, global-only keys `hosts`, `concurrency`) then typed serde with `deny_unknown_fields`; `[plugins.<id>]` validated against the static `KNOWN` list | a global-only key needs its own message (§7.2, gate); typed parse gives "unknown key" errors for free (D17) | typed parse only (generic "unknown field" for `[hosts]`) |
| T-D7 | Env names `PIN_TOOLKIT_{UPDATE, UPDATE_MAJOR, MIN_AGE_DAYS, RELEASE_AGE_UNKNOWN, CONCURRENCY, JSON, ONLY, SKIP, GITLAB_HOST, CONFIG}`; booleans `1/0, true/false, yes/no, on/off` (case-insensitive); lists comma-separated; empty value = unset; a bad value of a known variable → exit 2; any other `PIN_TOOLKIT_*` → one stderr line `pin-toolkit: ignoring unknown environment variable {VAR}` per variable (sorted), exit code unaffected (main, PR-M3) | §7.1 `env PIN_TOOLKIT_*`; GitLab exports every CI/CD variable to the job, so `PIN_TOOLKIT_VERSION` for the image tag is realistic and must not break runs; typos stay visible | exit 2 on unknown vars (one pipeline variable breaks every run); silently ignoring them (typos invisible). Open for main 7 |
| T-D8 | CLI-only: `--dry-run`, `--check` (`check ⇒ dry_run`), `--kind`, `--config`, `--no-config`; CLI flags are presence-only (no `--no-update`) | §6.6 list; env can still say `PIN_TOOLKIT_UPDATE=0` over a repo `true` | `--no-*` twins (surface growth). Open for main 9 |
| T-D9 | **Subcommand per known plugin id** + `plugins`; argv[0] `pin-<id>` (id ∈ KNOWN) and the single alias entry `("pin-ci", "gitlab-ci")` insert the subcommand | §6.6; main: keep `pin-ci` as one dispatch entry (D33 cost rule met: one table row here, one symlink in M6) | `pin-ci` legacy mode (D33 forbids) |
| T-D10 | Explicit file: with a plugin subcommand/alias it is claimed regardless of globs; in all-plugins mode it must match exactly one enabled plugin's globs, else exit 2 naming the file (fatal, before the run). A **walked** file matching 2+ plugins is a **per-file error** (PR-L11): `ErrorEntry { file, line: None, message: <claimed-by text> }` under the first claimant in `compiled()` order, the file is not routed, the other files run, exit 2 | §6.2 rule; main Q4; D11 one file = one plugin; §6.3 per-file errors, others continue | silently picking the first plugin; a fatal error for a walked file (one ambiguous glob would stop every run, e.g. `.gitlab/ci/values.yaml` under gitlab-ci and helm in M8). Open for main 3 |
| T-D11 | "Plugin enabled but not compiled in" (§6.5 exit 2) = an explicit CLI/env request (subcommand, alias, `--only`, or `PIN_TOOLKIT_ONLY` in all-plugins mode only — PR-L6); a repo `[plugins.<p>]` table with `run ≠ false` for a known, unbuilt plugin → one stderr notice `pin-toolkit: skipped (plugin <p> not in this build)`, exit unaffected | reconciles §6.5 with §8.2/O1 ("reported once … exit unaffected") | exit 2 for config tables (a shared repo config would break smaller builds). Open for main 2 |
| T-D12 | GitLab: primary = `--gitlab-host` > `PIN_TOOLKIT_GITLAB_HOST` > `CI_SERVER_HOST` > `gitlab.com`; resolve set = {primary} ∪ `plugins.gitlab-ci.gitlab-hosts`; `GITLAB_TOKEN` → `PRIVATE-TOKEN` to primary only; `CI_JOB_TOKEN` → `JOB-TOKEN` to `CI_SERVER_HOST` only (GITLAB_TOKEN wins when both apply); else anonymous | D31 + D37 verbatim; the toolkit precedence lives in `fold`, the `CI_SERVER_HOST`/gitlab.com fallback and the token binding live in pin-core `GitlabEnv` (resolver policy, next to the clients) | `CI_SERVER_HOST` implicitly added to the resolve set (credentials would follow it; the spec says only bound hosts get them) |
| T-D13 | `GitlabCi` becomes a unit struct; "host enabled" = `ctx.gitlab_enabled(host)` derived from the one `GitlabEnv` | L9: one source of truth | keeping `gitlab_hosts` on the plugin (two sources) |
| T-D14 | D38: raw `RegistryClient`, `Creds`, `GitLabClient`, `GitLabHosts`, `Ctx::with_oci/with_gitlab` are `#[doc(hidden)] pub` only under pin-core feature `test-support` (dev-dependencies enable it); `cargo check -p pin-gitlab-ci -p pin-toolkit` proves plugin/binary code cannot name them, and `cargo tree -e normal,build` proves no normal dependency enables `test-support` (PR-M4) | §14.1 M3 "crate-private or doc-hidden behind a test-support feature"; e2e tests keep their fakes | convention only (unchecked) |
| T-D15 | Memo: `Memo<K, V> = RefCell<HashMap<K, Rc<tokio::sync::OnceCell<V>>>>` per entry point; keys `(host_str, api_repo, effective_tag)` digest, `(host_str, api_repo)` tags and tag times, `(host, project, ref)` GitLab; values incl. errors (`RegError: Clone`) | §6.7/D19; `OnceCell::get_or_init` coalesces concurrent lookups; `Ctx` is single-threaded (`LocalBoxFuture`), so `RefCell` + `Rc` suffice | `Mutex<HashMap>` (Send not needed); a disk cache (spec: none) |
| T-D16 | L3: `Plugin::resolve(&self, ctx, src: &Source<'_>, t)`, `Source { path, text, lines: LineIndex }` built once per file in `DynPlugin::plan`; `trailing_comment(text, lines, line, token_end, orig)` | §14.1 L3 "per-file context in `Plugin::resolve`"; also gives resolvers the path (helm needs it, M8) | line ends stored in every target (touches every target type) |
| T-D17 | `ignore`: `Selection { kinds, ignore: RefPatterns }` replaces `on: &KindSet` in `plan`/`pin_file`; `Plugin::ignore_key(&self, t) -> Option<String>` (default `None`); gitlab-ci key = the value with quotes stripped (`t.value()`) for image/service/component/unsupported (so `alpine:3.20` does not match an already pinned `alpine:3.20@sha256:…`; use `alpine:3.20*`, README), `{project}@{ref}` for include; wildcard `*` (any run, incl. `/`, incl. empty) and `?` (one char) over the whole key; ignored targets have no row and are never resolved | §7.1 `ignore = ["registry.example.com/legacy/*"]`; main Q6 (no row, no count) | glob crate semantics (`*` not crossing `/` would make `registry/*` miss `registry/a/b`). Open for main 4 |
| T-D18 | `AgePolicy { Allow (default), Block }`; guard active iff `min_age_days > 0 ∧ (docker.io ∨ unknown_age = Block)`; non-docker.io + Block → no timestamps (no request) → skip with its own reason `release age unknown ({host}; release_age.unknown = block)` (one more branch in `decide_image`, PR-L10) | §6.8 verbatim; the age is *unknown*, not too young, so the docker.io text `release age: no candidate ≥ {d}d` would mislead | fetching per-registry timestamps (not available, §6.8) |
| T-D19 | Report v1 model in pin-core: `schema_version`, `plugins.<id>.{summary, errors}`, `follow_ups`; `Report::exit_code(check)`; JSON fields `#[serde(skip)]` until T28 so earlier tasks keep JSON snapshots byte-identical | the model is needed by `run` (T12) before the visible schema change; one commit owns each snapshot diff | changing model and JSON in one commit |
| T-D20 | `run` processes files **sequentially** (per-file `plan` keeps its concurrency bound); file errors become `ErrorEntry { file, line, message }` under the plugin, later files continue | §6.3; memo makes repeated refs cheap; sequential keeps output order and writes deterministic | files in parallel (no measured need) |
| T-D21 | Human output: row `{file}:{line}  {plugin}:{kind}  {status}[ ({reason})]  {old} -> {new|-}  # {reference}`; the summary line (total, format unchanged) always; when > 1 plugin ran, one `{plugin}: pinned: …` line per plugin follows (main, PR-L1: §6.4 verbatim); then `follow-up: {text}` lines (dedup, first-seen order); file errors on **stderr** as `pin-toolkit: {file}[:{line}]: {message}` in both modes | §6.4; D16 (no row for disabled kinds) | errors on stdout (would corrupt `--json`) |
| T-D22 | Fatal errors (exit 2 before the run: usage, config, routing, not-in-build): stderr `pin-toolkit: {msg}`, **stdout empty** (also with `--json`) | a half-written report is worse than none; CI greps the exit code | a JSON error object (schema growth). Open for main 6 |
| T-D23 | Walk: git root = first ancestor of `cwd` (incl.) containing a `.git` entry, else `cwd`; `ignore::WalkBuilder` with `hidden(false)` (keeps `.gitlab-ci.yml`, `.gitlab/`), `.git` filtered, gitignore/`.ignore`/global excludes at their defaults; globs (globset, `literal_separator(true)`) match the path relative to the git root; display = path relative to `cwd` (implicit) or as passed (explicit); `rel` = lexically normalised `cwd.join(p)` (`.`/`..` resolved without touching the file system) stripped of `root`; a path outside the root has no `rel` and is claimed only with a subcommand/alias, else `no plugin claims …`; dedup key = the normalised absolute path (first spelling in argument order wins); routes sorted by display (PR-M9) | §6.3; `**/*.gitlab-ci.yml` then also matches this repo's `ci/.gitlab-ci.yml` (`*` matches empty) | globs relative to the walk root (an explicit `ci/` would change what matches) |
| T-D24 | Snapshots: pin-toolkit `tests/fixture_snapshots.rs` reuses `crates/pin-gitlab-ci/tests/common/mod.rs` via `#[path]`; the 38 `__human`/`__json` snapshots `git mv` to `crates/pin-toolkit/tests/snapshots/` (same names), the 19 `__file` snapshots stay; toolkit-written bytes must equal driver-written bytes | §9 gate "toolkit snapshots over the M5a fixtures"; one owner per snapshot (no duplicate diffs in T27–T29) | duplicating all 38 (every format change would diff twice) |
| T-D25 | The pin-ci driver moves **verbatim** to `crates/pin-gitlab-ci/tests/common/driver.rs` (test helper; e2e, e2e_components, live, oracle keep their bodies) | preserves ≈ 45 e2e tests unchanged; toolkit semantics are tested in pin-toolkit | rewriting e2e onto the binary (large churn, no new coverage). Open for main 11 |
| T-D26 | `service` kind: `PinTarget::Image` gains `kind: &'static str` (`IMAGE` or `SERVICE` from the path head); `KINDS` = image, service, component, include | §4 C2 "reported as image until M5d adds the service kind tag"; a field keeps `PinTarget::Image { .. }` patterns compiling | a new `Service` variant (every match grows) |
| T-D27 | `plugins` subcommand output (pinned literal frame, kind lines derived from `KINDS`): `{id}: {enabled | disabled | not in this build}` then per compiled kind `  {kind}: {on|off} — {help}` | §6.6 "compiled-in plugins, kinds, effective enable state" | a table with column alignment (brittle) |
| T-D28 | **CLI** `--only`/`--skip` together with a plugin subcommand/alias → exit 2 (checked in `execute` on the parsed `Cli`); env `PIN_TOOLKIT_ONLY`/`PIN_TOOLKIT_SKIP` are ignored in subcommand/alias mode (main, PR-L6) | ambiguous intent on the command line; a CI-wide env value must not break `pin-ci` | silently ignoring CLI flags; exit 2 on env values (CI-wide settings would break every alias run). Open for main 15 |
| T-D29 | Token-holding types get a **redacting `Debug`** (T7, PR-M5): `GitlabEnv`, `GitlabAuth`, `RegistryEnv` (prints keys only), `GitLabClient` (`token`), `HostCreds`; values print as `<redacted>` | M5a–c review M2: tokens land in T7; `{:?}`, `dbg!`, failing `assert_eq!` must not leak `CI_JOB_TOKEN` into CI logs | derived `Debug` (leaks) |
| T-D30 | §7.3 "unbound host fails → `unresolved (<host>: not trusted for credentials — bind it in …)`" is **deferred to M5e'** (main, PR-M6 option b); in M5d a non-primary `gitlab-hosts` host resolves anonymously and a failure keeps today's `unresolved` reason; recorded in T31 (spec §7.3 as built, README) | the message points at the global config, which exists only from M5e' | implementing it now with a message naming a file that cannot exist yet |

## Gate (spec §11; no principles document in the repo)

| Property | Status | Where |
|---|---|---|
| Byte-exact | PASS | unchanged P-bytes/P-span (pin-gitlab-ci); T21 P-same-bytes (toolkit bytes = driver bytes ∀ fixture × mode); T3 non-pinned edits never written |
| Idempotent | PASS | unchanged `idempotent_over_all_fixtures`; T22 row "second run with `--check` → 0" |
| Snapshot review | PASS | T21 move (R100, no diff); T26 headers only (mechanical check); T27 (19 human), T28 (19 JSON), T29 (service subset) — each list in the commit body, each with a mechanical diff check (PR-L4) |
| Lazy resolvers | PASS | unchanged construction counters; T6 memo keeps laziness (`resolvers_built()` empty for scan-only runs); T19 not-built plugins build nothing; T22 row "templated only → no resolver" |
| Plugin isolation | PASS | T13 `compiled_plugins_are_known`; T25 `cargo check -p pin-gitlab-ci -p pin-toolkit` + `cargo tree` feature check (D38, PR-M4); pin-core matrix unchanged + `test-support` leg; pin-toolkit `--no-default-features` leg |
| Fail loud | PASS | T22 exit table: parse error → 2 (D32), `[hosts]` → 2, unknown key → 2, bad glob → 2, unclaimed explicit file → 2, not built → 2; resolver construction error stays `unresolved` exit 1 (D36, unchanged core test) |
| Planted violations | PASS once every box is ticked | spec's "put `[hosts]` in a repo file" = T18 planted (a) + T22 row |
| §9 M5d gate items | PASS | toolkit snapshots T21/T27–T29; exit-code table T22; repo `[hosts]` → 2 T18/T22; wiremock `CI_JOB_TOKEN` never to another host T7 (core) + T23 (binary path); `[[bin]] pin-ci` deleted T24 |

## Files to read first (implementer step 0)

Spec §6.1–§6.9, §7.1–§7.3, §8.1–§8.2, §9 (M5d, M5e', M5f rows), §11, §12 D9–D19/D29–D38, §13 O1, §14.1. Review `.superpowers/sdd/2026-11-10-m5abc/code-review.md` (L2–L10, N5, M3).
Code: `crates/pin-core/src/{run.rs, plugin.rs, report.rs, edit.rs:90-130, text.rs, gitlab.rs:90-160 and :260-345, oci/registry.rs:30-60 and :220-260, oci/decide.rs, oci/mod.rs, lib.rs}`,
`crates/pin-core/tests/{run_pin_file.rs:1-130, mock_registry.rs:1-80, gitlab_client.rs:1-80, oci_decide.rs:1-90, live_gitlab.rs:90-150}`, `crates/pin-core/src/oci/registry.rs:55-110` (`HostCreds`, `Creds::load_from`/`load_from_env`), `crates/pin-core/Cargo.toml`,
`crates/pin-gitlab-ci/src/{lib.rs, policy.rs, driver.rs, main.rs, discover.rs:1-120 and :190-330}`, `crates/pin-gitlab-ci/tests/{common/mod.rs, fixture_snapshots.rs, fixture_properties.rs:1-35, plugin.rs:1-120, discovery_parity.rs:1-75, e2e.rs:1-120 and :270-330}`, `crates/pin-gitlab-ci/Cargo.toml`, root `Cargo.toml`, `README.md`.
Crate docs (local after T13 locks them): `ignore::WalkBuilder` (`hidden`, `filter_entry`, `sort_by_file_name`), `globset::GlobBuilder::literal_separator`, `toml::Table`, clap derive (`global = true` args, optional subcommand next to a positional `Vec`), `Error::kind()`; insta `INSTA_FORCE_UPDATE` (T26).

## Type-checking strategy

Gate clippy (`-D warnings`, workspace, all features) + pin-core matrix + pin-toolkit `--no-default-features` + (T25) `cargo check -p pin-gitlab-ci -p pin-toolkit` + (T25) `cargo tree -e normal,build` feature check. Expected rejections that prove structure:
- T1: removing `impl PartialEq<&str> for Status` rejects every `status == "…"` site (E0277) — the compiler lists all 19.
- T4: the new `Plugin::resolve` signature rejects every impl and direct call (E0053/E0308); `trailing_comment` arity rejects its callers.
- T7: deleting `GitLabClient::from_env` rejects `driver::gitlab_from_env` until it uses `GitlabEnv::auth_for`, and the three calls in `tests/live_gitlab.rs` (compiled under `--all-features`); deleting `ResolverEnv.gitlab_host` rejects the three `ResolverEnv { gitlab_host: … }` literals (constraint 2, T7).
- T8: removing `GitlabCi::gitlab_hosts` rejects every struct literal and `policy::resolve_component(&self.gitlab_hosts, …)`.
- T9: the new `UpdatePolicy::unknown_age` field rejects the one full literal `driver.rs:101` (E0063); the two `..Default::default()` literals compile unchanged.
- T10: the new `Row::follow_up` field rejects the six `Row { … }` test literals (constraint 2, T10) and the production literal in `pin_file` (E0063).
- T11: `plan`/`pin_file` taking `&Selection` rejects every `&KindSet` call site.
- T25: without `test-support`, `pin_core::oci::RegistryClient` / `pin_core::gitlab::GitLabClient` / `Ctx::with_oci` are unresolved in plugin and binary code (E0432/E0599); `dead_code` on items only tests use is allowed by `#[cfg_attr(not(feature = "test-support"), allow(dead_code))]` on those modules. `cargo check` cannot see a normal dependency that *enables* `test-support`; the `cargo tree` gate line does (PR-M4, verified on a scratch copy: steady state silent, planted normal-dep feature prints `FAIL D38`).
- pin-toolkit `--no-default-features`: any unguarded `pin_gitlab_ci::` path (E0433).
- T29: `PinTarget::Image { kind, .. }` construction rejects literals missing `kind`.

## Testing strategy (per layer)

| Layer | Covers | Needs |
|---|---|---|
| Unit (pin-core) | `Status`, `RegError: Clone`, `Memo`, `RefPatterns`, `GitlabEnv::auth_for` table, registry creds from the env map, redacting `Debug`, `AgePolicy`, `Report` assembly/exit code/render | inline values |
| Core integration (`run_pin_file.rs`, new `run_routes.rs`, `memo.rs`, `gitlab_hosts.rs`) | L2, ignore, `run` continuation + errors + follow-ups, memo coalescing, D37 on the wire | fake plugin, `.tmp/` files, wiremock, `test-support` |
| Plugin (`pin-gitlab-ci` unit + `tests/plugin.rs`, `discover.rs`) | L9 host gate, ignore keys, `service` kind | inline texts, no network |
| Toolkit unit (`src/config/*`, `src/cli.rs`, `src/walk.rs`, `src/known.rs`) | fold precedence, repo/env/CLI parsing, unknown-env warnings, argv[0], walk/routing/display, `..`/`./` normalisation, walked 2+ claims | inline values, `.tmp/` repos with `.git/` |
| Toolkit integration (`tests/*.rs` via `execute`) | config layers end to end, exit-code table, credentials on the wire, end-to-end pin, memo across files, JSON errors | wiremock fakes via `tune`, `.tmp/` repos |
| Snapshot (`crates/pin-toolkit/tests/fixture_snapshots.rs`) | human + JSON over the 17 M5a fixtures (19 runs); P-same-bytes vs driver | shared `common` harness |
| Binary (`tests/aliases.rs`, std `Command` + `CARGO_BIN_EXE_pin-toolkit`, `#[cfg(unix)]` symlinks) | argv[0] dispatch `pin-ci`/`pin-gitlab-ci`/`pin-helm` | `.tmp/`, no network (templated targets only) |
| pin-gitlab-ci e2e/properties (existing) | plugin behaviour through the moved driver — unchanged bodies | wiremock |
| Live / oracle | real registries (`PIN_CI_LIVE=1`: pin-gitlab-ci `live.rs`, pin-core `live_gitlab.rs`, new pin-toolkit `live.rs`), old pipeline (`PIN_CI_ORACLE=1`) | network; close-out only |

## Properties (derived from the SUT)

𝔽 = `crates/pin-gitlab-ci/tests/fixtures/*.yml` (17), M(f) = modes(f); L = [defaults, repo, env, cli].
- **P-prec (T14):** ∀ scalar field x: fold(L).x = L_k.x for the largest k with L_k.x ≠ ∅, else default(x); `files`, `gitlab-hosts`: same, whole list; `ignore` = L_0.ignore ⧺ … ⧺ L_3.ignore; `kinds[p][k]`: per key as scalars. Test enumerates all 2⁴ presence patterns per field kind and derives the expectation from the pattern, not from literals.
- **P-exit (T10, T22, T28):** code = 2 ⇔ fatal ∨ errors ≠ ∅; else 1 ⇔ unresolved > 0 ∨ (check ∧ pinned + updated > 0); else 0. T22 additionally recomputes the code from the printed JSON summary and asserts equality with the process code; `plugins.*.errors` is serialised only from T28, which extends the recomputation to the errors (PR-L8).
- **P-check-pure (T22):** `--check` ∨ `--dry-run` ⇒ ∀ files: bytes after = bytes before.
- **P-route (T20):** ∀ walked f: f routed ⇔ |{p enabled : globs_p(rel f)}| = 1; |…| ≥ 2 ⇒ f not routed ∧ one `ErrorEntry` for f (exit 2, others run; PR-L11); routes sorted by display and unique by normalised absolute path (PR-M9).
- **P-same-bytes (T21):** ∀ f ∈ 𝔽, m ∈ M(f): bytes written by `pin-toolkit gitlab-ci` = bytes written by the driver.
- **P-token (T7, T23):** ∀ request r received by host h: `JOB-TOKEN` ∈ r ⇒ h = CI_SERVER_HOST; `PRIVATE-TOKEN` ∈ r ⇒ h = primary. Also for the default primary taken from `CI_SERVER_HOST` (T23 `ci_server_host_is_default_primary`, PR-M10).
- **P-redact (T7):** ∀ token-holding value v with secret s: `format!("{v:?}")` ⊅ s (PR-M5).
- **P-memo (T6, T21):** ∀ key k within one run: #requests(k) ≤ 1, incl. failed lookups and concurrent callers.
- **P-known (T13):** ∀ compiled p: p.id ∈ ids(KNOWN) ∧ kinds(p) ⊆ KNOWN[p.id].
- **P-ignore (T11):** ∀ target t with ignore_key(t) = k: pattern k ⇒ no row for t ∧ resolve(t) never called; ∀ other targets rows unchanged.
- **P-sum (T10):** Σ_p plugins[p].summary = summary (componentwise); rows of p counted only under p.
- Literals pinned only where they are the criterion: the user texts below, the `plugins` frame, the exit codes in the T22 table.

## Scenario coverage (spec → check)

| Spec scenario | Check |
|---|---|
| §6.3 inputs = explicit files/dirs or git root (fallback cwd), `.gitignore`, sorted | T20 `root_walk_respects_gitignore_and_hidden`, `no_git_root_falls_back_to_cwd`, `explicit_dir_is_walked` |
| §6.3 per-file read/parse/write failure → `plugins.<id>.errors`, others continue, exit 2 | T12 `run_continues_after_file_error`, `run_parse_error_has_line`; T22 rows 6–7; T28 `json_lists_file_errors` |
| §6.2 explicit path claimed by 0 or 2+ plugins → exit 2 with a hint; walk ignores unclaimed; `..`/`./`/outside-root spellings | T20 `explicit_file_claim_rules`, `explicit_parent_path_from_subdir`, `same_file_two_spellings_one_route`; T22 row 10 |
| §6.3 walked file claimed by 2+ plugins → per-file error, others continue (T-D10, PR-L11) | T20 `walked_file_claimed_twice_is_a_file_error` (fake ids; end to end only from M7) |
| §6.4 JSON v1 fields (`schema_version`, `plugin`, `kind`, `plugins`, `follow_ups`), D15 | T28 `json_schema_v1` + 19 JSON snapshots |
| §6.4 human row `plugin:kind`, total summary line then per-plugin summaries when > 1 plugin ran, `follow-up:` lines | T27 `human_render_exact`, `human_render_multi_plugin` + 19 human snapshots |
| §6.4/D16 disabled kind → no row | T29 `service_kind_off_drops_service_rows`; T17/T19 `--kind` mapping |
| §6.5 exit codes 0/1/2, `--check` | T10 `exit_code_rules`; T22 `exit_code_table` |
| §6.6 `pin-toolkit [PATHS]`, `<plugin>` subcommand, `pin-<plugin>` aliases, `plugins`, option list | T13 `plugins_lists_compiled_and_known`; T17 `cli_flags_map_to_layer`, `paths_and_subcommands_parse`, `argv0_aliases`; T24 alias binary tests |
| §6.6/D33 `pin-ci` = plain argv[0] alias only, `[[bin]] pin-ci` deleted | T24 `pin_ci_alias_is_gitlab_ci_subcommand`; `cargo metadata` check in T24 |
| §6.7 memo keys, errors cached, coalesced, per run | T6 (4 tests); T21 `same_image_in_two_files_is_one_lookup` |
| §6.8 `release_age.unknown` allow/block | T9 (2 tests, block reason `release age unknown (…)`); T15/T16 parsing |
| §6.9/D32 parse error → exit 2 | T22 row 6 |
| §7.1 layers defaults < repo < env < CLI, `version = 1`, unknown keys, merge rules, `--no-config`, `--config`/`PIN_TOOLKIT_CONFIG`; unknown `PIN_TOOLKIT_*` → warning | T14 `fold_*`; T15 `repo_*`; T16 `env_unknown_vars_warn`; T22 row 16; T18 `precedence_repo_env_cli`, `config_flag_and_env_path`, `no_config_skips_repo_file` |
| §7.2 `[hosts]` (global-only) in a repo file → exit 2 | T15 `repo_global_only_keys`; T18 `repo_hosts_exits_2`; T22 row 9 |
| §7.3/D37 `CI_JOB_TOKEN` → `CI_SERVER_HOST` only; D31 `GITLAB_TOKEN` → primary only | T7 `auth_for_binds_tokens` + 3 wiremock tests; T23 (3 binary-path tests incl. default primary from `CI_SERVER_HOST`) |
| §7.3 unbound host fails → `not trusted for credentials` message | **deferred to M5e'** (T-D30, main); recorded in T31 spec §7.3 as built + README |
| §7.1 helm `aliases` are URLs, "never credentials" | T15 `repo_rejects` row (userinfo in an alias URL, PR-L14) |
| §7.1 `gitlab-hosts` = hosts to resolve, others `skipped (host not enabled: …)` | T8 `component_host_enabled_by_env`; existing e2e_components "not enabled" test |
| §7.1 `ignore`, `kinds`, `files`, `run` per plugin | T11 (ignore), T19 (`run`), T20 (`files`), T29 (`kinds`) |
| §8.2/O1 config for an unbuilt plugin → notice, exit unaffected; `pin-toolkit <p>` → exit 2 | T19 `not_built_plugin_in_config_is_a_notice`, `only_and_subcommand_rules`; T24 `pin_helm_alias_not_in_build` |
| §9 `service` kind tag | T29 |
| §14.1 M3/D38, L2, L3, L4, L5, L6, L8, L9, L10, N5 | T25, T3, T4, T2/T6, T2, T1, T31 (README), T8, T5, T5 |

## Coverage gap (what a human runs)

1. Live and oracle suites (network, crane, renovate): at close-out `PIN_CI_LIVE=1 cargo test -p pin-gitlab-ci --test live`, `PIN_CI_LIVE=1 cargo test -p pin-core --features gitlab,test-support --test live_gitlab`, `PIN_CI_LIVE=1 cargo test -p pin-toolkit --test live`, and in devenv `PIN_CI_ORACLE=1 cargo test -p pin-gitlab-ci --test oracle`.
2. Dogfood: `cargo run -p pin-toolkit -- --check` at the repo root → exits 1 or 0, lists `ci/.gitlab-ci.yml` rows with `gitlab-ci:image`/`gitlab-ci:service`, writes nothing (`git status` clean).
3. A real GitLab pipeline with `CI_JOB_TOKEN` (`pin-toolkit --check --json > pin-report.json`, §8.4) — not reproducible here; the wiremock tests stand in.
4. Installed aliases (symlinks) are M6; T24 covers argv[0] via symlinks in `.tmp/` on unix only (no Windows `.exe` test).
5. The walk honours the user's global gitignore and `.ignore` files (crate defaults); results can differ per machine — documented in the README.
6. Multi-plugin output (per-plugin summary lines, follow-ups, 2+ claims of a walked file) is exercised only with fake plugins/ids until M7.
7. Process-level output: `pin-toolkit --json | head -c 10` (EPIPE must not panic) and an unreadable cwd (exit 2) — `main.rs` is the untested shell (PR-L12); run once by hand at close-out.

## Review Focus

1. **`pin-toolkit` with no arguments in a subdirectory** walks the whole git root and prints paths relative to the cwd (`../.gitlab-ci.yml`) — T20 `display_relative_to_cwd`.
2. **`CI_JOB_TOKEN` in a GitLab job that resolves a gitlab.com component**: never sent to gitlab.com — T7 wiremock + T23 `ci_job_token_never_sent_to_another_host` and `ci_server_host_is_default_primary` (no `--gitlab-host`: the primary comes from `CI_SERVER_HOST`, PR-M10).
3. **`--check` in CI**: exit 1 when anything would change, nothing written, 0 after a pin run — T22 rows 2, 3, 5 + P-check-pure.
4. **A typo or a `[hosts]` table in `.pin-toolkit.toml`**: exit 2, message names the file and the key, no file touched — T15 `repo_rejects`, T18 `repo_hosts_exits_2`.
5. **One broken YAML file among several**: the others are still pinned, exit 2, stderr `pin-toolkit: <file>:<line>: invalid YAML: …` — T12 + T22 row 6.
6. **`pin-ci` muscle memory**: `pin-ci` with no paths now walks the repo (no `ci/.gitlab-ci.yml` default), and accepts any explicit file — T24 alias tests + README.

---

## Snippets (shared contracts)

### pin-core

```rust
// report.rs (T1, T10, T27, T28)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = "lowercase")]   // derives unchanged (no Deserialize, PR-L12)
pub enum Status { Pinned, Ok, Updated, Skipped, Unresolved }          // T1: no PartialEq<&str>/<str>
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]             // T10: + Default, PartialEq, Eq (PluginReport/Report derive them, PR-L12)
pub struct Summary { /* fields unchanged: pinned, ok, updated, skipped, unresolved */ }
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    pub plugin: &'static str, pub kind: &'static str,                  // T28: serialized (were #[serde(skip)])
    pub file: String, pub line: usize, pub reference: String, pub old: String,
    pub new: Option<String>, pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")] pub reason: Option<String>,
    #[serde(skip)] pub follow_up: Option<String>,                      // T10
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorEntry { pub file: String, #[serde(skip_serializing_if = "Option::is_none")] pub line: Option<usize>, pub message: String }
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PluginReport { pub summary: Summary, pub errors: Vec<ErrorEntry> }
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    #[serde(skip)] pub schema_version: u32,                            // T28: serialized first
    pub results: Vec<Row>, pub summary: Summary,
    #[serde(skip)] pub plugins: BTreeMap<String, PluginReport>,        // T28: serialized
    #[serde(skip)] pub follow_ups: Vec<String>,                        // T28: serialized
}
impl Report {
    pub const SCHEMA_VERSION: u32 = 1;
    pub fn new(rows: Vec<Row>) -> Self;                                // = assemble(rows, &[], vec![]) — kept for driver/tests
    /// `ran`: plugins with ≥ 1 routed file (entry even without rows). Follow-ups: rows' `follow_up`, dedup, first seen.
    pub fn assemble(rows: Vec<Row>, ran: &[&'static str], errors: Vec<(&'static str, ErrorEntry)>) -> Self;
    pub fn exit_code(&self, check: bool) -> u8;                        // P-exit without "fatal"
    pub fn render_human(&self) -> String;                              // T-D21 (T27): rows, total summary line, per-plugin lines iff plugins.len() > 1, follow-ups
    pub fn to_json(&self) -> String;                                   // pretty; field order = struct order
}
```

```rust
// plugin.rs (T4, T11)
pub struct Source<'a> { pub path: &'a Path, pub text: &'a str, pub lines: LineIndex }
impl<'a> Source<'a> { pub fn new(path: &'a Path, text: &'a str) -> Self }
#[derive(Debug, Clone, Default)] pub struct RefPatterns(Vec<String>);
impl RefPatterns { pub fn new(p: impl IntoIterator<Item = String>) -> Self; pub fn is_empty(&self) -> bool; pub fn matches(&self, key: &str) -> bool }
#[derive(Debug, Clone)] pub struct Selection { pub kinds: KindSet, pub ignore: RefPatterns }
impl From<KindSet> for Selection { /* ignore empty */ }
pub trait Plugin: PluginMeta {
    type Target;
    fn scan(&self, path: &Path, text: &str, on: &KindSet) -> Result<Vec<Self::Target>, ScanError>;   // unchanged
    fn ignore_key(&self, _t: &Self::Target) -> Option<String> { None }                               // T11
    async fn resolve(&self, ctx: &Ctx, src: &Source<'_>, t: &Self::Target) -> Finding;                // T4
}
pub trait DynPlugin: PluginMeta {
    fn plan<'a>(&'a self, ctx: &'a Ctx, path: &'a Path, text: &'a str, sel: &'a Selection, concurrency: usize)
        -> LocalBoxFuture<'a, Result<Vec<Finding>, ScanError>>;
}
// blanket plan: ts = scan(path, text, &sel.kinds)?; ts.retain(|t| sel.ignore.is_empty() || !self.ignore_key(t).is_some_and(|k| sel.ignore.matches(&k)));
//               let src = Source::new(path, text);   // the one LineIndex per file
//               resolve each t under Semaphore(concurrency.max(1)) — unchanged.
```

Wildcard (T11), over `char`s, whole string, iterative two-pointer with backtrack to the last `*`:

```text
match(p, s): i=j=0; star=None
  while j < |s|: if i<|p| ∧ (p[i]='?' ∨ p[i]=s[j]) {i++, j++}
                 elif i<|p| ∧ p[i]='*' {star=(i, j); i++}
                 elif star=(si, sj) {i=si+1; j=sj+1; star=(si, sj+1)}
                 else return false
  while i<|p| ∧ p[i]='*': i++
  return i = |p|
```

```rust
// run.rs (T6, T7, T8, T9, T12)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)] pub enum AgePolicy { #[default] Allow, Block }
#[derive(Debug, Clone, Default, PartialEq, Eq)]   // existing derives kept (Effective derives PartialEq, PR-L12)
pub struct UpdatePolicy { pub enabled: bool, pub allow_major: bool, pub min_age_days: u32, pub unknown_age: AgePolicy }

#[derive(Clone, PartialEq, Eq)] pub enum GitlabAuth { Anonymous, Private(String), Job(String) }   // pure data, no client
impl GitlabAuth { pub fn into_parts(self) -> (Option<String>, bool) /* (token, is_job_token) */ }
impl fmt::Debug for GitlabAuth { /* Anonymous | Private(<redacted>) | Job(<redacted>) — T-D29, PR-M5 */ }
#[derive(Clone, PartialEq, Eq)]                                     // Debug hand-written: tokens print as Some(<redacted>) (T-D29)
pub struct GitlabEnv { pub primary: String, pub hosts: BTreeSet<String>, pub gitlab_token: Option<String>, pub ci_job_token: Option<String>, pub ci_server_host: Option<String> }
impl Default for GitlabEnv { /* primary "gitlab.com", rest empty */ }
impl GitlabEnv {
    /// primary = `primary` (toolkit: CLI > PIN_TOOLKIT_GITLAB_HOST) else CI_SERVER_HOST else "gitlab.com"; empty values = unset.
    pub fn from_vars(vars: &BTreeMap<String, String>, primary: Option<String>, hosts: BTreeSet<String>) -> Self;
    pub fn enabled(&self, host: &str) -> bool;                      // host == primary ∨ hosts ∋ host
    pub fn resolve_set(&self) -> BTreeSet<String>;                  // {primary} ∪ hosts
    pub fn auth_for(&self, host: &str) -> GitlabAuth;               // T-D12: Private if host==primary ∧ token; else Job if Some(host)==ci_server_host ∧ job token; else Anonymous
}
pub const REGISTRY_VARS: [&str; 3] = ["DOCKERHUB_USER", "DOCKERHUB_TOKEN", "GITHUB_TOKEN"];   // PR-M2
#[derive(Clone, Default, PartialEq, Eq)] pub struct RegistryEnv(BTreeMap<String, String>);    // REGISTRY_VARS ∩ vars, empty values dropped; Debug prints keys only (T-D29)
#[derive(Debug, Clone, Default)]                                    // derived Debug is safe: every token field redacts itself
pub struct ResolverEnv {
    pub home: Option<PathBuf>, pub gitlab: GitlabEnv, pub registry: RegistryEnv,
    #[cfg(feature = "test-support")] #[doc(hidden)] pub base_urls: BaseUrls,
}
impl ResolverEnv {
    pub fn from_vars(vars: &BTreeMap<String, String>, primary: Option<String>, hosts: BTreeSet<String>) -> Self;  // home = HOME; registry = REGISTRY_VARS
    #[cfg(feature = "oci")] pub(crate) fn creds(&self) -> Creds;   // = Creds::load_from_env(self.home.as_deref(), |v| self.registry.0.get(v).cloned()) — no process env (PR-M2)
    #[cfg(any(test, feature = "test-support"))] #[doc(hidden)]       // gate added in T25 (PR-L9); T7–T24: plain pub
    pub fn from_process(primary: &str) -> Self;   // = from_vars(&std::env::vars().collect(), Some(primary), ∅); driver/live/oracle only
}
// oci/registry.rs (T7): `fn load_from_env` → `pub(crate) fn load_from_env`; `HostCreds` Debug hand-written (Basic { user, password: <redacted> }, Bearer { token: <redacted> }).
// gitlab.rs (T7): `GitLabClient` Debug hand-written (`token: Some(<redacted>)`, other fields as today).
#[cfg(feature = "test-support")] #[doc(hidden)] #[derive(Debug, Clone, Default)]
pub struct BaseUrls { pub registry: HashMap<String, String>, pub hub: Option<String>, pub gitlab: BTreeMap<String, String> /* host → API base */ }

struct Memo<K, V>(RefCell<HashMap<K, Rc<tokio::sync::OnceCell<V>>>>);
impl<K: Eq + Hash, V: Clone> Memo<K, V> {
    async fn get<F: Future<Output = V>>(&self, k: K, f: impl FnOnce() -> F) -> V {
        let cell = Rc::clone(self.0.borrow_mut().entry(k).or_default());   // borrow ends before the await
        cell.get_or_init(f).await.clone()
    }
}
// Ctx gains: memo fields (digest, tags, times, gitlab); pub fn gitlab_enabled(&self, host: &str) -> bool { self.env.gitlab.enabled(host) }
// Ctx::oci():    RegistryClient::try_new(self.env.creds(), base_urls.registry)?(.with_hub_base(hub)) — Err(String) cached (L5); no Creds::load_from (PR-M2)
// Ctx::gitlab(): GitLabHosts::new(primary, ∀ h ∈ resolve_set: { let (token, job) = auth_for(h).into_parts(); GitLabClient::try_new(base(h), token, job)? })
//                base(h) = base_urls.gitlab[h] or "https://{h}/api/v4"; Err → "gitlab: {h}: {e}"

pub struct Route<'a> { pub plugin: &'a dyn DynPlugin, pub sel: &'a Selection, pub path: PathBuf /* I/O */, pub display: String }
/// Sequential over routes; `pin_file` each; rows' `file` = display; FileError → ErrorEntry under the plugin; never stops early.
pub async fn run(routes: &[Route<'_>], ctx: &Ctx, o: RunOptions) -> Report;
impl FileError { pub fn entry(&self, display: &str) -> ErrorEntry }   // Read/Write → line None; Parse → line
```

### pin-gitlab-ci

```rust
pub const IMAGE: &str = "image"; pub const SERVICE: &str = "service"; pub const COMPONENT: &str = "component"; pub const INCLUDE: &str = "include";  // T5 (SERVICE: T29)
pub const KINDS: [KindSpec; 4] = [ /* T29 */
    KindSpec { id: IMAGE, default_on: true, help: "image: references (C1)" },
    KindSpec { id: SERVICE, default_on: true, help: "services: references (C2)" },
    KindSpec { id: COMPONENT, default_on: true, help: "top-level component: references (C3)" },
    KindSpec { id: INCLUDE, default_on: true, help: "include: project/ref mappings (C4)" },
];
#[derive(Debug, Clone, Copy, Default)] pub struct GitlabCi;                        // T8 (L9)
// discover.rs (T29): PinTarget::Image { kind: &'static str, line, start, end, value }; fn image_kind(p: &[PathSeg]) -> Option<&'static str>  // head "image" → IMAGE, "services" → SERVICE
// policy.rs (T5, N5): no `let … else { unreachable!() }`; callers pass the destructured fields:
pub(crate) fn unsupported(src: &Source<'_>, t: &PinTarget, kind: &'static str, reason: &'static str) -> Finding;
pub(crate) async fn resolve_component(ctx: &Ctx, src: &Source<'_>, t: &PinTarget, parts: Option<&ComponentParts>) -> Finding;   // gate: ctx.gitlab_enabled(host) (T8)
pub(crate) async fn resolve_include(ctx: &Ctx, src: &Source<'_>, t: &PinTarget, project: &str) -> Finding;
// ignore_key (T11): IncludeRef { project, value, .. } → "{project}@{value}"; every other target → t.value()
```

### pin-toolkit

```toml
# crates/pin-toolkit/Cargo.toml (T13)
[features]
default = ["gitlab-ci"]
gitlab-ci = ["dep:pin-gitlab-ci"]
[dependencies]
pin-core = { workspace = true }
pin-gitlab-ci = { workspace = true, optional = true }        # root: pin-gitlab-ci = { path = "crates/pin-gitlab-ci" }
clap = { workspace = true }; serde = { workspace = true }; serde_json = { workspace = true }
tokio = { workspace = true, features = ["rt"] }
toml = { workspace = true }; ignore = { workspace = true }; globset = { workspace = true }   # root: toml "0.9", ignore "0.4", globset "0.4"
[dev-dependencies]
pin-core = { workspace = true, features = ["oci", "gitlab", "yaml", "test-support"] }
pin-gitlab-ci = { workspace = true }
tokio = { workspace = true, features = ["rt-multi-thread", "macros"] }
wiremock = { workspace = true }; insta = { workspace = true }; serde_json = { workspace = true }
```

```rust
// lib.rs
pub struct Process { pub args: Vec<OsString>, pub env: BTreeMap<String, String>, pub cwd: PathBuf }
impl Process { pub fn from_os() -> std::io::Result<Self> }           // the only std::env reader (non-UTF-8 vars dropped)
#[derive(Debug, Default)] pub struct Output { pub stdout: String, pub stderr: String, pub code: u8 }
pub async fn execute(p: Process, tune: impl FnOnce(&mut ResolverEnv)) -> Output;   // tests: tune sets base_urls
pub fn main_with(p: Process) -> Output;                               // current-thread runtime, block_on(execute(p, |_| {}))
// main.rs (PR-L12): let out = match Process::from_os() { Ok(p) => main_with(p),
//     Err(e) => Output { stderr: format!("pin-toolkit: cannot read the current directory: {e}\n"), code: 2, ..Default::default() } };
//   let _ = io::stdout().write_all(out.stdout.as_bytes()); let _ = io::stderr().write_all(out.stderr.as_bytes());   // no print!: EPIPE (`| head`) must not panic
//   ExitCode::from(out.code)

// known.rs
pub struct Known { pub id: &'static str, pub kinds: &'static [&'static str] }
pub const KNOWN: [Known; 3] = [
    Known { id: "gitlab-ci", kinds: &["image", "service", "component", "include", "helm-oci"] },
    Known { id: "helm", kinds: &["chart-dependency", "values-image", "helmfile-release"] },
    Known { id: "github-actions", kinds: &["action", "docker-action", "container"] },
];
pub const ALIASES: [(&str, &str); 1] = [("pin-ci", "gitlab-ci")];     // D33: the one dispatch entry
pub fn compiled() -> Vec<&'static dyn DynPlugin>;                     // #[cfg(feature = "gitlab-ci")] static GITLAB_CI: GitlabCi = GitlabCi;

// cli.rs (clap derive)
#[derive(Parser)] #[command(name = "pin-toolkit", version)]   // NO args_conflicts_with_subcommands (PR-H1): with it, clap 4.6.7 stops
// recognising subcommands after any global flag: [--check, gitlab-ci, --json, a.yml] → paths [gitlab-ci, a.yml], cmd None.
// Without it (verified, clap 4.6.7): that input → cmd GitlabCi{[a.yml]}, both flags; [--dry-run, gitlab-ci] → GitlabCi{[]};
// a subcommand is recognised only before the first path: [a.yml, gitlab-ci, b.yml] → paths [a.yml, gitlab-ci, b.yml], cmd None
// (so `cmd.is_some() ∧ paths ≠ ∅` cannot occur; `gitlab-ci` there is a path and later fails as `no plugin claims gitlab-ci …`).
pub struct Cli { #[command(flatten)] pub opts: Opts, pub paths: Vec<PathBuf>, #[command(subcommand)] pub cmd: Option<Cmd> }
#[derive(Subcommand)] pub enum Cmd { #[command(name = "gitlab-ci")] GitlabCi { paths: Vec<PathBuf> }, Helm { paths: Vec<PathBuf> },
    #[command(name = "github-actions")] GithubActions { paths: Vec<PathBuf> }, Plugins }
#[derive(Args)] pub struct Opts {   // every arg `global = true`
    dry_run: bool, check: bool, json: bool, update: bool, update_major: bool,
    min_age_days: Option<u32>, concurrency: Option<usize>,
    #[arg(value_delimiter = ',')] only: Option<Vec<String>>, #[arg(value_delimiter = ',')] skip: Option<Vec<String>>,
    kind: Vec<String> /* "<p>:<k>=on|off", repeatable */, config: Option<PathBuf>, no_config: bool, gitlab_host: Option<String> }
pub enum Mode { All, Plugin(&'static str), Plugins }
pub fn dispatch_argv0(args: Vec<OsString>) -> Vec<OsString>;          // basename (minus ".exe"): ALIASES, else "pin-<id>" with id ∈ KNOWN → insert id at 1
pub fn parse(args: &[OsString]) -> Result<(Cli, Mode), clap::Error>;
pub fn layer(cli: &Cli) -> Result<Layer, Fatal>;                      // presence flags → Some(true); --kind validated against KNOWN
// T-D28 (T19, in execute): Mode::Plugin(_) ∧ (cli.opts.only ∨ cli.opts.skip) → Fatal — CLI only; env ONLY/SKIP never reach this check

// config/mod.rs
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layer {
    pub update: Option<bool>, pub update_major: Option<bool>, pub min_age_days: Option<u32>, pub unknown_age: Option<AgePolicy>,
    pub concurrency: Option<usize>, pub json: Option<bool>, pub dry_run: Option<bool>, pub check: Option<bool>,
    pub only: Option<Vec<String>>, pub skip: Option<Vec<String>>, pub gitlab_host: Option<String>,
    pub plugins: BTreeMap<String, PluginLayer>,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginLayer { pub run: Option<bool>, pub files: Option<Vec<String>>, pub ignore: Vec<String>,
    pub kinds: BTreeMap<String, bool>, pub gitlab_hosts: Option<Vec<String>>, pub options: Option<toml::Table> /* helm values/helmfile/aliases: validated, unused */ }
#[derive(Debug, Clone, PartialEq)]
pub struct Effective { pub update: UpdatePolicy, pub concurrency: usize /* 8 */, pub json: bool, pub dry_run: bool /* ∨ check */, pub check: bool,
    pub only: Option<Vec<String>>, pub skip: Vec<String>, pub gitlab_host: Option<String>, pub plugins: BTreeMap<String, PluginLayer> }
impl Effective { pub fn fold(layers: &[Layer]) -> Self }               // T-D4/T-D5 — the only precedence code
#[derive(Debug, Clone, PartialEq, Eq)] pub struct Fatal(pub String);   // Display = "pin-toolkit: {0}"
pub mod repo { pub fn parse(path: &Path, text: &str) -> Result<Layer, Fatal>; }
pub mod env  { pub fn layer(vars: &BTreeMap<String, String>) -> Result<Layer, Fatal>;   // known vars only; unknown PIN_TOOLKIT_* skipped
               pub fn unknown_vars(vars: &BTreeMap<String, String>) -> Vec<String>;      // PIN_TOOLKIT_* ∉ known names, sorted (PR-M3)
               pub fn config_path(vars) -> Option<PathBuf>; }
/// Repo file = --no-config ? none : --config ? (must exist) : PIN_TOOLKIT_CONFIG ? (must exist) : <git root>/.pin-toolkit.toml if it exists.
pub fn load(p: &Process, root: &Path, cli: &Cli) -> Result<Effective, Fatal>;

// enable.rs (T19)
pub struct Plan { pub plugins: Vec<(&'static dyn DynPlugin, Selection, GlobSet)>, pub notices: Vec<String> }
pub fn enable(eff: &Effective, mode: &Mode) -> Result<Plan, Fatal>;
// enabled(id) = compiled(id) ∧ (mode = Plugin(id) ∨ (mode = All ∧ (only = None ∨ only ∋ id) ∧ skip ∌ id ∧ plugins[id].run ≠ Some(false)))
//   → in Plugin mode `eff.only`/`eff.skip` (env or CLI) are not consulted: env values are ignored there (PR-L6); CLI values were rejected in execute
// kinds = KindSet::defaults(p.kinds()) then plugins[id].kinds for compiled kinds (known-but-uncompiled kinds, e.g. helm-oci, ignored)
// globs = plugins[id].files or p.default_files(); bad glob → Fatal

// walk.rs (T20)
pub fn git_root(cwd: &Path) -> Option<PathBuf>;                     // = git_root_with(cwd, |p| p.exists())
fn git_root_with(cwd: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf>;   // seam: first ancestor a (incl. cwd) with exists(a/.git)
fn normalize(p: &Path) -> PathBuf;   // lexical: drop `.`, pop on `..` (never above `/`), no fs access (PR-M9)
pub struct Input { pub path: PathBuf /* normalize(cwd.join(p)) */, pub display: String,
    pub rel: Option<String> /* path.strip_prefix(root), '/'-joined; None outside the root (PR-M9) */, pub explicit: bool }
pub fn inputs(cwd: &Path, root: &Path, paths: &[PathBuf]) -> Vec<Input>;   // dedup by `path` (first spelling in argument order), then sorted by display
pub struct Routed { pub routes: Vec<(Input, &'static str)>, pub conflicts: Vec<(&'static str, ErrorEntry)> }
/// explicit: forced ⇒ claimed; else rel = None ∨ 0 claims ⇒ Fatal `no plugin claims …`; 2+ ⇒ Fatal claimed-by.
/// walked: 0 claims ⇒ dropped; 2+ ⇒ conflicts += (first claimant in `claims` order, ErrorEntry { file: display, line: None, message: claimed-by text }) (PR-L11).
pub fn route(inputs: Vec<Input>, claims: &[(&'static str, &GlobSet)], forced: Option<&'static str>) -> Result<Routed, Fatal>;
```

`execute` (T21): `dispatch_argv0` → `parse` (help/version → stdout, 0; else stderr `e.render()`, 2) → stderr += `pin-toolkit: ignoring unknown environment variable {VAR}\n` ∀ VAR ∈ `env::unknown_vars` (T16 wires it in `execute`, PR-M3) →
Plugin mode ∧ CLI `--only`/`--skip` ⇒ Fatal (T19, T-D28) → `root = git_root(cwd) ∪ cwd` → `load` →
`Mode::Plugins` ⇒ listing, 0 → `enable` (notices → stderr) → `inputs` + `route` → `ResolverEnv::from_vars(&p.env, eff.gitlab_host, gitlab-hosts)` → `tune` →
`Ctx::new(eff.update, now_unix(), env)` → `run(&routes, &ctx, RunOptions { dry_run, check, concurrency })` → ∀ (p, e) ∈ `routed.conflicts`: `report.plugins.entry(p).or_default().errors.push(e)` (PR-L11) →
stdout = json ? `to_json() + "\n"` : `render_human()`; stderr += one line per `ErrorEntry`; code = `report.exit_code(check)`. Every `Fatal` → stderr `"{fatal}\n"`, stdout empty, code 2.

### Repo config schema (`.pin-toolkit.toml`, T15) — the repo layer only

```toml
version = 1                                   # required, must be 1
[update]
enabled = false                               # bool
major = false                                 # bool (meaningful only with enabled)
[release_age]
min_days = 0                                  # u32
unknown = "allow"                             # "allow" | "block"
[plugins.gitlab-ci]                           # ids ∈ KNOWN; tables for unbuilt known plugins are validated too
run = true                                    # bool
files = [".gitlab-ci.yml", "**/*.gitlab-ci.yml", ".gitlab/ci/**/*.yml"]   # replaces defaults
ignore = ["registry.example.com/legacy/*"]    # concatenates across layers
gitlab-hosts = ["gitlab.com", "gitlab.example.com"]   # hosts to resolve; never credentials
kinds = { image = true, service = true, component = true, include = true, helm-oci = true }
[plugins.helm]                                # accepted (O1), unused until M7
run = true
kinds = { chart-dependency = true, values-image = true, helmfile-release = true }
values = { tag-digest = true }
helmfile = { oci-digest = false }
aliases = { stable = "https://charts.helm.sh/stable" }
[plugins.github-actions]
run = true
kinds = { action = true, docker-action = true, container = true }
```

Rejected in a repo file (exit 2): `hosts`, `concurrency` (global-only); any other unknown key; unknown plugin id; unknown kind id; wrong types; `version` missing or ≠ 1;
invalid glob; `gitlab-hosts` entry empty or containing `/`, `://` or whitespace; a `plugins.helm.aliases` value that is not a string or whose authority contains userinfo (`@` before the host, PR-L14); `gitlab-host` at top level or under `[plugins.gitlab-ci]` (unknown key — a repo cannot set the primary). Not settable from the repo (no key exists): output format, `dry_run`/`check`, `only`/`skip`, primary GitLab host.

### Env (T16) and other inputs

| Variable | Field | Values |
|---|---|---|
| `PIN_TOOLKIT_UPDATE` / `PIN_TOOLKIT_UPDATE_MAJOR` / `PIN_TOOLKIT_JSON` | `update` / `update_major` / `json` | bool |
| `PIN_TOOLKIT_MIN_AGE_DAYS` | `min_age_days` | u32 |
| `PIN_TOOLKIT_RELEASE_AGE_UNKNOWN` | `unknown_age` | `allow`, `block` |
| `PIN_TOOLKIT_CONCURRENCY` | `concurrency` | usize |
| `PIN_TOOLKIT_ONLY` / `PIN_TOOLKIT_SKIP` | `only` / `skip` | comma list of known ids |
| `PIN_TOOLKIT_GITLAB_HOST` | `gitlab_host` | host |
| `PIN_TOOLKIT_CONFIG` | repo file path (meta, below `--config`) | path |
| `CI_SERVER_HOST`, `GITLAB_TOKEN`, `CI_JOB_TOKEN`, `HOME` | `GitlabEnv` / `ResolverEnv` (T-D12) | — |
| `DOCKERHUB_USER`, `DOCKERHUB_TOKEN`, `GITHUB_TOKEN` | `ResolverEnv.registry` → `Creds` (PR-M2; read from `Process.env`, never `std::env`) | — |
| any other `PIN_TOOLKIT_*` | none: one stderr warning each, exit unaffected (PR-M3) | — |
| `PIN_CI_LIVE`, `PIN_CI_ORACLE` | test gates (names kept, D29) | `1` |

### User texts (pinned literally; `{…}` filled)

- Fatal prefix `pin-toolkit: `; notice `pin-toolkit: skipped (plugin {p} not in this build)`.
- `plugin {p} not in this build` · `unknown plugin \`{p}\` (known: gitlab-ci, helm, github-actions)` · `unknown kind \`{k}\` for plugin \`{p}\``
- `{path}: \`{key}\` is a global-only key; it is not allowed in a repository config` · `{path}: missing \`version = 1\`` · `{path}: unsupported version {v} (expected 1)` · `{path}: {toml error}` · `config file not found: {path}`
- `{VAR}: expected a boolean (1/0, true/false, yes/no, on/off), got "{v}"` · `{VAR}: expected a number, got "{v}"` · `{VAR}: expected allow or block, got "{v}"` (fatal, exit 2) · warning `pin-toolkit: ignoring unknown environment variable {VAR}` (exit unaffected, PR-M3)
- `cannot read the current directory: {e}` (main.rs, exit 2, PR-L12) · `{path}: plugins.helm.aliases.{name}: a repository URL must not contain credentials` (PR-L14)
- skip reason `release age unknown ({host}; release_age.unknown = block)` (T9, PR-L10)
- `--kind expects <plugin>:<kind>=on|off, got "{v}"` · `--only/--skip cannot be combined with a plugin subcommand`
- `no plugin claims {file} (choose one: pin-toolkit <plugin> {file})` · `{file} is claimed by {p1} and {p2} (narrow their \`files\` globs, or choose one: pin-toolkit <plugin> {file})`
- `invalid glob \`{g}\` in plugins.{p}.files: {msg}` · `invalid GitLab host \`{h}\` in plugins.gitlab-ci.gitlab-hosts`
- Per-file error lines `pin-toolkit: {file}:{line}: {message}` / `pin-toolkit: {file}: {message}`; human summary: the unchanged total line, then (> 1 plugin ran) per plugin `{plugin}: pinned: …` (PR-L1); `follow-up: {text}`.

## Setup

- [ ] `git switch m5d-toolkit-binary` (exists, created from `main` @ c6a6ef3 with only the plan commits: 8f053e2 and the plan-review fix commit). Run the gate once → **passed 239** (pin-toolkit legs start at T13).

## Task T1 — L6: `Status` loses string equality (refactor)

**Files:** `crates/pin-core/src/report.rs`; tests `crates/pin-gitlab-ci/tests/{common/mod.rs (4), e2e.rs (13), e2e_components.rs (1), live.rs (1)}`.

- [ ] Evidence before: `INSTA_UPDATE=no cargo test --workspace --all-features > .tmp/t1-before.log 2>&1`.
- [ ] Delete `impl PartialEq<&str> for Status` and `impl PartialEq<str> for Status`. The compiler lists every site (19); rewrite each `status == "pinned"` / `assert_eq!(x.status, "updated")` to `Status::Pinned` / `Status::Updated` (allowed edit, constraint 2). Unit test `status_serde_display_and_str_eq` drops its str-eq asserts and is renamed `status_serde_and_display`.
- [ ] Evidence after: `diff <(grep '^test ' .tmp/t1-before.log | sed 's/str_eq/X/' | sort) <(grep '^test ' .tmp/t1-after.log | sed 's/status_serde_and_display/status_serde_display_and_X/' | sort)` empty; `grep -rn 'PartialEq<&str>\|PartialEq<str>' crates/pin-core/src` empty.
- [ ] Gate → 239. Commit: `refactor: M5d drop string equality on Status (review L6)`.

## Task T2 — L4 + L5: `RegError: Clone`, fallible client constructors

**Files:** `crates/pin-core/src/{oci/registry.rs, gitlab.rs, run.rs}`.
**Produces:** `RegError: Clone + PartialEq + Eq`; `RegistryClient::try_new(creds, base_urls) -> Result<Self, String>`, `GitLabClient::try_new(base_url, token, job_token) -> Result<Self, String>`; `new` = `try_new(..).expect(..)` kept (test convenience, moves behind `test-support` in T25).

- [ ] RED (unit, `registry.rs`): `reg_error_is_clone_and_eq`: `let e = RegError::Network("x".into()); assert_eq!(e.clone(), e); assert_ne!(e, RegError::Unresolved("x".into()))`. Fails to compile.
- [ ] GREEN: derives; `try_new` maps `reqwest::Client::builder().build()` errors to `"http client: {e}"`; `Ctx::oci()`/`Ctx::gitlab()` use `try_new` and cache the `Err(String)` (no `.expect` on any `Ctx` path).
- [ ] Check for L5 (no runtime test can make `reqwest` fail to build), non-test part of `run.rs` only (PR-L7: the test-only `GitLabClient::new(…)` at `run.rs:503` in `resolver_tests` would match): `sed '/^#\[cfg(all(test/,$d' crates/pin-core/src/run.rs | grep -nE '\.expect\(|::new\(' | grep -E 'RegistryClient|GitLabClient'` → empty (on 8f053e2 it prints `99: Ok(RegistryClient::new(` — the red). Planted: call `RegistryClient::new` in `Ctx::oci()` → the grep prints a line ☐; revert.
- [ ] Gate → 240. Commit: `refactor: M5d RegError is Clone; resolver clients built with try_new (review L4, L5)`.

## Task T3 — L2: edits of non-pinned findings are never written

**Files:** `crates/pin-core/src/run.rs` (`pin_file`), `crates/pin-core/tests/run_pin_file.rs`.

- [ ] RED: `non_pinned_edits_are_not_written`: Fake returns for target 0 `Finding { status: Skipped, edits: [Splice::replace(0, 1, "Z")], .. }` and for target 1 a `Pinned` finding with an edit → file has only target 1's edit; row 0 `skipped`, unchanged `old`. Run → FAIL (both edits written).
- [ ] GREEN: in `pin_file`, `edits` of a finding count only when `status ∈ {Pinned, Updated}` (before overlap marking, so a stray edit cannot mark a real pin as overlapping).
- [ ] Planted: remove the status filter → test FAILS ☐; revert.
- [ ] Gate → 241; snapshots unchanged. Commit: `fix: M5d write only edits of pinned/updated findings (review L2)`.

## Task T4 — L3: one `LineIndex` per file (`Source`)

**Files:** `crates/pin-core/src/{plugin.rs, edit.rs, run.rs (test Fake)}`, `crates/pin-core/tests/run_pin_file.rs` (Fake), `crates/pin-gitlab-ci/src/{lib.rs, policy.rs}`, `crates/pin-gitlab-ci/tests/plugin.rs` (4 calls).
**Produces:** `Source`, new `Plugin::resolve` and `trailing_comment(text, lines, line, token_end, orig)` signatures (Snippets).

- [ ] RED (unit, `edit.rs`): `trailing_comment_uses_the_given_index`: `let t = "a: x\r\nb: y\n"; let li = LineIndex::new(t); assert_eq!(trailing_comment(t, &li, 0, 4, "o"), Some(Splice::insert(4, "  # o")))` and line 1 → insertion at `t.len() - 1`. Fails to compile.
- [ ] GREEN: `Source::new` builds the index; blanket `plan` builds one `Source` per call and passes `&src` to every `resolve`; `policy::pinned` uses `src.lines`. Allowed edits (constraint 2): existing `trailing_comment_*` unit tests pass `&LineIndex::new(text)`; the two test `Fake::resolve` impls take `_src: &Source<'_>`; `tests/plugin.rs` direct calls pass `&Source::new(path, text)`.
- [ ] Evidence: `grep -n 'LineIndex::new' crates/pin-core/src/edit.rs crates/pin-gitlab-ci/src/policy.rs` → only in tests.
- [ ] Planted: build `LineIndex::new(src.text)` inside `policy::pinned` instead of using `src.lines` → `grep` evidence above non-empty ☐ (no behavioural test can see it; the grep is the check); revert.
- [ ] Gate → 242; snapshots unchanged. Commit: `refactor: M5d one LineIndex per file via plugin::Source (review L3)`.

## Task T5 — L10 kind constants + N5 no `unreachable!` in policy (refactor)

**Files:** `crates/pin-gitlab-ci/src/{lib.rs, policy.rs, discover.rs}`.

- [ ] Evidence before: test list as in T1.
- [ ] `pub const IMAGE/COMPONENT/INCLUDE` used by `KINDS`, `target_kind`, `resolve`, `policy`, and the **kind** arguments in `discover.rs` (PR-L13): `yaml_target(text, s, IMAGE, …)` (:211), `yaml_target(text, s, COMPONENT, …)` (:213), `unsupported(text, git_ref, INCLUDE)` (:230), `kind == COMPONENT` (:272). YAML **key** comparisons stay literal (`head != "image" && head != "services"` :297, `k == "include"` :317, `k == "component"` :324): they name YAML keys, and a kind rename must not change what is parsed; `GitlabCi::resolve` destructures each `PinTarget` arm and passes the fields (`unsupported(src, t, kind, reason)`, `resolve_component(ctx, src, t, parts.as_ref())`, `resolve_include(ctx, src, t, project)`).
- [ ] Evidence after: identical test list; `grep -nE '"(image|component|include)"' crates/pin-gitlab-ci/src/{lib,policy}.rs` → only the const definitions; `sed '/^#\[cfg(test)\]/,$d' crates/pin-gitlab-ci/src/discover.rs | grep -nE '(yaml_target|unsupported)\([^)]*"(image|component|include)"|kind == "'` → empty (on 8f053e2 it prints :211, :213, :230, :272 — the red); `grep -n 'unreachable!' crates/pin-gitlab-ci/src/policy.rs` → empty.
- [ ] Gate → 242. Commit: `refactor: M5d gitlab-ci kind constants, no unreachable! in policy (review L10, N5)`.

## Task T6 — per-run memo in `Ctx` (§6.7, D19)

**Files:** `crates/pin-core/src/run.rs`, new `crates/pin-core/tests/memo.rs` (`[[test]] required-features = ["oci", "gitlab"]`).
**Consumes:** T2 `RegError: Clone`. **Produces:** `Memo` behind `oci_digest`, `oci_tags`, `oci_tag_times`, `gitlab_resolve`.

- [ ] RED (`memo.rs`, wiremock registry + GitLab, presets via `with_oci`/`with_gitlab`, responses `set_delay(100ms)` so calls overlap): `concurrent_digest_lookups_are_one_request` (`join!(ctx.oci_digest(&r), ctx.oci_digest(&r))` → both `Ok(d)`, mock `.expect(1)`) · `failed_lookups_are_cached` (404; two concurrent + one later call → three equal `Err`, `.expect(1)`) · `concurrent_gitlab_lookups_are_one_request` (same `(host, project, ref)` ×2 → `.expect(1)`; different ref → one more) · `distinct_keys_are_not_shared` (`alpine:3.20` vs `alpine:3.21` → two requests; characterisation, may pass before GREEN). Run → the first three FAIL.
- [ ] GREEN: `Memo` per Snippets; keys per T-D15; each entry point = `self.memo.x.get(key, || async { …current body… }).await`.
- [ ] Planted: key the digest memo by `(host, repo)` only → `distinct_keys_are_not_shared` FAILS ☐; drop the memo on the error path → `failed_lookups_are_cached` FAILS ☐. Revert each.
- [ ] Gate → 246. Commit: `feat: M5d per-run resolver memo with coalescing, errors cached (§6.7)`.

## Task T7 — `GitlabEnv`: D31 primary, D37 token binding, multi-host clients

**Files:** `crates/pin-core/{Cargo.toml (feature `test-support = []`, `[[test]] gitlab_hosts` required-features `["gitlab", "test-support"]`), src/run.rs, src/gitlab.rs, src/oci/registry.rs}`, new `crates/pin-core/tests/gitlab_hosts.rs`, `crates/pin-core/tests/live_gitlab.rs` (3 calls), `crates/pin-gitlab-ci/src/{driver.rs, main.rs}`, `crates/pin-gitlab-ci/tests/{common/mod.rs, plugin.rs}` (`ResolverEnv` literals), `crates/pin-gitlab-ci/Cargo.toml` (dev-dep pin-core `test-support`).
**Produces:** `GitlabAuth`, `GitlabEnv`, `RegistryEnv`, `REGISTRY_VARS`, `ResolverEnv { home, gitlab, registry, base_urls }`, `ResolverEnv::{from_vars, creds, from_process}`, `GitLabHosts::new`, redacting `Debug` (T-D29); deletes `GitLabClient::from_env` and `ResolverEnv.gitlab_host`.

- [ ] RED (unit, `run.rs`): `auth_for_binds_tokens` — table over `(primary, ci_server_host, gitlab_token, ci_job_token, host) → GitlabAuth`: `(gitlab.com, -, pt, -, gitlab.com) → Private(pt)` · `(gitlab.com, -, pt, -, gitlab.example.com) → Anonymous` · `(gitlab.example.com, gitlab.example.com, -, jt, gitlab.example.com) → Job(jt)` · `(gitlab.com, gitlab.example.com, -, jt, gitlab.com) → Anonymous` · `(gitlab.com, -, -, jt, gitlab.com) → Anonymous` (today's leak) · `(h, h, pt, jt, h) → Private(pt)`; plus `from_vars` primary: explicit > `CI_SERVER_HOST` > `gitlab.com`, empty strings = unset · `registry_creds_from_vars` (PR-M2; `resolver_tests`): `ResolverEnv::from_vars(&{GITHUB_TOKEN: "x"}, None, ∅).creds().for_host("ghcr.io") == Some(&HostCreds::Bearer { token: "x" })`; `{DOCKERHUB_USER: u, DOCKERHUB_TOKEN: t}` → `docker.io` Basic; empty map → `for_host("ghcr.io") == None` (no `HOME` in the map ⇒ no docker config read: deterministic on any machine) · `debug_redacts_tokens` (PR-M5, P-redact; `resolver_tests`): with every token = `"s3cr3t"`, `format!("{:?}", x)` contains no `s3cr3t` for x ∈ {`GitlabEnv`, `GitlabAuth::Private`, `GitlabAuth::Job`, `ResolverEnv` (incl. `registry`), `GitLabClient::try_new(…, Some("s3cr3t".into()), true)`, `HostCreds::Basic`, `HostCreds::Bearer`}, and does contain `<redacted>`. Fails to compile.
- [ ] RED (`gitlab_hosts.rs`, two MockServers A = `gitlab.example.com`, B = `gitlab.com` via `base_urls.gitlab`): `job_token_only_reaches_ci_server_host` (env: primary gitlab.com, hosts {gitlab.example.com}, `CI_JOB_TOKEN=jt`, `CI_SERVER_HOST=gitlab.example.com`; resolve on both; A: `header("JOB-TOKEN", "jt")` `.expect(1)`; B: `header_exists("JOB-TOKEN")` → 500 `.expect(0)`) · `job_token_without_ci_server_host_is_never_sent` (B `.expect(0)` on `JOB-TOKEN`) · `private_token_only_reaches_primary` (`GITLAB_TOKEN=pt`, primary A; B: `header_exists("PRIVATE-TOKEN")` `.expect(0)`). Response shape: copy from `tests/gitlab_client.rs`.
- [ ] GREEN: types per Snippets; `Ctx::gitlab()` builds one client per `resolve_set()` host with `auth_for(h).into_parts()`; `with_gitlab(primary, client)` also sets `env.gitlab.primary`. `Ctx::oci()` uses `self.env.creds()` (no `Creds::load_from`, PR-M2; `Creds::load_from_env` → `pub(crate)`); hand-written `Debug` per T-D29 (`GitlabEnv`, `GitlabAuth`, `RegistryEnv`, `GitLabClient`, `HostCreds`). Delete `GitLabClient::from_env` and its env-mutating unit test `env_precedence` (allowed edit: subject deleted; replaced by `auth_for_binds_tokens`, no `set_var`). Interim pin-ci: `driver::gitlab_from_env(host)` = `let (token, job) = ResolverEnv::from_process(host).gitlab.auth_for(host).into_parts(); GitLabClient::try_new(format!("https://{host}/api/v4"), token, job).expect("reqwest client builds")` (PR-L12: `try_new` takes the parts, not the tuple); `main.rs` uses `ResolverEnv::from_process(&host)`. Allowed edits (constraint 2, PR-M1): `ResolverEnv { gitlab_host: …, ..Default::default() }` → `ResolverEnv::default()` at `pin-gitlab-ci/tests/common/mod.rs:434-437`, `pin-gitlab-ci/tests/plugin.rs:16-21`, `pin-core/src/run.rs:420-425`; `live_gitlab.rs:101,122,147` `GitLabClient::from_env(GITLAB_API.to_string())` → `{ let (t, j) = ResolverEnv::from_process("gitlab.com").gitlab.auth_for("gitlab.com").into_parts(); GitLabClient::try_new(GITLAB_API.into(), t, j).unwrap() }`. `PinOptions.gitlab_host` fields in e2e/e2e_components/live/oracle are untouched.
- [ ] Planted: in `auth_for`, compare the job token against `primary` instead of `ci_server_host` → `job_token_only_reaches_ci_server_host` FAILS (A gets none, or B gets one) ☐; drop the `CI_SERVER_HOST` requirement → `job_token_without_ci_server_host_is_never_sent` FAILS ☐; `creds()` ignores the map (`|_| None`) → `registry_creds_from_vars` FAILS ☐; derive `Debug` on `GitlabEnv` again → `debug_redacts_tokens` FAILS ☐. Revert each. Evidence: `grep -n 'Creds::load_from(' crates/pin-core/src/run.rs` → empty.
- [ ] Gate (+ matrix leg `--features oci,gitlab,test-support`) → 251 (−1 deleted unit test, +6: `auth_for_binds_tokens`, `registry_creds_from_vars`, `debug_redacts_tokens`, 3 wiremock). Commit: `fix: M5d CI_JOB_TOKEN only to CI_SERVER_HOST, GITLAB_TOKEN only to the primary host (D31, D37)`.

## Task T8 — L9: one GitLab host set (`GitlabCi` unit struct)

**Files:** `crates/pin-core/src/run.rs` (`Ctx::gitlab_enabled`), `crates/pin-gitlab-ci/src/{lib.rs, policy.rs, driver.rs}`, `crates/pin-gitlab-ci/tests/plugin.rs`.

- [ ] RED (`plugin.rs`): `component_host_enabled_by_env`: `Ctx` with `GitlabEnv { primary: "gitlab.com", hosts: {"gitlab.example.com"}, .. }` preset to a wiremock for gitlab.example.com; `component: gitlab.example.com/g/p/c@main` → `pinned`; `component: other.example/g/p/c@main` → `skipped (host not enabled: other.example)`, no request. Fails to compile (`GitlabCi` has a field).
- [ ] GREEN: `GitlabCi` unit struct (`Default`); `resolve_component` gates on `ctx.gitlab_enabled(host)`; driver builds `GitlabCi`. Allowed edit: `tests/plugin.rs:67` (`p.gitlab_hosts == ["gitlab.com"]`) becomes `let ctx = Ctx::new(Default::default(), 0, ResolverEnv::default()); assert!(ctx.gitlab_enabled("gitlab.com") && !ctx.gitlab_enabled("gitlab.example.com"))`; every `GitlabCi { gitlab_hosts: … }` literal → `GitlabCi`.
- [ ] Planted: `gitlab_enabled` returns `true` → test FAILS (request to other.example) ☐; revert.
- [ ] Gate → 252; snapshots unchanged. Commit: `refactor: M5d GitLab hosts from one GitlabEnv; GitlabCi is a unit struct (review L9)`.

## Task T9 — `release_age.unknown = block` (§6.8)

**Files:** `crates/pin-core/src/{run.rs, oci/decide.rs}`, `crates/pin-core/tests/oci_decide.rs`; the one full literal in `crates/pin-gitlab-ci/src/driver.rs:101` (PR-M1).

- [ ] RED (`oci_decide.rs`): `unknown_age_block_skips_non_hub_updates`: ghcr.io `org/img:v1` with tags `[v1, v2]`, `UpdatePolicy { enabled: true, min_age_days: 7, unknown_age: Block, .. }` → `ImageDecision` skip with reason exactly `release age unknown (ghcr.io; release_age.unknown = block)` (PR-L10), no Hub request (Hub mock `.expect(0)`), no tag-time request · `unknown_age_allow_keeps_todays_update` (same with `Allow` → update to `v2`; characterisation). Fails to compile.
- [ ] GREEN: `AgePolicy`; guard condition per T-D18; non-hub + Block → no timestamp request, skip with the `release age unknown (…)` reason (one new branch in `decide_image`). Allowed edit (PR-M1): the **one** full literal `driver.rs:101` gains `unknown_age: AgePolicy::Allow`; `oci_decide.rs:58` and `common/mod.rs:430` use `..Default::default()` and stay; `run.rs` has no literal.
- [ ] Planted: drop `|| unknown_age == Block` → first test FAILS ☐; revert.
- [ ] Gate → 254. Commit: `feat: M5d release_age.unknown = block guards non-docker.io updates (§6.8)`.

## Task T10 — report v1 model + `Report::exit_code`

**Files:** `crates/pin-core/src/{report.rs, run.rs (rows carry follow_up)}`; `Row` literals in `crates/pin-gitlab-ci/src/driver.rs` (unit test) and `crates/pin-gitlab-ci/tests/fixture_properties.rs` (PR-M1).
**Produces:** `Row.follow_up`, `ErrorEntry`, `PluginReport`, `Summary: Default + PartialEq + Eq` (PR-L12), `Report::{SCHEMA_VERSION, assemble, exit_code}` (JSON unchanged: new fields `#[serde(skip)]`, T-D19).

- [ ] RED (unit): `exit_code_rules` — ∀ (statuses ⊆ Status, check ∈ {F, T}, errors ∈ {0, 1}): code per P-exit, expectation computed from the inputs by a reference function written in the test · `plugin_summaries_partition_total` (rows of `a` and `b`, `ran = [a, b, c]`: `plugins[c]` exists with zero summary, P-sum) · `follow_ups_dedup_first_seen` (`[x, y, x]` → `[x, y]`). Fails to compile.
- [ ] GREEN: `assemble`; `new(rows) = assemble(rows, &[], vec![])` with `plugins` derived from row ids; `pin_file` copies `Finding.follow_up` into `Row.follow_up`. Allowed edits (constraint 2, PR-M1): `follow_up: None` added to the `Row { … }` literals at `report.rs:148,159,170,215`, `driver.rs:178`, `fixture_properties.rs:18`.
- [ ] Planted: `exit_code` ignores `check` → `exit_code_rules` FAILS ☐; revert.
- [ ] Gate → 257; `git status` no `*.snap` change (JSON byte-identical). Commit: `feat: M5d report v1 model (plugins, errors, follow-ups) and exit codes`.

## Task T11 — `Selection`: kinds + `ignore` patterns

**Files:** `crates/pin-core/src/{plugin.rs, run.rs (incl. `resolver_tests`)}`, `crates/pin-core/tests/run_pin_file.rs`, `crates/pin-gitlab-ci/src/lib.rs`, `crates/pin-gitlab-ci/tests/{plugin.rs, common/mod.rs}`, `crates/pin-gitlab-ci/src/driver.rs`.

- [ ] RED: unit `wildcard_matches_whole_key` table: (`registry.example.com/legacy/*`, `registry.example.com/legacy/a/b:1` ✓), (`*/legacy/*`, `x/legacy/y` ✓), (`alpine:3.??`, `alpine:3.20` ✓, `alpine:3.2` ✗), (`alpine`, `alpine:3` ✗), (`*`, empty key ✓), (`ä?`, `äb` ✓) · `ignored_targets_have_no_row_and_no_resolve` (`run_pin_file.rs`: Fake `ignore_key(t) = Some(format!("r{t}"))`, pattern `r1`, resolve counter → target 1 has no row, counter == targets − 1) · `gitlab_ci_ignore_keys` (`plugin.rs`: image `registry.example.com/legacy/x:1` and include `{project: g/p, ref: main}` with patterns `registry.example.com/legacy/*`, `g/p@main` → both gone; others unchanged, P-ignore). Fails to compile.
- [ ] GREEN: Snippets. Allowed edits: `run_pin_file.rs` helper `on()` returns `Selection` (`KindSet::defaults(&KINDS).into()`); `tests/plugin.rs:171,282` and `common/mod.rs:442` pass `&sel`; the `pin_file(fake, ctx, &p, &KindSet::defaults(&KINDS), &o)` call in `run.rs:437` (`resolver_tests`, PR-M1) passes `&KindSet::defaults(&KINDS).into()`; driver passes `KindSet::defaults(&KINDS).into()`.
- [ ] Planted: match prefix instead of whole key → `wildcard_matches_whole_key` FAILS (`alpine` vs `alpine:3`) ☐; filter after resolving → counter assertion FAILS ☐. Revert each.
- [ ] Gate → 260. Commit: `feat: M5d Selection with ignore patterns; ignored targets have no row (§7.1)`.

## Task T12 — `run::{Route, run}`: one pass, keeps going after file errors (§6.3)

**Files:** `crates/pin-core/src/run.rs`, new `crates/pin-core/tests/run_routes.rs` (reuses the `run_pin_file.rs` Fake pattern; no features).

- [ ] RED: `run_continues_after_file_error` (routes: missing `a.yml`, good `b.yml` → rows only of b with `file == "b-display"`, b written, `plugins["fake"].errors == [ErrorEntry { file: "a-display", line: None, message: <read msg> }]`, `exit_code(false) == 2`) · `run_parse_error_has_line` (Fake scan `Err(ScanError { line: Some(3), msg: "invalid YAML: x" })` → entry line `Some(3)`, message `invalid YAML: x`, file untouched) · `run_rows_use_display_path` · `run_collects_follow_ups` (two files emit the same follow-up → one). Fails to compile.
- [ ] GREEN: `Route`, `run`, `FileError::entry` per Snippets (message = the variant's `msg`, without the path).
- [ ] Planted: `return` on the first error → `run_continues_after_file_error` FAILS ☐; revert.
- [ ] Gate → 264. Commit: `feat: M5d core run over routes; per-file errors recorded, later files continue (§6.3)`.
- [ ] **Review checkpoint 1** (Open for main 12): request a review of T1–T12 (pin-core/pin-gitlab-ci carry-overs + core run) before T13.

## Task T13 — `crates/pin-toolkit` scaffold, static `KNOWN`, `plugins` subcommand

**Files:** root `Cargo.toml` (workspace deps `toml`, `ignore`, `globset`, `pin-gitlab-ci`), `Cargo.lock`, new `crates/pin-toolkit/{Cargo.toml, src/{lib.rs, main.rs, known.rs, cli.rs}, tests/cli.rs}`.

- [ ] RED: unit `compiled_plugins_are_known` (P-known) · `tests/cli.rs` `plugins_lists_compiled_and_known` (`execute(["pin-toolkit", "plugins"])` in a `.tmp/` repo with `.git/` → code 0; stdout = `gitlab-ci: enabled\n` + for each `k ∈ pin_gitlab_ci::KINDS`: `  {k.id}: on — {k.help}\n` + `helm: not in this build\ngithub-actions: not in this build\n`; `#[cfg(feature = "gitlab-ci")]`) · `#[cfg(not(feature = "gitlab-ci"))] plugins_without_gitlab_ci` (`gitlab-ci: not in this build` first line). Crate missing → FAIL.
- [ ] GREEN: crate per Snippets (no config yet: enable state = compiled); `main.rs` thin per Snippets (PR-L12: `from_os` error → exit 2 with `cannot read the current directory: {e}`; `write_all` with errors ignored instead of `print!`, so EPIPE does not panic — coverage gap 7); `Cli`/`Mode` parse with only `plugins` acted on (other modes: empty output, code 0 until T21). Commit body lists new `Cargo.lock` packages (constraint 6).
- [ ] Planted: remove `"image"` from `KNOWN[gitlab-ci].kinds` → `compiled_plugins_are_known` FAILS ☐; revert.
- [ ] Gate incl. new pin-toolkit legs → 266 (nodefault leg 2: `compiled_plugins_are_known`, `plugins_without_gitlab_ci`; PR-L3). Commit: `feat: M5d pin-toolkit crate: multi-call binary skeleton, known plugins, plugins subcommand`.

## Task T14 — `Layer` and `Effective::fold` (the one precedence function)

**Files:** new `crates/pin-toolkit/src/config/mod.rs`.

- [ ] RED (unit): `fold_scalars_last_wins` — for each scalar field, ∀ presence pattern ∈ {0,1}⁴ over L = [defaults, repo, env, cli] with distinct values per layer: result = value of the highest present layer else the default (P-prec; expectation derived from the pattern) · `fold_lists_and_kinds` (`files` and `gitlab_hosts` replace, `ignore` concatenates in layer order, `kinds` per key, `check ⇒ dry_run`, concurrency default 8). Fails to compile.
- [ ] GREEN: `Layer`, `PluginLayer`, `Effective::fold` per Snippets/T-D5.
- [ ] Planted: iterate layers in reverse → `fold_scalars_last_wins` FAILS ☐; make `ignore` replace → `fold_lists_and_kinds` FAILS ☐. Revert each.
- [ ] Gate → 268. Commit: `feat: M5d config layers folded in one place (defaults < repo < env < CLI)`.

## Task T15 — repo `.pin-toolkit.toml` parser

**Files:** new `crates/pin-toolkit/src/config/repo.rs`.

- [ ] RED (unit): `repo_full_schema_parses` (the Snippets schema text verbatim → expected `Layer`, helm options kept in `options`) · `repo_rejects` table (text → `Fatal` message, User texts): no `version`; `version = 2`; `[update] enable = true` (message contains ``unknown field `enable` ``); `[plugins.gitlab]` (unknown plugin); `kinds = { images = true }`; `[release_age] unknown = "maybe"`; `gitlab-hosts = ["https://x"]`; `gitlab-host = "evil.example"` at top level and under `[plugins.gitlab-ci]` (unknown field: a repo cannot set the primary host, security checklist); `[plugins.helm] aliases = { s = "https://u:p@charts.example" }` and `aliases = { s = "https://tok@charts.example" }` → `{path}: plugins.helm.aliases.s: a repository URL must not contain credentials` (PR-L14; `https://charts.example/a@b` is accepted: `@` after the host) · `repo_global_only_keys` (`[hosts]` table and `concurrency = 4` → `{path}: \`hosts\` is a global-only key; it is not allowed in a repository config`, same for `concurrency`; checked **before** the typed parse so the message is specific). Fails to compile.
- [ ] GREEN: `toml::Table` pre-pass → typed `RepoFile` (`deny_unknown_fields`, `rename_all = "kebab-case"` on plugin tables, `release_age`/`min_days` snake per spec) → `Layer`; helm `aliases` values: strings whose authority (between `://` and the next `/`) contains no `@` (PR-L14).
- [ ] Planted: skip the pre-pass → `repo_global_only_keys` FAILS (generic unknown-field text) ☐; drop the alias userinfo check → the `aliases` rows of `repo_rejects` FAIL ☐. Revert each.
- [ ] Gate → 271. Commit: `feat: M5d repo config parser: version 1, unknown keys and global-only keys rejected`.

## Task T16 — env layer

**Files:** new `crates/pin-toolkit/src/config/env.rs`; `crates/pin-toolkit/src/lib.rs` (`execute` prints the warnings).

- [ ] RED (unit): `env_vars_map_to_fields` (table from Snippets: each var with a valid value sets exactly its field; `""` = unset; bool spellings case-insensitive) · `env_rejects_bad_values` (each bad value of a known variable → its User text, `Err(Fatal)`; non-`PIN_TOOLKIT_` vars ignored) · `env_unknown_vars_warn` (PR-M3: `{PIN_TOOLKIT_FOO: 1, PIN_TOOLKIT_VERSION: 0.2, PIN_TOOLKIT_UPDATE: 1, CI_JOB_TOKEN: x}` → `layer` is `Ok` with only `update` set; `unknown_vars` = `[PIN_TOOLKIT_FOO, PIN_TOOLKIT_VERSION]` (sorted); `PIN_TOOLKIT_CONFIG` is known, never reported) · `env_config_path` (`PIN_TOOLKIT_CONFIG` → `config_path`, not a `Layer` field). Fails to compile.
- [ ] GREEN: `env::layer`, `env::unknown_vars`, `env::config_path`; `execute` writes `pin-toolkit: ignoring unknown environment variable {VAR}\n` per entry to stderr right after a successful `parse` (end to end: T22 row 16).
- [ ] Planted: `unknown_vars` returns `[]` → `env_unknown_vars_warn` FAILS ☐; make `layer` return `Err` for an unknown name (the old exit-2 rule) → `env_unknown_vars_warn` FAILS ☐. Revert each.
- [ ] Gate → 275. Commit: `feat: M5d PIN_TOOLKIT_* environment layer`.

## Task T17 — CLI layer, subcommands, argv[0] dispatch, usage errors

**Files:** `crates/pin-toolkit/src/{cli.rs, lib.rs}`, `crates/pin-toolkit/tests/cli.rs`.

- [ ] RED (unit): `cli_flags_map_to_layer` (every §6.6 option → its `Layer` field; absent → `None`) · `paths_and_subcommands_parse` (`[a.yml]` → All [a.yml]; `[gitlab-ci, a.yml]` → Plugin(gitlab-ci) [a.yml]; `[--check, gitlab-ci, --json, a.yml]` → Plugin(gitlab-ci) [a.yml] with both flags; `[--dry-run, gitlab-ci]` → Plugin(gitlab-ci) [] with `dry_run` (PR-H1); `[gitlab-ci, a.yml, --check]` → Plugin(gitlab-ci) [a.yml] with `check`; `[a.yml, gitlab-ci, b.yml]` → All [a.yml, gitlab-ci, b.yml] (a subcommand is recognised only before the first path; documented in the README, PR-H1 alternative); `[plugins]` → Plugins; `[plugins, x]` → usage error; `[./plugins]` → All [./plugins]; `--kind gitlab-ci:image=off` → `plugins[gitlab-ci].kinds[image] = false`; `--kind gitlab-ci:image` → `--kind expects …`; `--kind helm:nope=on` → unknown kind) · `argv0_aliases` (`pin-ci` → +`gitlab-ci`; `/x/pin-gitlab-ci` → +`gitlab-ci`; `pin-helm` → +`helm`; `pin-toolkit`, `pin-foo`, `pin-ci.sh` → unchanged; `pin-ci.exe` → +`gitlab-ci`) · `tests/cli.rs` `usage_error_exits_2` (`execute([pin-toolkit, --bogus])` → 2, stderr starts with `error:`, stdout empty; `--help` → 0, stdout non-empty). Run → FAIL.
- [ ] GREEN: Snippets (no `args_conflicts_with_subcommands`, PR-H1); `parse` uses `try_parse_from`, `DisplayHelp`/`DisplayVersion` → 0.
- [ ] Planted: drop the `KNOWN` membership check in `dispatch_argv0` → the `pin-foo` row FAILS ☐; strip `.exe` nowhere → the `pin-ci.exe` row FAILS ☐; add `args_conflicts_with_subcommands = true` → the `[--check, gitlab-ci, --json, a.yml]` row FAILS (parses as All [gitlab-ci, a.yml]) ☐. Revert each.
- [ ] Gate → 279. Commit: `feat: M5d CLI: options, plugin subcommands, pin-<plugin>/pin-ci argv[0] dispatch`.

## Task T18 — config loading: file discovery, precedence end to end, `[hosts]` → exit 2

**Files:** `crates/pin-toolkit/src/{config/mod.rs (load), lib.rs}`, new `crates/pin-toolkit/tests/config_layers.rs`.

- [ ] RED (`config_layers.rs`, `.tmp/` repos with `.git/`): `precedence_repo_env_cli` (repo `[update] enabled = true`; env `PIN_TOOLKIT_UPDATE=0`; CLI `--update` → `load(..).update.enabled` = true; without CLI → false; without env → true) · `repo_hosts_exits_2` (`execute` in a repo whose `.pin-toolkit.toml` has `version = 1` + `[hosts]` → code 2, stderr = `pin-toolkit: {display path}: \`hosts\` is a global-only key; it is not allowed in a repository config\n`, stdout empty, `.gitlab-ci.yml` bytes unchanged) · `config_flag_and_env_path` (`--config other.toml` wins over `PIN_TOOLKIT_CONFIG` wins over root file; missing → `config file not found: …`, 2) · `no_config_skips_repo_file` (invalid root file + `--no-config` → loads defaults). Run → FAIL.
- [ ] GREEN: `load` per Snippets; `execute` calls `load` before acting on any mode (so `plugins` also fails on a bad config).
- [ ] Planted (spec §11): put `[hosts]` in a fixture repo file with the pre-pass disabled → `repo_hosts_exits_2` FAILS (message) ☐; swap env and CLI order in `load` → `precedence_repo_env_cli` FAILS ☐. Revert each.
- [ ] Gate → 283. Commit: `feat: M5d load repo config (git root, --config, PIN_TOOLKIT_CONFIG, --no-config); [hosts] in a repo config exits 2`.

## Task T19 — enablement, `--only`/`--skip`, "not in this build" (O1)

**Files:** new `crates/pin-toolkit/src/enable.rs`, `crates/pin-toolkit/src/lib.rs` (`plugins` listing uses `enable`), `crates/pin-toolkit/tests/config_layers.rs`.

- [ ] RED: `run_false_disables_but_subcommand_runs` (repo `[plugins.gitlab-ci] run = false` → `plugins` prints `gitlab-ci: disabled`; `enable(Mode::Plugin("gitlab-ci"))` enables it) · `only_and_subcommand_rules` (`--only helm` → `plugin helm not in this build`, 2; `--only nope` → unknown plugin, 2; `--skip helm` ok; `gitlab-ci --skip x` → `--only/--skip cannot be combined …`, 2; `execute([pin-toolkit, helm])` → 2; PR-L6 (env exempt): env `PIN_TOOLKIT_SKIP=gitlab-ci` + `[gitlab-ci, --dry-run]` → gitlab-ci enabled, no conflict error; env `PIN_TOOLKIT_ONLY=helm` + argv[0] `pin-ci` → gitlab-ci enabled, no `not in this build` error; the same env in All mode → `plugin helm not in this build`, 2) · `not_built_plugin_in_config_is_a_notice` (repo `[plugins.helm] run = true` → stderr contains the notice exactly once, code unaffected; `run = false` → no notice) · `#[cfg(not(feature = "gitlab-ci"))] gitlab_ci_subcommand_not_in_build` (→ 2, `plugin gitlab-ci not in this build`). Run → FAIL.
- [ ] GREEN: `enable` per Snippets (Plugin mode never consults `eff.only`/`eff.skip`); `execute`: Plugin mode ∧ CLI `--only`/`--skip` (on the parsed `Cli`, not on `Effective`) → Fatal (T-D28); kind overrides applied to compiled kinds only; invalid glob → `Fatal`.
- [ ] Planted: treat a config table for an unbuilt plugin as exit 2 → `not_built_plugin_in_config_is_a_notice` FAILS ☐; check the conflict on `Effective` instead of the CLI → the `PIN_TOOLKIT_SKIP` row of `only_and_subcommand_rules` FAILS ☐. Revert each.
- [ ] Gate → 286 (nodefault leg ≥ previous, PR-L3). Commit: `feat: M5d plugin enablement: run, --only/--skip, not-in-this-build handling (O1)`.

## Task T20 — walk and routing

**Files:** new `crates/pin-toolkit/src/walk.rs` (unit tests with `.tmp/` trees).

- [ ] RED (unit): `root_walk_respects_gitignore_and_hidden` (tree: `.git/x.gitlab-ci.yml`, `.gitignore` = `ignored/`, `.gitlab-ci.yml`, `ci/.gitlab-ci.yml`, `.gitlab/ci/a.yml`, `ignored/x.gitlab-ci.yml`, `docs/readme.yml`; gitlab-ci default globs → routes exactly `[.gitlab-ci.yml, .gitlab/ci/a.yml, ci/.gitlab-ci.yml]`) · `display_relative_to_cwd` (cwd = `repo/ci` → displays `../.gitlab-ci.yml`, `../.gitlab/ci/a.yml`, `.gitlab-ci.yml`; I/O paths exist) · `explicit_file_claim_rules` (all mode: `docs/readme.yml` → `no plugin claims docs/readme.yml (choose one: pin-toolkit <plugin> docs/readme.yml)`; two fake ids whose globs both match → claimed-by message naming both; `forced = gitlab-ci` → claimed; same file passed twice → one route) · `explicit_dir_is_walked` (`ci/` → only `ci/.gitlab-ci.yml`, globs relative to the git root) · `no_git_root_falls_back_to_cwd` (`git_root_with("/a/b", |_| false)` → `None`, and `inputs` with root = cwd walks cwd; `git_root_with("/a/b", |p| p == "/a/.git")` → `Some("/a")`; a real `.tmp/` tree cannot prove "no ancestor has `.git`" because this repo is one) · `explicit_parent_path_from_subdir` (PR-M9: cwd = `repo/ci`, all mode, paths `[../.gitlab-ci.yml]` → one route to gitlab-ci, display `../.gitlab-ci.yml`, `rel == Some(".gitlab-ci.yml")`; a file outside the root, `<tmp>/outside.gitlab-ci.yml` passed as `../../outside.gitlab-ci.yml` → `rel == None`, all mode → `no plugin claims ../../outside.gitlab-ci.yml (…)`, `forced = gitlab-ci` → claimed) · `same_file_two_spellings_one_route` (PR-M9: `[a.gitlab-ci.yml, ./a.gitlab-ci.yml, ci/../a.gitlab-ci.yml]` → exactly one route, display `a.gitlab-ci.yml`) · `walked_file_claimed_twice_is_a_file_error` (PR-L11: two fake ids `p1`, `p2` whose globs both match the walked `x.yml`, plus `y.yml` matched only by `p1` → `Ok(Routed)` with routes `[y.yml → p1]` and conflicts `[(p1, ErrorEntry { file: "x.yml", line: None, message: "x.yml is claimed by p1 and p2 (…)" })]`; the same file passed explicitly → `Err(Fatal)`). Fails to compile.
- [ ] GREEN: `normalize`, `git_root`, `inputs`, `route` per Snippets/T-D23; walked 2+ claims → `Routed.conflicts` (T-D10).
- [ ] Planted: `hidden(true)` (default) → `root_walk…` FAILS (`.gitlab-ci.yml` missing) ☐; match globs against the display path → `display_relative_to_cwd` FAILS ☐; skip `normalize` (plain `cwd.join`) → `explicit_parent_path_from_subdir` and `same_file_two_spellings_one_route` FAIL ☐; walked 2+ claims → `Fatal` → `walked_file_claimed_twice_is_a_file_error` FAILS ☐. Revert each.
- [ ] Gate → 294 (+8). Commit: `feat: M5d walk git root with ignore, route files by plugin globs (§6.2, §6.3)`.

## Task T21 — wire the run; toolkit snapshots take over the 38 human/JSON snapshots

**Files:** `crates/pin-toolkit/src/lib.rs` (`execute` per Snippets), new `crates/pin-toolkit/tests/{fixture_snapshots.rs, run.rs}`, `git mv crates/pin-gitlab-ci/tests/snapshots/fixture_snapshots__*__{human,json}.snap crates/pin-toolkit/tests/snapshots/`, `crates/pin-gitlab-ci/tests/{fixture_snapshots.rs, common/mod.rs}`.

- [ ] RED (`run.rs`, fakes from `common` via `#[path = "../../pin-gitlab-ci/tests/common/mod.rs"] mod common;`, `tune` maps `docker.io`/`ghcr.io` → fake registry, `gitlab.com` → fake GitLab): `pins_a_repo_end_to_end` (repo `.gitlab-ci.yml` = `image: alpine:3.20\n` → code 0, file bytes exactly `format!("image: alpine:3.20@sha256:{}\n", common::fake_hex("library/alpine:3.20", 64))` — C1 pins inline with **no** trailing comment (PR-M8; cf. `fixture_snapshots__ci-simple__pin__file.snap`), stdout = one row + summary) · `same_image_in_two_files_is_one_lookup` (`.gitlab-ci.yml` + `ci/x.gitlab-ci.yml` both `alpine:3.20` → `received_requests()` has exactly one manifest HEAD for `library/alpine/manifests/3.20`, P-memo).
- [ ] RED (`fixture_snapshots.rs`): `toolkit_snapshots_over_fixtures`: ∀ fixture × mode: two scratch copies (each dir gets `.git/`); run `execute([pin-toolkit, gitlab-ci, --concurrency, 4, (--update), {name}])` and the same with `--json` in `cwd = scratch dir`; `assert_snapshot!("{stem}__{mode}__human", redact(&out.stdout))`, `…__json` likewise; P-same-bytes: both written files = the bytes `common::pin_once` writes on a third copy. Run → FAIL (snapshots not yet moved → `.snap.new`; delete them). Every `execute` in T21–T23 passes an explicit `Process.env` map without `HOME`/registry tokens unless the test sets them, so no machine credential reaches a fake (PR-M2).
- [ ] GREEN: `execute`; `git mv` the 38 files (`git diff --cached -M --stat` must show 38 renames at 100%); allowed edits (constraint 2): pin-gitlab-ci `fixture_snapshots.rs` keeps only the `__file` assertion, `common::fixtures()`/`FIXTURES` use `concat!(env!("CARGO_MANIFEST_DIR"), "/../pin-gitlab-ci/tests/fixtures")` (same dir for pin-gitlab-ci, reachable from pin-toolkit).
- [ ] Run → PASS with `INSTA_UPDATE=no`; `git status`: no `*.snap` content change, no `.snap.new`.
- [ ] Planted: render rows with `{plugin}:{kind}` early (T27 preview) → 19 human snapshots FAIL ☐; make the toolkit skip the write → P-same-bytes FAILS ☐. Revert each.
- [ ] Gate → 297. Commit: `feat: M5d pin-toolkit runs the one-pass pipeline; fixture snapshots move to the toolkit (38 renames, no diff)`.

## Task T22 — exit-code table (§6.5, §9 gate)

**Files:** new `crates/pin-toolkit/tests/exit_codes.rs`.

- [ ] Test `exit_code_table` (one row per case, failure message names the row; each row its own `.tmp/` repo with `.git/`; fakes as T21; after each run assert P-check-pure where marked and recompute P-exit from `--json` output when the row uses it — until T28 the JSON has no `plugins.*.errors`, so the recomputation sees summary counts only; T28 extends it to the errors and adds row 6j, PR-L8):

  | # | Setup | Args | Code | Also |
  |---|---|---|---|---|
  | 1 | `.gitlab-ci.yml` needs a pin | — | 0 | file changed |
  | 2 | same | `--check` | 1 | bytes unchanged |
  | 3 | after row 1 | `--check` | 0 | idempotent |
  | 4 | `image: missing/x:1` (fake 404) | — | 1 | row `unresolved` |
  | 5 | same | `--check --json` | 1 | P-exit from JSON |
  | 6 | `bad.gitlab-ci.yml` = `a: 1\nb: c: d\n` + good `.gitlab-ci.yml` | — | 2 | good file pinned; stderr `pin-toolkit: bad.gitlab-ci.yml:2: invalid YAML: …` |
  | 7 | — | `gitlab-ci missing.yml` | 2 | stderr `pin-toolkit: missing.yml: …` |
  | 8 | — | `--bogus` | 2 | stdout empty |
  | 9 | repo file with `[hosts]` | — | 2 | nothing written |
  | 10 | `docs/readme.yml` | `docs/readme.yml` | 2 | `no plugin claims …` |
  | 11 | — | `helm` | 2 | `plugin helm not in this build` |
  | 12 | — | `--only helm` | 2 | same |
  | 13 | needs a pin | `--dry-run` | 0 | bytes unchanged |
  | 14 | `image: $CI_REGISTRY_IMAGE:1` only | `--check` | 0 | row `skipped` (reason as today), no fake request |
  | 15 | repo `files = ["["]` | — | 2 | `invalid glob …` |
  | 16 | needs a pin; env `PIN_TOOLKIT_VERSION=0.2` | — | 0 | file changed; stderr = `pin-toolkit: ignoring unknown environment variable PIN_TOOLKIT_VERSION\n` (PR-M3) |

- [ ] Run → PASS (characterisation of T10–T21; the planted reds prove it bites).
- [ ] Planted: map `FileError::Parse` to no entry (exit 1, D32 regression) → row 6 FAILS ☐; let `--check` write → rows 2/13 FAIL ☐; exit 2 on an unknown `PIN_TOOLKIT_*` → row 16 FAILS ☐. Revert each.
- [ ] Gate → 298. Commit: `test: M5d exit-code table for pin-toolkit (0/1/2, --check, fatal cases)`.

## Task T23 — credentials on the wire through the binary path (§9 gate)

**Files:** new `crates/pin-toolkit/tests/credentials.rs`.

- [ ] Tests (two MockServers A = `gitlab.example.com`, B = `gitlab.com` via `tune`; the include resolves on the primary, the component on its own host; response shape from `crates/pin-core/tests/gitlab_client.rs`): `ci_job_token_never_sent_to_another_host` (env `CI_JOB_TOKEN=jt`, `CI_SERVER_HOST=gitlab.example.com`; args `--gitlab-host gitlab.com` (primary B ≠ CI server A); repo `gitlab-hosts = ["gitlab.example.com"]`; file `include:\n  - project: g/p\n    ref: main\ncomponent: gitlab.example.com/g/q/c@main\n` → A receives `JOB-TOKEN: jt` (`.expect(1..)`), B: `header_exists("JOB-TOKEN")` → 500 `.expect(0)`; both rows `pinned`, code 0) · `gitlab_token_only_to_primary` (env `GITLAB_TOKEN=pt`; args `--gitlab-host gitlab.example.com`; repo `gitlab-hosts = ["gitlab.com"]`; file with the include and `component: gitlab.com/g/q/c@main` → A receives `PRIVATE-TOKEN: pt`, B: `header_exists("PRIVATE-TOKEN")` `.expect(0)`) · `ci_server_host_is_default_primary` (PR-M10, the common self-hosted CI job: env `CI_SERVER_HOST=gitlab.example.com`, `CI_JOB_TOKEN=jt`; **no** `--gitlab-host`/`PIN_TOOLKIT_GITLAB_HOST`; repo `gitlab-hosts = ["gitlab.com"]`; file with the include (→ primary A, `header("JOB-TOKEN", "jt")` `.expect(1..)`) and `component: gitlab.com/g/q/c@main` (→ B anonymous: `header_exists("JOB-TOKEN")` → 500 `.expect(0)`); both rows `pinned`, code 0; exercises `fold` → `from_vars` fallback → `auth_for` end to end). P-token. Run → PASS (T7 + T21 already wired; the planted reds prove the tests bite).
- [ ] Planted: in `GitlabEnv::auth_for`, bind the job token to `primary` (today's behaviour) → `ci_job_token_never_sent_to_another_host` FAILS (B receives `JOB-TOKEN`) ☐; send `GITLAB_TOKEN` to every host → `gitlab_token_only_to_primary` FAILS ☐; send the job token to every host in the resolve set → `ci_server_host_is_default_primary` FAILS (B receives `JOB-TOKEN`) ☐. Revert each.
- [ ] Gate → 301 (+3). Commit: `test: M5d pin-toolkit sends CI_JOB_TOKEN only to CI_SERVER_HOST (wiremock)`.

## Task T24 — delete `[[bin]] pin-ci`; driver becomes a test helper; argv[0] alias tests

**Files:** `crates/pin-gitlab-ci/{Cargo.toml, src/lib.rs}`, delete `src/main.rs`, `git mv src/driver.rs tests/common/driver.rs`, `tests/common/mod.rs` (`pub mod driver;`), `tests/{e2e.rs, e2e_components.rs, live.rs, oracle.rs}` (imports only; `e2e.rs` also receives the moved driver unit test, PR-L2), new `crates/pin-toolkit/tests/aliases.rs`.

- [ ] RED (`aliases.rs`, `#[cfg(unix)]`, std `Command` on symlinks in `.tmp/alias-{pid}/bin/` → `env!("CARGO_BIN_EXE_pin-toolkit")`, `env_clear()`, `HOME` = temp, repo with `.git/`, `notes.yml` = `image: $CI_REGISTRY_IMAGE:1\n`): `pin_ci_alias_is_gitlab_ci_subcommand` (`pin-ci --dry-run notes.yml` → 0, stdout row for `notes.yml:1`; control: `pin-toolkit --dry-run notes.yml` → 2 `no plugin claims notes.yml …`) · `pin_gitlab_ci_alias` (same via `pin-gitlab-ci`) · `pin_helm_alias_not_in_build` (→ 2, stderr `pin-toolkit: plugin helm not in this build`). Run → PASS already for the toolkit (dispatch exists since T17) — RED here is the deletion check: `cargo metadata --format-version 1 | jq -r '.packages[] | select(.name=="pin-gitlab-ci") | .targets[] | select(.kind==["bin"]) | .name'` prints `pin-ci` → must print nothing after GREEN.
- [ ] GREEN: remove `[[bin]]`, `clap`, `tokio` (→ dev-dependency with `rt-multi-thread`, `macros`), the doc-hidden `driver` module and re-exports; driver moves **verbatim** (T-D25) except `use crate::…` → `use pin_gitlab_ci::…` and its `#[cfg(test)] mod tests`, which is stripped: `tests/common/` is mounted by 8 pin-gitlab-ci test binaries and by pin-toolkit's, so a test there would run once per binary (PR-L2). Its one test `outcome_prints_partial_rows_before_the_file_error` (with its `row()` helper) moves into `tests/e2e.rs` unchanged except imports (`common::driver::{outcome, PinFilesError}`, `pin_core::report::{Report, Row, Status}`) — allowed edit (constraint 2). Tests change imports to `common::driver::{…}` (+ `mod common;` where missing). Deleted: `main.rs` unit tests `cli_update_flags_parsed`, `cli_gitlab_host_parsed`, `flag_default_zero` (subject deleted; superseded by T17 `cli_flags_map_to_layer`).
- [ ] Planted: remove the `ALIASES` entry → `pin_ci_alias…` FAILS ☐; revert.
- [ ] Evidence: `grep -c 'fn outcome_prints_partial_rows_before_the_file_error' crates/pin-gitlab-ci/tests/e2e.rs` = 1 and `grep -n '#\[test\]\|#\[tokio::test' crates/pin-gitlab-ci/tests/common/driver.rs` → empty.
- [ ] Gate → 301 (−3 `main.rs` tests, −1 driver unit test in the lib, +1 the same test in `e2e.rs`, +3 aliases = 0). Commit: `feat!: M5d delete the pin-ci binary; pin-ci is an argv[0] alias of pin-toolkit gitlab-ci (D33)`.

## Task T25 — D38: raw clients only behind `test-support` (review M3)

**Files:** `crates/pin-core/{Cargo.toml, src/lib.rs, src/oci/mod.rs, src/gitlab.rs, src/run.rs}`, `[[test]]` entries `mock_registry`, `registry_hosts`, `hub_api`, `oci_decide`, `gitlab_client`, `live_gitlab`, `memo` (`required-features` += `"test-support"`), dev-deps of pin-gitlab-ci and pin-toolkit (already `test-support`).

- [ ] RED: add the gate lines `cargo check -p pin-gitlab-ci -p pin-toolkit` and the `cargo tree` D38 loop (PR-M4), then plant each of `use pin_core::oci::RegistryClient as _;`, `use pin_core::gitlab::GitLabClient as _;`, `#[allow(dead_code)] fn _plant(c: pin_core::run::Ctx) { let _ = c.with_oci(Err(String::new())); }` (names only `Ctx::with_oci`) in `crates/pin-gitlab-ci/src/lib.rs`, and the first one once in `crates/pin-toolkit/src/lib.rs` (binary side) → `cargo check` **succeeds** today (raw clients public) — that success is the red ☐.
- [ ] GREEN: `oci::registry` and `gitlab` modules: `#[cfg(feature = "test-support")] #[doc(hidden)] pub mod …` / `#[cfg(not(feature = "test-support"))] pub(crate) mod …` with `#[cfg_attr(not(feature = "test-support"), allow(dead_code))]`; `pub use registry::RegError` stays public (plugins see it via `Ctx`); `Ctx::with_oci/with_gitlab` `#[cfg(any(test, feature = "test-support"))] #[doc(hidden)]`; `RegistryClient::new`/`GitLabClient::new` likewise; `ResolverEnv::from_process` likewise (PR-L9: a process-env reader below the binary edge; after T24 only test helpers call it).
- [ ] Run: each planted item → `cargo check -p pin-gitlab-ci -p pin-toolkit` FAILS (E0603/E0432/E0599) ☐ ☐ ☐ ☐ (three plugin-side plants, one binary-side); revert the plants → passes. Plant `features = ["oci", "gitlab", "yaml", "test-support"]` on pin-gitlab-ci's **normal** `[dependencies] pin-core` → the `cargo tree` loop prints `FAIL D38 [pin-gitlab-ci]` and `FAIL D38 [pin-toolkit]` while `cargo check` stays green ☐ (verified on a scratch copy of 8f053e2 while revising the plan); revert. Matrix legs without `test-support` still compile (tests needing raw clients are skipped by `required-features`).
- [ ] Gate → 301. Commit: `refactor: M5d raw registry/GitLab clients only under pin-core test-support (D38, review M3)`.
- [ ] **Review checkpoint 2** (Open for main 12): request a review of T13–T25 (toolkit before visible output changes) before T26.

## Task T26 — snapshot metadata only (PR-M7)

The 38 snapshots moved in T21 still carry the pin-gitlab-ci headers (`source: tests/fixture_snapshots.rs` in 26 files, `source: crates/pin-gitlab-ci/tests/fixture_snapshots.rs` in 12; `expression: redact(&r.render_human())` / `redact(&r.to_json())`). insta compares contents only, so T21 passes, but the first `INSTA_UPDATE=always` in T27–T29 would rewrite both header lines in every file and mix them into the reviewed content diffs. This commit takes the header churn alone.

**Files:** the 38 `crates/pin-toolkit/tests/snapshots/fixture_snapshots__*__{human,json}.snap` (header lines only).

- [ ] `INSTA_FORCE_UPDATE=1 INSTA_UPDATE=always cargo test -p pin-toolkit --test fixture_snapshots` (insta 1.49 honours `INSTA_FORCE_UPDATE`).
- [ ] Check (each must print nothing): `git diff -U0 -- '*.snap' | grep -E '^[-+]' | grep -vE '^(---|\+\+\+) [ab/]' | grep -vE '^[-+](source|expression): '` · `git diff --name-only -- '*.snap' | grep -vE '^crates/pin-toolkit/tests/snapshots/fixture_snapshots__.*__(human|json)\.snap$'`; and `git diff --name-only -- '*.snap' | wc -l` ≤ 38; no `.snap.new`.
- [ ] Planted: after the force update, `sed -i '5s/$/ X/'` on one of the files (a content line) → the first check prints that line ☐; rerun the force update (content restored) → empty again.
- [ ] Gate → 301 (`INSTA_UPDATE=no` green, no count change). Commit: `test: M5d toolkit snapshot headers only (source/expression; no content change)`, body: the file count and the check output (empty).

## Task T27 — human output: `plugin:kind`, per-plugin summaries, follow-ups (§6.4)

**Files:** `crates/pin-core/src/report.rs`, 19 `crates/pin-toolkit/tests/snapshots/*__human.snap`, `crates/pin-gitlab-ci/tests/e2e.rs` (2 tests, PR-M1).

- [ ] RED (unit): allowed edit — `human_render_exact` expectation gains `  {plugin}:{kind}` after `file:line` (the spec'd change) · new `human_render_multi_plugin` (rows of `a` and `b`, follow-ups `[x]` → rows, then the total summary line, then `a: pinned: …`, `b: pinned: …`, then `follow-up: x`; single plugin → rows + the total line only, no per-plugin line; PR-L1, §6.4). Run → FAIL.
- [ ] GREEN: `render_human` per T-D21. Allowed edits (constraint 2, PR-M1): `e2e.rs:308-313` `"{}:2  skipped (unparseable reference)"` → `"{}:2  gitlab-ci:image  skipped (unparseable reference)"`; the moved `outcome_prints_partial_rows_before_the_file_error` in `e2e.rs` (T24): both `"a.yml:2  skipped (r)"` → `"a.yml:2  gitlab-ci:image  skipped (r)"`. (`e2e.rs:716`'s `contains("  updated  …")` and `ends_with("pinned: 2, …")` survive unchanged: the total line keeps its format.) Pin-toolkit sites (review-2 M3): `tests/run.rs:34`, `tests/exit_codes.rs:131` (row 4), `:232` (row 14) gain `  gitlab-ci:image` after `:1`; `aliases.rs:51` (`starts_with("notes.yml:1  ")`), the review-2 rows that check `starts_with("…:1  ")` + `contains("  pinned  ")`/`contains("skipped (…)")`, and the `ends_with("pinned: …")` summary checks survive. "More than one plugin ran" (L7, main): the plugins that had ≥ 1 routed file; an entry that holds only errors from routing/walking (conflicts, walk errors) does not count.
- [ ] Snapshots: `INSTA_UPDATE=always cargo test -p pin-toolkit --test fixture_snapshots`; `git status` lists exactly the 19 `__human` files; every changed line is a row line gaining `  gitlab-ci:{image|component|include}` (services rows still `image` until T29); summary lines unchanged. Mechanical check (PR-L4), must print nothing:
  ```bash
  diff <(git diff -U0 -- '*__human.snap' | grep '^-[^-]' | cut -c2-) \
       <(git diff -U0 -- '*__human.snap' | grep '^+[^+]' | cut -c2- | sed -E 's/  gitlab-ci:(image|component|include)  /  /')
  ```
- [ ] Planted: put `plugin:kind` after the status → `human_render_exact` FAILS ☐; after the snapshot update, hand-edit one summary line in one `__human` file → the mechanical check prints it ☐. Revert each.
- [ ] Gate → 302. Commit: `feat: M5d human report shows plugin:kind, per-plugin summaries and follow-ups (19 human snapshots)`, body lists the 19 files.

## Task T28 — JSON report v1 (§6.4, D15)

**Files:** `crates/pin-core/src/report.rs`, `crates/pin-toolkit/tests/{run.rs, exit_codes.rs}`, 19 `*__json.snap`.

- [ ] RED (unit): `json_schema_v1` — top-level keys in order `[schema_version, results, summary, plugins, follow_ups]`, `schema_version == 1`, row keys `[plugin, kind, file, line, reference, old, new, status(, reason)]`, `plugins.<id> = {summary, errors}`, `errors[i] = {file, line?, message}`; allowed edit `json_render_roundtrip` for the new fields · `json_lists_file_errors` (`run.rs`: row-6 setup with `--json` → `plugins["gitlab-ci"].errors == [{file: "bad.gitlab-ci.yml", line: 2, message: "invalid YAML: …"}]`, code 2). Run → FAIL.
- [ ] GREEN: remove the T10 `#[serde(skip)]`s (and Row's on `plugin`/`kind`); field order as Snippets. Allowed edit (PR-L8): `exit_codes.rs`' P-exit recomputation also counts `plugins.*.errors`, and a new row **6j** = row 6 with `--json` → 2, P-exit recomputed from the JSON (now sees the error). Decided by main (review-2 L7): `Report.plugins` is seeded with every enabled plugin (`Plan::ids()`), so the JSON `plugins` keyset = the enabled plugins (an empty repo → each enabled plugin with a zero summary and no errors); pinned in `json_schema_v1` and spec §6.4 as built. Each plugin's `errors` are sorted by `(file, line)` regardless of origin (run errors, conflicts, walk errors), so the JSON is deterministic.
- [ ] Snapshots: `INSTA_UPDATE=always …`; exactly the 19 `__json` files; each diff = `schema_version`, per-row `plugin`/`kind`, `plugins`, `follow_ups: []`. Mechanical check (PR-L4), must print nothing:
  ```bash
  body() { awk 'n>=2 {print} /^---$/ {n++}'; }   # snapshot content after the 2nd `---`
  for f in $(git diff --name-only -- '*__json.snap'); do
    diff <(git show HEAD:"$f" | body | jq -S .) \
         <(body < "$f" | jq -S 'del(.schema_version, .plugins, .follow_ups) | .results |= map(del(.plugin, .kind))') >/dev/null || echo "FAIL old content $f"
    body < "$f" | jq -e '.schema_version == 1 and .follow_ups == [] and (.plugins | keys) == ["gitlab-ci"]
      and .plugins["gitlab-ci"].summary == .summary and .plugins["gitlab-ci"].errors == []
      and (.results | all(.plugin == "gitlab-ci" and (.kind | IN("image", "component", "include"))))' >/dev/null || echo "FAIL v1 fields $f"
  done
  ```
  (all 19 bodies parse with `jq` on 8f053e2.)
- [ ] Planted: drop `schema_version` → `json_schema_v1` FAILS ☐; after the snapshot update, change one `"status"` value in one `__json` file → the mechanical check prints `FAIL old content …` ☐. Revert each.
- [ ] Gate → 304. Commit: `feat: M5d JSON report schema v1 (schema_version, plugin, kind, plugins, follow_ups; 19 JSON snapshots)`, body lists the 19 files.

## Task T29 — `service` kind tag (§9, §4 C2)

**Files:** `crates/pin-gitlab-ci/src/{lib.rs, discover.rs}`, `tests/{plugin.rs, discovery_parity.rs}`, snapshots (subset).

- [ ] RED: unit `services_are_service_kind` (`discover.rs`: `"image: a:1\nservices: [b:1, {name: c:1}]\njob:\n  image: d:1\n  services:\n    - e:1\n"` → kinds `[image, service, service, image, service]`; `services: >-` block → `Unsupported { kind: "service" }`) · `service_kind_off_drops_service_rows` (`plugin.rs`: `KindSet` with `service` off → no services targets, images kept; D16). Run → FAIL.
- [ ] GREEN: `SERVICE`, `KINDS` (Snippets help texts), `image_kind`, `PinTarget::Image.kind`, `target_kind`, `pin_inline_image(.., kind)`. Allowed edits: `tests/plugin.rs:63` kind list gains `"service"`; `discovery_parity.rs::kind()` maps `Image`/`Unsupported` with kind `service` → `"image"` (the frozen legacy record has no service kind); any assertion `kind == "image"` on a services row → `"service"` (list in the commit body).
- [ ] Snapshots: `INSTA_UPDATE=always cargo test -p pin-toolkit --test fixture_snapshots`; changed files ⊆ `{ci-anchors, ci-images-edge, ci-multidoc, ci-pipeline, ci-quoted-comment, ci-r2-crlf, ci-rich, ci-simple, ci-tags, ci-unicode, ci-update-clean, ci-update}__{pin,update}__{human,json}` (fixtures containing `services`); each diff only `gitlab-ci:image` → `gitlab-ci:service` / `"kind": "image"` → `"kind": "service"` on services lines. `__file` snapshots unchanged. Mechanical check (PR-L4), must print nothing:
  ```bash
  diff <(git diff -U0 -- '*.snap' | grep '^-[^-]' | cut -c2- | sed -E 's/  gitlab-ci:image  /  gitlab-ci:service  /; s/"kind": "image"/"kind": "service"/') \
       <(git diff -U0 -- '*.snap' | grep '^+[^+]' | cut -c2-)
  ```
- [ ] Planted: map `services` to `IMAGE` → `services_are_service_kind` FAILS ☐; after the snapshot update, hand-edit a non-services row in one changed file → the mechanical check prints it ☐. Revert each.
- [ ] Gate → 306. Commit: `feat: M5d gitlab-ci service kind (services: rows tagged service; N snapshots)`, body lists the files.

## Task T30 — toolkit live smoke (`PIN_CI_LIVE=1`)

**Files:** new `crates/pin-toolkit/tests/live.rs` (`#![cfg(feature = "gitlab-ci")]`).

- [ ] Test `live_pins_public_image`: returns early unless `PIN_CI_LIVE=1`; binary (`Command`) in a `.tmp/` repo with `.gitlab-ci.yml` = `image: alpine:3.20\n` → code 0, file matches `^image: alpine:3\.20@sha256:[0-9a-f]{64}\n$` (C1 pins inline without a comment; the `# alpine:3.20` belongs to the report row, PR-M8), stdout row contains `gitlab-ci:image  pinned` and ends with `# alpine:3.20`; then `--check` → 0.
- [ ] Run once with `PIN_CI_LIVE=1` if network allows; record result or "not run" in the commit body. Planted (only if run): expect the old `  # alpine:3\.20` suffix in the file regex → the test FAILS ☐.
- [ ] Gate → 307. Commit: `test: M5d pin-toolkit live smoke (PIN_CI_LIVE=1)`.

## Task T31 — docs: README + spec close-out

**Files:** `README.md`, `docs/superpowers/specs/2026-11-09-pin-toolkit-design.md`.

- [ ] README (minimal toolkit rewrite; M5f writes the full per-plugin README): title/usage → `pin-toolkit [OPTIONS] [PATHS]…`, `pin-toolkit gitlab-ci …`, `pin-toolkit plugins`; aliases: "`pin-<plugin>` and `pin-ci` are argv[0] aliases of `pin-toolkit <plugin>` / `pin-toolkit gitlab-ci`; the `pin-ci` alias is kept as one dispatch entry, the symlink ships with the nix package (M6). `pin-ci` has no legacy mode: without paths it walks the repository."; options table (§6.6); config section with the repo schema, layers, merge rules, global-only keys; env table; exit codes 0/1/2 incl. `--check`; GitLab auth (D31/D37 wording); `release_age.unknown`; `ignore`; `service` kind; walk rules (git root, `.gitignore`, global excludes); merge the two "Environment" sections and the two exit-code sections; **L8** under R1/"What gets skipped": "On a flow line with several includes where only some are pinned, the combined `# …` comment lists only the pinned refs; a later run treats the line as commented and adds none for the rest."; Tests section: `PIN_CI_LIVE`/`PIN_CI_ORACLE` unchanged, new toolkit test files. Also (plan review): a plugin subcommand must come before the paths (`pin-toolkit a.yml gitlab-ci` treats `gitlab-ci` as a path, PR-H1); unknown `PIN_TOOLKIT_*` variables only warn, bad values of known ones exit 2 (PR-M3); `--only`/`--skip` on the command line conflict with a subcommand, the env variables are ignored there (PR-L6); human summary = total line, plus one line per plugin when several ran (PR-L1); a walked file claimed by two plugins is a per-file error (PR-L11); `ignore` matches the reference with quotes stripped, so `ignore = ["alpine:3.20"]` does not match an already pinned `alpine:3.20@sha256:…` — use `alpine:3.20*` (review nit); helm `aliases` URLs must not contain credentials (PR-L14); **unbound hosts (PR-M6):** "A `gitlab-hosts` entry other than the primary host and `CI_SERVER_HOST` is resolved anonymously; a private project there fails as `unresolved`. Binding credentials to further hosts arrives with the global config (M5e')."
- [ ] Spec: §9 M5d row → done (date, commit range); §6.1 crate list "config/cli live in pin-toolkit (T-D2)" (Open 1 accepted); §6.2/§6.3/§6.4/§6.6/§6.7 "as built" notes (`Source`, `Selection`, `Route`/`run`, `Report::assemble`/`exit_code`, human/JSON format, memo keys, CLI surface, argv[0] table); §7.1/§7.2 as built (env names, booleans, `concurrency` global-only, unknown `PIN_TOOLKIT_*` → stderr warning, exit unaffected; bad known values → 2 (main, PR-M3); env `only`/`skip` ignored in subcommand/alias mode (PR-L6); helm alias URLs without userinfo (PR-L14); `gitlab-hosts` replace rule); §6.4 as built: total summary line kept, per-plugin lines follow when > 1 plugin ran (main, PR-L1 — matches §6.4); §6.2/§6.3 as built: a walked file claimed by 2+ plugins is a per-file error (PR-L11); §7.3/D37 implemented; **§7.3 as built (PR-M6, main: option b): the `unresolved (<host>: not trusted for credentials — bind it in …)` message is deferred to M5e' — in M5d an unbound host resolves anonymously and a failure keeps the plain `unresolved (…)` reason**; §8.2 notice text; §13 O1 resolved as T-D11; §14.1: M3/D38, L2–L6, L9, L10, N5 resolved (commit ids), L8 documented; record T-D10, T-D12, T-D17 (include key), T-D21, T-D22, T-D28 (env exempt), T-D30 (deferral) as decided in "Open for main — final decisions".
- [ ] Gate → 307; `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --all-features` → 0 warnings. Commit: `docs: M5d pin-toolkit README and spec close-out`.

## Close-out (controller, before merge)

- [ ] Gate + all legs green; `git log --oneline main..` = the two plan commits (8f053e2 + plan-review fixes) + T1–T31 in order.
- [ ] Coverage gap 1–2 and 7 by hand; record "not run" where network/devenv is unavailable.
- [ ] Review the header-only commit (T26) and the three snapshot commits (T27–T29) file by file against their commit bodies; rerun each mechanical check on its commit (`git diff <c>~1 <c>`).

## Self-review

- **Spec coverage:** §6.2 interface (T4, T12, T11), §6.3 run (T12, T20, T21), §6.4 report (T10, T27, T28), §6.5 exit codes (T10, T22), §6.6 CLI + aliases + D33 (T13, T17, T19, T24), §6.7 memo (T6, T21), §6.8 (T9), §6.9 exit 2 (T22 row 6), §7.1–§7.3 (T7, T14–T19, T23), §8.2/O1 (T19), §9 gate items (T18, T21–T24, T27–T29), §14.1 (T1–T5, T8, T25, T31). Not in M5d by design: global config, `config show`, symlinks, helm/gha, the §7.3 "not trusted for credentials" message (T-D30, recorded in T31).
- **Review checkpoints:** after T12 (core) and after T25 (toolkit before visible output changes); T26 (headers only) opens the output-change group.
- **Type consistency:** `Selection` (T11) lands before `Route { sel: &Selection }` (T12); `Source` (T4) before `Selection`'s `plan`; `GitlabEnv`/`ResolverEnv::from_vars` (T7) consumed by T21; `Ctx::gitlab_enabled` (T8) by policy; `Report::assemble/exit_code` (T10) by `run` (T12) and `execute` (T21); `Layer`/`Effective` (T14) by T15–T19; `Fatal` everywhere in pin-toolkit.
- **Planted violations:** every new check has a box; T1, T5, T25 are refactors with before/after evidence or a compile check; T4's only check is a grep (stated); the T25 `cargo tree` line, the T26 header check and the T27–T29 diff checks each have a planted box (PR-M4, PR-M7, PR-L4).
- **Proportion:** bodies only for the wildcard, memo, fold rules, exit table and routing rules; the rest are signatures and tests.

## Open for main — final decisions (2026-11-10)

Main accepted every default at the handshake; the plan review changed #3, #5, #7, #12, #15 and added #17. Each item shows the decision the tasks implement.

1. **`config`/`cli` in pin-toolkit, not pin-core** (§6.1 lists them in pin-core; T-D2). **Decided: accepted; spec §6.1 updated in T31.**
2. **§6.5 "enabled but not compiled in → 2" = explicit CLI/env requests only; repo config → notice** (T-D11, reconciles §6.5 with §8.2). **Decided: accepted.**
3. **Files claimed by 2+ plugins** (T-D10). **Decided:** an *explicit* path claimed by 0 or 2+ plugins → exit 2 before the run (accepted default); a *walked* file claimed by 2+ → per-file `ErrorEntry` under the first claimant, the other files run, exit 2 (PR-L11, the reviewer's fix, applied under main's "apply every finding" rule; observable only from M7).
4. **`ignore` keys:** the value with quotes stripped for image/service/component; `{project}@{ref}` for include; `*` crosses `/` (T-D17). **Decided: accepted** (wording fixed per review nit; README notes `alpine:3.20*`).
5. **Human summary** (T-D21). **Decided (main, PR-L1): the total summary line always, then one `{plugin}: …` line per plugin when more than one plugin ran** (§6.4 verbatim; no spec deviation).
6. **Fatal errors print nothing on stdout, also with `--json`** (T-D22). **Decided: accepted.**
7. **Unknown `PIN_TOOLKIT_*` variable** (T-D7). **Decided (main, PR-M3): one stderr warning `pin-toolkit: ignoring unknown environment variable {VAR}` per variable, exit code unchanged; a bad value of a known variable is still exit 2.**
8. **`concurrency` in a repo file → exit 2 with the global-only message** (§7.2 says "—", not "✗"). **Decided: accepted.**
9. **Presence-only CLI flags; no `--no-update`** (T-D8). **Decided: accepted.**
10. **Primary host precedence `--gitlab-host` > `PIN_TOOLKIT_GITLAB_HOST` > `CI_SERVER_HOST` > gitlab.com; `CI_SERVER_HOST` is not added to the resolve set implicitly** (T-D12). **Decided: accepted** (end-to-end test T23 `ci_server_host_is_default_primary`).
11. **pin-gitlab-ci e2e/live/oracle keep the moved driver (stop-at-first-error, plugin-level); the toolkit gets its own live smoke** (T-D25, T30). Reading of "oracle/live switched": off the deleted public driver API onto the test helper, commands unchanged. **Decided: accepted; full migration at DF1.**
12. **Size and checkpoints.** **Decided (main): one branch, 31 commits (T1–T31, incl. the T26 header-only commit), two review checkpoints: after T12 (core done) and after T25 (toolkit before visible output changes).**
13. **`KINDS` help texts change** (`image: references (C1)`, `services: references (C2)`). **Decided: accepted.**
14. **Globs are relative to the git root; `**/*.gitlab-ci.yml` therefore also claims `ci/.gitlab-ci.yml`** (this repo's dogfood file). **Decided: accepted.**
15. **`--only`/`--skip` with a plugin subcommand** (T-D28). **Decided (main, PR-L6): CLI `--only`/`--skip` + subcommand/alias → exit 2; env `PIN_TOOLKIT_ONLY`/`PIN_TOOLKIT_SKIP` are exempt and ignored in subcommand/alias mode.**
16. **`helm-oci` accepted in `kinds` as a no-op until M7.** **Decided: accepted.**
17. **§7.3 "not trusted for credentials" message for an unbound host** (T-D30). **Decided (main, PR-M6 option b): deferred to M5e'; recorded in T31 (spec §7.3 "as built") plus a README sentence.**

## Review fixes applied

Source: `.superpowers/sdd/2026-11-10-m5d/plan-review.md` (review of 8f053e2). Every site list below was re-checked against the code on 8f053e2 (grep/reads; PR-H1 and PR-M4 by experiment in scratch crates under `.tmp/`).

| Finding | Where in this plan |
|---|---|
| PR-H1 clap `args_conflicts_with_subcommands` | Snippets `cli.rs` (setting removed; verified clap 4.6.7 behaviour noted); T17 RED rows `[--dry-run, gitlab-ci]`, `[gitlab-ci, a.yml, --check]`, `[a.yml, gitlab-ci, b.yml]`, `[plugins, x]` + planted box; Files to read (clap docs). **Not applied as proposed:** the `cmd.is_some() ∧ paths ≠ ∅` guard and its text `paths must follow the plugin subcommand` — verified unreachable (`[a.yml, gitlab-ci, b.yml]` parses as paths `[a.yml, gitlab-ci, b.yml]`, cmd `None`), so the reviewer's alternative (document that `gitlab-ci` there is a path) is applied: T17 row + T31 README |
| PR-M1 allowed-edit lists | Constraint 2 (per-task list with file:line); T7 Files/GREEN (`common/mod.rs:434-437`, `plugin.rs:16-21`, `run.rs:420-425`, `live_gitlab.rs:101,122,147`); T9 (one literal, `driver.rs:101`); T10 Files/GREEN (`report.rs:148,159,170,215`, `driver.rs:178`, `fixture_properties.rs:18`); T11 (`run.rs:437`); T27 (`e2e.rs:308-313` + the moved driver test); Type-checking T7/T9/T10 |
| PR-M2 registry creds read process env | Constraint 5; T-D3; Snippets `REGISTRY_VARS`, `RegistryEnv`, `ResolverEnv.registry`, `ResolverEnv::creds`, `Ctx::oci()`; Env table; T7 RED `registry_creds_from_vars` + planted + grep; T21 env-map note |
| PR-M3 unknown `PIN_TOOLKIT_*` | T-D7; Snippets `env::unknown_vars`, `execute`; User texts; T16 (`env_unknown_vars_warn`, planted); T22 row 16; T31; Open 7 |
| PR-M4 D38 feature check | Constraint 3; gate `cargo tree` loop; T-D14; Gate table; Type-checking; T25 RED (plants `GitLabClient`, `Ctx::with_oci`, binary-side plant) and Run (normal-dep feature plant) |
| PR-M5 tokens in `Debug` | T-D29; Snippets (`GitlabAuth`, `GitlabEnv`, `RegistryEnv`, `GitLabClient`, `HostCreds`); P-redact; T7 RED `debug_redacts_tokens` + planted |
| PR-M6 §7.3 unbound-host message | T-D30 (deferred, main option b); Scenario coverage; T31 README sentence + spec §7.3 as built; Self-review; Open 17 |
| PR-M7 snapshot headers | New **T26** (header-only commit, mechanical check, planted); old T26–T30 renumbered T27–T31 everywhere; Approach; constraints 1–2; Gate table; Close-out |
| PR-M8 pin format | T21 `pins_a_repo_end_to_end` exact bytes via `common::fake_hex`; T30 regex without comment + planted |
| PR-M9 `..`/`./`/outside-root paths | T-D23; Snippets `walk.rs` (`normalize`, `Input.rel: Option<String>`, dedup key); P-route; T20 `explicit_parent_path_from_subdir`, `same_file_two_spellings_one_route` + planted; Scenario coverage |
| PR-M10 default primary from `CI_SERVER_HOST` | T23 `ci_server_host_is_default_primary` + planted; P-token; Review Focus 2; Scenario coverage; counts |
| PR-L1 total summary line | T-D21; Snippets `render_human`; User texts; T27 RED; Scenario coverage; T31; Open 5 |
| PR-L2 T24 count / driver test | T24 GREEN (test module stripped from `tests/common/driver.rs`, test moved into `e2e.rs`), evidence step, count `−3 −1 +1 +3 = 0`; constraint 2 |
| PR-L3 `--no-default-features` counts | Counts paragraph (T13 = 2, then ≥ previous, named in commit bodies); T13; T19 |
| PR-L4 mechanical diff checks | T27 (human `diff` + `sed`), T28 (`jq` semantic check), T29 (service `diff` + `sed`), each with a planted box; Gate table; Close-out |
| PR-L5 checkpoint after T12 | Approach (T12 row, checkpoint markers); T12 and T25 checkpoint steps; Open 12; Self-review |
| PR-L6 env `only`/`skip` exempt | T-D28; Snippets (`cli.rs` comment, `enable` note, `execute`); T19 RED rows + planted; T31; Open 15 |
| PR-L7 T2 grep | T2 check greps only the non-test part of `run.rs` (`sed '/^#\[cfg(all(test/,$d'`); red on 8f053e2 recorded |
| PR-L8 P-exit before T28 | P-exit property; T22 note; T28 allowed edit (`exit_codes.rs`, row 6j); constraint 2 |
| PR-L9 `from_process` public | Snippets (`cfg(any(test, feature = "test-support"))`); T25 GREEN |
| PR-L10 unknown-age reason | T-D18; User texts; T9 RED/GREEN; Scenario coverage |
| PR-L11 walked 2+ claims | T-D10; Snippets `Routed`, `route`, `execute`; P-route; T20 `walked_file_claimed_twice_is_a_file_error` + planted; Scenario coverage; T31; Open 3 |
| PR-L12 Snippets inconsistencies | Snippets: `Status` derives unchanged (no `Deserialize`), `Summary: Default + PartialEq + Eq`, `UpdatePolicy` keeps `PartialEq, Eq`, `GitLabClient::try_new(base, token, job)` from `into_parts()`, `main.rs` exit 2 on `from_os` error + `write_all` (EPIPE); T7 driver; T10 Produces; T13 GREEN; Coverage gap 7 |
| PR-L13 kind literals in `discover.rs` | T5 Files, GREEN (kind positions :211/:213/:230/:272; YAML-key literals :297/:317/:324 stay), evidence grep (red on 8f053e2 recorded) |
| PR-L14 helm alias userinfo | Repo "Rejected" list; User texts; T15 `repo_rejects` rows + GREEN + planted; Scenario coverage |
| Nits / security checklist | T-D17 "value with quotes stripped" + README `alpine:3.20*`; T15 `repo_rejects` rows `gitlab-host = "evil.example"` (top level, `[plugins.gitlab-ci]`). Not applied (optional, outside H1/M/L): `execute_with` split, `git_global(false)` walk seam |

Counts after the fixes: **31 tasks**; all-features passed **239 → 307** (versus the reviewed plan: T7 +2, T16 +1, T20 +3, T23 +1; T24 net 0).

