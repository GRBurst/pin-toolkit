# M5a–c plan — snapshots → workspace → pin-core split (pin-toolkit)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

- Date: 2026-11-10
- Source of truth: `docs/superpowers/specs/2026-11-09-pin-toolkit-design.md` @ 451609a — §6.1–6.2 (workspace, core interface, D36 lazy `Ctx`), §6.9 (context only), §9 (M5a/M5b/M5c rows, R1/R2), §10–11, §12 (D25, D29, D33, D36)
- Base: `main` @ the commit that adds this plan (spec 451609a, M4b-1 0e2047b). Branch: `m5-workspace-core`.
- Out of scope: M5c-y (parser discovery), M5d and later. The M5c core interface below is the contract they build on.
- Tech stack: Rust 1.95, edition 2024, tokio (joined futures, no spawn), reqwest/rustls, wiremock, insta 1.49.

## Goal

Turn the single `pin-ci` crate into a Cargo workspace with a `pin-core` library (resolvers behind
cargo features, built lazily) and a `pin-gitlab-ci` plugin crate (`GitlabCi: Plugin`), guarded by
snapshot + byte-exact + idempotence tests written first; fix R1 and R2 as two reviewed snapshot diffs.

## Motivation (spec §9, D33)

pin-ci has no users, so pin-toolkit has no compatibility layer: the safety net for the restructure is
reviewed snapshots of every fixture's human report, JSON report and edited file, plus two hard
invariants as properties — edits touch only reference bytes, and a second run edits nothing. M5a
builds that net on today's code; M5b and M5c must leave it green, except the two deliberate fixes
(R1 comment order, R2 CRLF offsets) whose snapshot diffs are reviewed.

## Approach (shape of the diff)

| Milestone | Commits | Diff shape |
|---|---|---|
| M5a | A1–A3 + tag | tests only: `insta` dev-dep, `tests/common/mod.rs` harness with hash-driven fake registry/GitLab, `tests/fixture_snapshots.rs`, `tests/fixture_properties.rs`, 5 new fixtures, `tests/snapshots/*.snap`. Zero `src/` change |
| M5b | B1 (2 commits) | `git mv src tests Cargo.toml → crates/pin-gitlab-ci/`; new virtual root `Cargo.toml`; `oracle.rs:21` path fix; one guard test |
| M5c | C1–C12 | C1 move-only (5 modules + 5 tests → `crates/pin-core`), C2 package rename, C3–C5 refactors, C6–C8 new core API, C9 `GitlabCi`, C10 switch + delete old path, C11 R1 fix, C12 R2 fix |

## Architecture

| Unit | Responsibility | Task |
|---|---|---|
| `tests/common/mod.rs` (gitlab-ci) | fake servers, fixture list, run helper, `redact`, `visible`, property checkers | A1, A2, C11, C12 |
| `tests/fixture_snapshots.rs` | ∀ fixture × mode: 3 insta snapshots | A1 |
| `tests/fixture_properties.rs` | byte-exact, idempotence (+R1/R2 properties later) | A2, C11, C12 |
| root `Cargo.toml` | virtual workspace, `[workspace.package]`, `[workspace.dependencies]` | B1, C1 |
| `crates/pin-core/src/oci/{image_ref,registry,splice,decide}.rs` | image refs, docker-v2 client `[oci]`, image splices, `decide_image`/`pin_inline_image` `[oci]` | C1, C3, C5, C8 |
| `crates/pin-core/src/gitlab.rs` `[gitlab]` | `GitLabClient`, `is_commit_sha`, `GitLabHosts` | C1, C3, C7 |
| `crates/pin-core/src/ver/{docker,age}.rs` | today's `versioning`; RFC3339 + `is_old_enough` | C1, C3 |
| `crates/pin-core/src/report.rs` | `Status`, `Row`, `Report`, `Summary`, renderers | C1, C4 |
| `crates/pin-core/src/edit.rs` | `Splice`, `apply`, `requote`, `trailing_comment`, `coalesce_comments` | C5, C11, C12 |
| `crates/pin-core/src/text.rs` | `LineIndex` | C12 |
| `crates/pin-core/src/plugin.rs` | `KindSpec`, `KindSet`, `Finding`, `Plugin`, `DynPlugin`, `join_all` | C6 |
| `crates/pin-core/src/run.rs` | `UpdatePolicy`, `ResolverEnv`, `Ctx` (lazy), `RunOptions`, `FileError`, `pin_file`, `now_unix` | C6, C7 |
| `crates/pin-gitlab-ci/src/lib.rs` | `GitlabCi: Plugin` (scan = `discover` filtered by kind) | C9, C10 |
| `crates/pin-gitlab-ci/src/policy.rs` | component/include policy, `is_catalog_managed` | C9, C10 |
| `crates/pin-gitlab-ci/src/driver.rs` (`#[doc(hidden)]`, deleted in M5d) | `PinOptions`, `pin_files`, `exit_code`, `client_from_env`, `gitlab_from_env` for the `pin-ci` bin and tests | C10 |
| `crates/pin-gitlab-ci/src/discover.rs` | unchanged (verbatim until M5c-y) | — |
| `crates/pin-gitlab-ci/src/main.rs` | `[[bin]] pin-ci`, unchanged but for `use` paths | C2 |

## Global constraints

1. **No behaviour change** in any commit except C11 (R1) and C12 (R2). Proof per commit: `INSTA_UPDATE=no` snapshot run green and `git status` shows no `*.snap` change. C11/C12 change only the snapshots named in their task.
2. **Existing tests are preserved.** Edits allowed only where a task says so and why (import paths after moves/renames; three report/edit unit tests whose types change; moved unit tests travel with their function).
3. **Move-only commits** (B1, C1) contain `git mv` plus the minimum to compile: manifests, `mod`/`pub use` lines, one visibility token (`is_old_enough`: `pub(crate)` → `pub`), test `use` lines. No reformatting of moved files; `git show --stat -M` must list them as renames.
4. **Features (D36):** pin-core `default = []`, `oci`, `gitlab`; every reqwest/sha2 use behind them. pin-gitlab-ci enables `["oci", "gitlab"]`. `cargo test -p pin-core` alone silently skips `required-features` tests (verified in scratch) → always test with `--workspace --all-features` and check the passed-count.
5. **Error texts carry over verbatim** (spec §6.2 rules): digest errors = inner message, `tag list: {e}`, `hub api: …` prefix rule, GitLab `e.to_string()`, skip reasons `unparseable reference`, `host not enabled: {host}`, `catalog-managed version`, `templated reference`, `release age: no candidate ≥ {N}d`.
6. **Async without Send:** futures are joined, never spawned (`join_all` + `tokio::sync::Semaphore`, as today). `Ctx` uses `std::cell::OnceCell`.
7. **Snapshots:** `insta = "1"` (1.49.0, fetched OK from crates.io in the sandbox). insta normalises `\r\n`→`\n` and trailing newlines (verified), so file snapshots go through `visible()`. Accept with `INSTA_UPDATE=always`, review with `git diff`, never commit `*.snap.new`. No `cargo-insta` needed.
8. **Tests never touch the network by default:** registry hosts in fixtures ∈ {`docker.io`, `ghcr.io`} (harness asserts), GitLab component host `gitlab.com` → fake. `PIN_CI_LIVE`/`PIN_CI_ORACLE` keep their names (D29).
9. Env/commits: `export CARGO_HOME="$PWD/.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH"`; unsigned commits `git -c commit.gpgsign=false -c core.excludesFile=/dev/null commit`; every `git add`/`status` with `-c core.excludesFile=/dev/null`; scratch only under `.tmp/` (gitignored at any depth via `.tmp/`).

### Check (run after every task; "the gate")

```bash
export CARGO_HOME="$PWD/.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH"
INSTA_UPDATE=no cargo test --workspace --all-features > .tmp/m5-test.log 2>&1; echo "cargo exit $?"
grep -E '^test result' .tmp/m5-test.log | awk '{p+=$4; f+=$6} END {print "passed", p, "failed", f}'
cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings
git -c core.excludesFile=/dev/null status --porcelain     # only this task's files; never *.snap.new
```

From C1 on, also the feature matrix:

```bash
for f in "" "--features oci" "--features gitlab"; do
  cargo clippy -p pin-core --no-default-features $f --all-targets -- -D warnings || echo "FAIL [$f]"; done
```

Baseline (measured 2026-11-10 on main 451609a with this env): `cargo exit 0`, **passed 150 failed 0**, fmt and clippy clean.
Expected passed after each task (= previous + tests the task adds): A1 153 · A2 156 · A3 156 · B1 157 · C1 157 · C2 157 · C3 158 · C4 159 · C5 160 · C6 169 · C7 172 · C8 180 · C9 184 · C10 184 · C11 186 · C12 190. A lower number is a failure (silently skipped tests) unless the task explains it.

## Decisions

| # | Decision | Rationale | Rejected |
|---|---|---|---|
| P1 | insta, default features, `assert_snapshot!` on strings | spec suggestion; works offline once fetched; readable `.snap` diffs | hand-rolled goldens (no review tooling); `insta` `glob`/`filters` features (extra deps; a loop + hand-rolled `redact` suffice) |
| P2 | One hash-driven fake registry + fake GitLab for all fixtures | a new fixture needs no new mock; deterministic digests/SHAs | per-fixture mocks (rot with every fixture) |
| P3 | `visible()` for file snapshots (`\r`→`␍`, missing final `\n` marked) | insta hides CRLF and EOF newline, which R2 changes | snapshot raw bytes (R2 diff invisible) |
| P4 | Report `file` normalised to the fixture name; `127.0.0.1:<port>` → `127.0.0.1:[PORT]` | scratch paths carry the PID; mock ports are random | insta redactions (need `filters`) |
| P5 | Byte-exact checked by *reverting* rows on the edited text (line-aligned), not by splice lists | works on today's API and survives M5b/M5c unchanged | reading splices (no public access before C6) |
| P6 | C1 compiles moved files verbatim via one pin-core root alias + pin-gitlab-ci `pub use` re-exports; cleanup in C3/C10 | true move-only commit, zero gitlab-ci test churn | fixing paths inside moved files in C1 (breaks rename detection review) |
| P7 | `Status` gets `as_str`, `Display`, `PartialEq<&str>`, `PartialEq<str>` | ~25 test comparisons (`r.status == "ok"`) stay verbatim | rewriting tests to `Status::Ok` |
| P8 | `Row` has `plugin`, `kind` fields (§6.2) marked `#[serde(skip)]`, not rendered, until M5d | M5c snapshots must not change (§9); data exists for M5d | adding them to JSON now (unreviewable churn in M5c) |
| P9 | No `memo` field, no `run()`/`Route` in M5c | §9 assigns memo, routing, combined report to M5d; a private field added later breaks nothing | placeholder structs |
| P10 | `Ctx::with_oci`/`with_gitlab(Result<…>)` presets + `Ctx::resolvers_built()` | test injection of mock clients, construction-error coverage, the §11 "construction counter" | `ResolverEnv` base-URL overrides (moves test hooks into prod config) |
| P11 | `RegistryClient`, `GitLabClient`, `Creds` become `Clone` (caches → `Arc<Mutex<…>>`) | driver's `pin_files(&reg, &gitlab)` clones into `Ctx`; ~40 test call sites stay verbatim; shared caches = today's semantics | changing every e2e call site |
| P12 | `pin_files` survives in `#[doc(hidden)] pub mod driver` (re-exported at the crate root) until M5d | the `pin-ci` bin and tests need it; spec: "test helper until M5d, not API" | keep it in lib.rs as API |
| P13 | `GitLabHosts { primary, clients }`; host gate = `GitlabCi.gitlab_hosts` (plugin config); includes resolve on the primary | §6.2 "plugin owns host gates"; M5d fills both from `gitlab-hosts`/D31 | gate inside core |
| P14 | `UpdatePolicy` has no `unknown_age`/`AgePolicy` in M5c (main, 2026-11-10) | YAGNI: `Block` has no switch before the M5d config; M5d adds field + enum + switch together; behaviour = today's (`allow`) | implementing `Block` now without a switch |
| P15 | Overlap check pairwise in `pin_file`; today's `BTreeMap` dedup-by-start dropped | §6.2 rule; discover never yields equal starts, so no output change | keep silent dedup |
| P16 | C5 core `apply`/`trailing_comment` are bug-compatible; C11/C12 change them | keeps each fix its own commit with its own snapshot diff | fixing while moving |
| P17 | `LineIndex` arrives with R2 (C12); `discover.rs` keeps its own line split | discover is verbatim until M5c-y (§9) | rewiring discover in M5c |
| P18 | README: test commands fixed in C2; one note each in C11/C12; rest in M5f | spec: README note per fix; full rewrite is M5f | leaving broken commands until M5f |

Recorded, not fixed (main, 2026-11-10: left to M5c-y; spec §9 row R8): **R8** — under CRLF, `discover()` treats `key:\r` as a key with a value, so block children are not discovered (`include:` blocks, `services:` blocks, every job-nested `image:`); and `.base: &base` children are not discovered (anchor counts as a value). A3 snapshots record both.

## Gate (spec §11; the repo has no principles document)

| Property | Status | Where |
|---|---|---|
| Byte-exact | PASS | A2 `byte_exact_over_all_fixtures` (+ C11/C12 refinements) |
| Idempotent | PASS | A2 `idempotent_over_all_fixtures` |
| Snapshot review | PASS | A1/A3 baseline; C11/C12 the only diffs |
| Discovery parity | N/A | M5c-y |
| Lazy resolvers | PASS | C7 `lazy_resolvers.rs` + C1 feature matrix (`--no-default-features`, `oci`, `gitlab`) |
| Plugin isolation | PASS (part) | C9 `scan_filters_by_kind` (no network); single-plugin builds N/A until M5d/M6 |
| Fail loud | PASS (part) | C7/C8 construction error → `unresolved`; exit-2 rules N/A (M5d) |
| Planted violations | PASS once every box in each task is ticked | per task |

## Files to read first (implementer step 0)

Spec §6.1, §6.2, §9, §11. `src/lib.rs` (whole), `src/edit.rs`, `src/report.rs`, `src/main.rs`, `src/discover.rs:1-160`,
`src/registry.rs:1-280` and `:750-860`, `src/gitlab.rs:1-160` and `:315-340`, `tests/e2e.rs:1-110`, `tests/e2e_components.rs:1-100`,
`tests/oracle.rs:14-48`, `Cargo.toml`, `.gitignore`. Line refs below are on main @ 451609a.

## Type-checking strategy

`cargo clippy --workspace --all-targets --all-features -D warnings` after every task, plus the pin-core feature matrix from C1.
Expected rejections that prove structure: `--no-default-features` rejects any ungated `reqwest`/`RegistryClient`/`GitLabClient`
use; the `async_fn_in_trait` lint rejects `Plugin` without `#[allow(async_fn_in_trait)]`; dead private `Ctx.env` is avoided by
the `pub fn env()` accessor; `Row` literals without `plugin`/`kind` fail to compile (C4 catches every construction site).

## Testing strategy (per layer)

| Layer | Covers | Needs |
|---|---|---|
| Unit (in-module) | pure fns: splices, `apply`, `coalesce_comments`, `LineIndex`, `KindSet`, `Status`, age math, `is_commit_sha` | nothing |
| Core integration (`crates/pin-core/tests/`) | registry/GitLab clients (moved), `pin_file` with a fake plugin, laziness, `decide_image`/`pin_inline_image` | wiremock |
| Plugin integration (`crates/pin-gitlab-ci/tests/plugin.rs`) | `GitlabCi` scan/resolve contract | fake servers |
| E2E (existing `e2e*.rs`) | full `pin_files` behaviour per milestone | wiremock |
| Snapshot + property (new) | every fixture: output, byte-exactness, idempotence | fake servers |
| Live / oracle | real registries / old pipeline | env-gated: `PIN_CI_LIVE=1`, `PIN_CI_ORACLE=1` + renovate/crane (devenv); not in the gate |

## Properties (derived from the SUT, survive new fixtures)

Let 𝔽 = `tests/fixtures/*.yml` (read at runtime, sorted), modes(f) = {pin} ∪ {update | name starts with `ci-update`}.
For O = fixture text, (R₁, E₁) = report and file after run 1, (R₂, E₂) after run 2, Lᵢ(x) = `x.split_inclusive('\n')`:

- **P-bytes (A2):** |L(O)| = |L(E₁)| ∧ ∀i: revert(E₁ᵢ, rowsᵢ) = Oᵢ, or revert(E₁ᵢ, rowsᵢ) = Oᵢ[..p] ⧺ s ⧺ Oᵢ[p..] with rowsᵢ ≠ ∅, s = `"  # "`⧺…, `'\n' ∉ s`. revert replaces each pinned/updated row's `new` by its `old`, once, in report order; rowsᵢ = rows with `line = i+1` and status ∈ {pinned, updated}.
- **P-idem (A2):** E₂ = E₁ ∧ ∀r ∈ R₂: status ∉ {pinned, updated} ∧ #ok(R₂) = #{pinned, updated, ok}(R₁).
- **P-one-comment (C11):** every inserted s contains exactly one `#`.
- **P-crlf (C12):** ∀i: Oᵢ ends with `\r\n` ⇒ E₁ᵢ ends with `\r\n` ∧ every insertion on line i has `at ≤ |Oᵢ| − 2`.
- **P-comment-present (C12):** ∀i, ∀ pinned row on line i whose `new` has no `@sha256:` (component/include): `'#' ∉ Oᵢ` ⇒ an insertion s exists on line i.

Literals are pinned only where the literal is the criterion: R1's `  # main, dev`, error/skip texts (constraint 5).

## Scenario coverage (spec → check)

| Spec scenario | Check |
|---|---|
| §9 M5a snapshots of human, JSON, edited file for every fixture | A1 `fixture_snapshots` |
| §9 mock-port URLs redacted | A1 `redact_masks_mock_ports` |
| §11 byte-exact / idempotent over all fixtures | A2 `byte_exact_over_all_fixtures` / `idempotent_over_all_fixtures` |
| §9 fixtures for R1, R2 (CRLF), anchors/aliases record today's output | A3 snapshots `ci-r1-flow-include__pin__file`, `ci-r2-crlf__pin__file`, `ci-anchors__pin__human` |
| §9 tag `pre-pin-toolkit` | A3 step |
| §9 M5b move, oracle path | B1 `oracle_repo_root_has_script` + unchanged snapshots |
| §9 M5c move-only commits, `is_sha` dedup, package rename, `[[bin]] pin-ci` kept | C1, C3 `is_commit_sha_table`, C2 bin check |
| §6.2 `Status` serialises lowercase | C4 `status_serde_display_and_str_eq` + JSON snapshots |
| §6.2 `apply` / `requote` / `trailing_comment` | C5 unit tests (moved) |
| §6.2 `Plugin`/`DynPlugin`, bounded join, sort by line, overlap → unresolved, partial writes, dry-run/check no write | C6 `run_pin_file.rs` |
| §6.2/D36 lazy feature-gated resolvers, construction error cached → unresolved | C7 `lazy_resolvers.rs`, C8 `inline_construction_error_is_unresolved`, C1 feature matrix |
| §6.2 `decide_image`/`pin_inline_image`, error texts | C8 `oci_decide.rs` |
| §6.2 `GitlabCi`: pure kind-filtered scan, host gate before client | C9 `plugin.rs` |
| §9 lib.rs → `GitlabCi`, `pin_files` only as helper | C10 (all e2e + snapshots unchanged) |
| §9 R1 `  # main, dev` | C11 snapshot diff + `one_comment_per_line_over_all_fixtures` + `coalesce_merges_same_offset_comments` |
| §9 R2 comment before `\r\n` | C12 snapshot diff + `crlf_preserved_over_all_fixtures` + `pinned_ref_comments_present_over_all_fixtures` |
| §9 oracle command `cargo test -p pin-gitlab-ci --test oracle` | C2 README + close-out compile run |

## Coverage gap (what a human runs)

1. Live and oracle suites need network/credentials/renovate/crane: at close-out run `PIN_CI_LIVE=1 cargo test -p pin-gitlab-ci --test live`, `PIN_CI_LIVE=1 cargo test -p pin-core --all-features --test live_gitlab`, and (in devenv) `PIN_CI_ORACLE=1 cargo test -p pin-gitlab-ci --test oracle`. Without them, real-registry behaviour after the client `Clone` change (C10) is covered only by wiremock.
2. A write failure mid-run (read-only file) is not tested: root in the sandbox bypasses permissions. Behaviour is today's: `pin_files` aborts at the first `FileError`.
3. `pin-ci` binary end-to-end (clap → exit code) has unit tests for flags only; run `cargo run -p pin-gitlab-ci --bin pin-ci -- --dry-run tests/fixtures/ci-simple.yml` by hand once after C10 (expect network use or `unresolved`, exit 0/1, no panic).
4. R8 CRLF/anchor discovery gaps stay (recorded, not fixed).

## Review Focus

1. **CRLF files** (R2/R8): after C12 a CRLF file keeps `\r\n` on every edited line and gets every comment; block children still go undiscovered (R8). A3's `ci-r2-crlf` snapshots make the remaining gap visible.
2. **File without a final newline**: the last line's comment goes at EOF. `ci-components.yml` ends without `\n` on a pinnable `ref:` (A3); P-bytes and the `[no newline at end of file]` marker cover it.
3. **Several refs on one line** (flow sequences): R1 combined comment; overlap rule never fires for distinct spans (C6 `overlapping_edits_mark_both_unresolved` proves it fires only on real overlap).
4. **`--concurrency 0`**: must behave as 1, as today (`.max(1)`): C6 `plan_concurrency_zero_is_one`.
5. **Resolver never needed**: a file of only skipped refs must build no client and send no token: C7 `no_resolver_needed_builds_none`, C9 `component_other_host_builds_no_client`.

---

## Snippets (shared contracts)

### Fake servers and harness — `tests/common/mod.rs` (A1; extended A2, C11, C12)

```rust
#![allow(dead_code)]                       // each test crate uses a subset
pub const MAPPED_HOSTS: [&str; 2] = ["docker.io", "ghcr.io"];
pub struct Fakes { pub registry: MockServer, pub gitlab: MockServer }
pub async fn start_fakes() -> Fakes;       // mounts FakeRegistry on path_regex(r"^/v2/.+$"), FakeGitLab on r"^/projects/.+$"
pub fn registry_client(f: &Fakes) -> RegistryClient;   // Creds default, base_urls {docker.io, ghcr.io} → f.registry.uri()
pub fn gitlab_client(f: &Fakes) -> GitLabClient;       // GitLabClient::new(f.gitlab.uri(), None, false)
pub fn fake_hex(key: &str, len: usize) -> String;      // len ≤ 64, lowercase hex
#[derive(Clone, Copy, Debug)] pub enum Mode { Pin, Update }   // tag(): "pin" | "update"
pub struct Fixture { pub name: String, pub stem: String, pub text: String }
pub fn fixtures() -> Vec<Fixture>;                     // tests/fixtures/*.yml, sorted by name, read_to_string
pub fn modes(name: &str) -> Vec<Mode>;                 // [Pin] + [Update] iff name.starts_with("ci-update")
pub fn scratch_copy(fx: &Fixture, mode: Mode, tag: &str) -> PathBuf;  // .tmp/fixture-{tag}-{stem}-{mode}-{pid}/{name}
pub async fn pin_once(f: &Fakes, path: &Path, mode: Mode) -> Report;  // asserts MAPPED_HOSTS, then pin_files(..).unwrap()
pub fn normalized(report: Report, name: &str) -> Report;              // every row.file = name; Report::new(rows)
pub fn redact(s: &str) -> String;          // "127.0.0.1:<digits>" → "127.0.0.1:[PORT]"
pub fn visible(text: &str) -> String;      // '\r' → '␍'; if !ends_with('\n') append "\n[no newline at end of file]"
```

- `pin_once` options: `PinOptions { concurrency: 4, update: matches!(mode, Mode::Update), ..Default::default() }`.
- Host guard: `∀ PinTarget::Image t ∈ discover(text)`: `ImageRef::parse(t.value())` is `Err` or `host_str() ∈ MAPPED_HOSTS`, else `panic!("fixture {name}: unmapped registry host {h} (would hit the network)")`.
- `fake_hex`: block k ∈ 0..4 = splitmix64(FNV-1a-64 over `[k as u8] ⧺ key.bytes()`), each as `{:016x}`, concatenated, truncated to `len`.
- **FakeRegistry** (`wiremock::Respond`): path `/v2/{repo}/manifests/{ref}` (any method): last segment of `repo` starts with `mystery` or `missing` → 404; else 200 + header `docker-content-digest: sha256:{fake_hex("{repo}:{ref}", 64)}`. Path `/v2/{repo}/tags/list` → 200 `{"tags": T}`, T = `["3.16","3.20","3.24"]` for `library/alpine`, `["v1"]` for `org/img`, else `[]`. Other → 404.
- **FakeGitLab**: path `/projects/{p}/repository/{kind}/{ref}` (p, ref URL-encoded as sent). `p` starts with `g%2Fmissing` → 404. tagish(ref) = first char is a digit, or `v` followed by a digit. `kind = tags` → 200 iff tagish; `kind = branches` → 200 iff ¬tagish. 200 body `{"commit":{"id": fake_hex("{p}@{ref}", 40)}}`.

Verified in a scratch run against today's code: every existing fixture resolves fully except `mystery:1.0` (404 → `unresolved (library/mystery:1.0 not found)`), and every second run is unchanged.

### Line-diff checker (A2) — the algorithm the properties share

```rust
/// Insertion found on one line: byte position in the O line and the inserted text.
pub struct Insertion { pub at: usize, pub text: String }
/// Err(msg) when E line ≠ O line modulo the rows and at most one inserted run.
pub fn line_diff(o_line: &str, e_line: &str, rows: &[&Row]) -> Result<Option<Insertion>, String> {
    // 1. l = e_line; for r in rows: new = r.new (Err if None); Err unless l.contains(new); l = l.replacen(new, &r.old, 1)
    // 2. l == o_line → Ok(None)
    // 3. p = longest common byte prefix of l and o_line, moved back to a char boundary of both
    //    k = l.len() - o_line.len() (Err if l shorter); s = &l[p..p+k]
    //    Ok(Some(Insertion{at: p, text: s})) iff !rows.is_empty() && s.starts_with("  # ") && !s.contains('\n') && l[p+k..] == o_line[p..]
    //    else Err(describe(i, o_line, e_line))
}
pub fn check_byte_exact(o: &str, e: &str, report: &Report) -> Result<Vec<Option<Insertion>>, String>; // per line; line-count check first
pub fn check_idempotent(r1: &Report, e1: &str, r2: &Report, e2: &str) -> Result<(), String>;
```

(`Row` is `RefResult` before C4; write the signature against `pin_ci::report::RefResult` in A2 and let C4's rename update it.)
Status tests use `r.status == "pinned"` so they compile before and after C4.

### Manifests (B1, C1, C2)

```toml
# Cargo.toml (root, B1; [workspace.dependencies] added in C1)
[workspace]
members = ["crates/*"]
resolver = "3"
[workspace.package]
version = "0.1.0"
edition = "2024"
[workspace.dependencies]            # C1
pin-core = { path = "crates/pin-core" }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
tokio = "1"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
wiremock = "0.6"
insta = "1"

# crates/pin-core/Cargo.toml (C1)
[package]  name = "pin-core", version.workspace, edition.workspace
[features] default = [], oci = ["dep:reqwest", "dep:sha2"], gitlab = ["dep:reqwest"]
[dependencies] serde, serde_json, tokio = { workspace = true, features = ["sync"] }, reqwest (optional), sha2 (optional)
[dev-dependencies] tokio = { workspace = true, features = ["rt-multi-thread", "macros"] }, wiremock
[[test]] name = "mock_registry" | "registry_hosts" | "hub_api"  → required-features = ["oci"]
[[test]] name = "gitlab_client" | "live_gitlab"                 → required-features = ["gitlab"]

# crates/pin-gitlab-ci/Cargo.toml (B1 moved; C1 adds pin-core; C2 renames)
[package] name = "pin-gitlab-ci" (C2; "pin-ci" before), version.workspace, edition.workspace
[[bin]] name = "pin-ci", path = "src/main.rs"           # C2; until M5d
[dependencies] pin-core = { workspace = true, features = ["oci", "gitlab"] }, clap, tokio (rt-multi-thread, macros), serde_json
[dev-dependencies] wiremock, insta
```

### pin-core interface (M5c result; what M5c-y/M5d/M8/M9 build on)

```rust
// lib.rs
pub mod edit; pub mod oci; pub mod plugin; pub mod report; pub mod run; pub mod text; pub mod ver;
#[cfg(feature = "gitlab")] pub mod gitlab;

// text (C12)
pub struct LineIndex { starts: Vec<usize>, ends: Vec<usize> }       // ends exclude "\n" and a preceding "\r"
impl LineIndex { pub fn new(text: &str) -> Self; pub fn len(&self) -> usize;
                 pub fn line_start(&self, line: usize) -> Option<usize>; pub fn line_end(&self, line: usize) -> Option<usize>; }

// edit (C5; coalesce C11)
#[derive(Debug, Clone, PartialEq, Eq)] pub struct Splice { pub start: usize, pub end: usize, pub text: String }
impl Splice { pub fn replace(start: usize, end: usize, text: impl Into<String>) -> Self; pub fn insert(at: usize, text: impl Into<String>) -> Self; }
pub fn apply(text: &str, s: &[Splice]) -> String;
pub fn coalesce_comments(s: Vec<Splice>) -> Vec<Splice>;
pub fn requote(old_token: &str, new_value: &str) -> String;                 // = today's splice_replace
pub fn trailing_comment(text: &str, line: usize, token_end: usize, orig: &str) -> Option<Splice>; // text "  # {orig}"

// ver (C1/C3)
pub mod docker;   // today's versioning.rs: Version, parse, compare, pick_latest_stable
pub mod age;      // pub fn parse_rfc3339_unix(&str) -> Option<i64>; pub fn is_old_enough(ts: i64, now: i64, min_days: u64) -> bool

// oci (C1/C3/C5/C8)
pub mod image_ref; pub use image_ref::ImageRef;
pub use splice::{splice_value, splice_update};                                // pure, ungated
#[cfg(feature = "oci")] pub mod registry; #[cfg(feature = "oci")] pub use registry::RegistryClient;
#[cfg(feature = "oci")] pub enum ImageDecision { Current, Pin { digest: String }, Update { tag: String, digest: String }, Skip(String) }
#[cfg(feature = "oci")] pub async fn decide_image(ctx: &Ctx, r: &ImageRef) -> Result<ImageDecision, String>;
#[cfg(feature = "oci")] pub async fn pin_inline_image(ctx: &Ctx, text: &str, line: usize, start: usize, end: usize, value: &str) -> Finding; // kind "image"

// gitlab [gitlab] (C1/C3/C7)
pub fn is_commit_sha(s: &str) -> bool;                     // 40 lowercase hex
pub struct GitLabHosts { primary: String, clients: BTreeMap<String, GitLabClient> }
impl GitLabHosts { pub fn single(host: &str, client: GitLabClient) -> Self; pub fn primary_host(&self) -> &str;
                   pub fn primary(&self) -> &GitLabClient; pub fn get(&self, host: &str) -> Option<&GitLabClient>; }
// GitLabClient::resolve(&self, project, ref) -> Result<String, GitLabError>  (unchanged)

// report (C1/C4)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] #[serde(rename_all = "lowercase")]
pub enum Status { Pinned, Ok, Updated, Skipped, Unresolved }
impl Status { pub fn as_str(self) -> &'static str }   // + Display, PartialEq<&str>, PartialEq<str>
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row { #[serde(skip)] pub plugin: &'static str, #[serde(skip)] pub kind: &'static str,   // serialised from M5d
                 pub file: String, pub line: usize /*1-based*/, pub reference: String, pub old: String,
                 pub new: Option<String>, pub status: Status,
                 #[serde(skip_serializing_if = "Option::is_none")] pub reason: Option<String> }
pub struct Report { pub results: Vec<Row>, pub summary: Summary }   // new, render_human, to_json unchanged

// plugin (C6)
pub type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct KindSpec { pub id: &'static str, pub default_on: bool, pub help: &'static str }
#[derive(Debug, Clone, Default, PartialEq, Eq)] pub struct KindSet(BTreeSet<&'static str>);
impl KindSet { pub fn defaults(kinds: &[KindSpec]) -> Self; pub fn contains(&self, id: &str) -> bool; pub fn set(&mut self, id: &'static str, on: bool); }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding { pub line: usize /*0-based*/, pub kind: &'static str, pub reference: String, pub old: String,
                     pub new: Option<String>, pub status: Status, pub reason: Option<String>,
                     pub edits: Vec<Splice>, pub follow_up: Option<String> }
#[allow(async_fn_in_trait)]   // no Send bound by design (§6.2)
pub trait Plugin { type Target;
    fn id(&self) -> &'static str; fn kinds(&self) -> &'static [KindSpec]; fn default_files(&self) -> &'static [&'static str];
    fn scan(&self, path: &Path, text: &str, on: &KindSet) -> Vec<Self::Target>;          // pure; sorted (line, start)
    async fn resolve(&self, ctx: &Ctx, text: &str, t: &Self::Target) -> Finding; }
pub trait DynPlugin { fn id(&self) -> &'static str; fn kinds(&self) -> &'static [KindSpec]; fn default_files(&self) -> &'static [&'static str];
    fn plan<'a>(&'a self, ctx: &'a Ctx, path: &'a Path, text: &'a str, on: &'a KindSet, concurrency: usize) -> LocalBoxFuture<'a, Vec<Finding>>; }
impl<P: Plugin> DynPlugin for P { /* UFCS to Plugin::*; plan = scan → Semaphore(concurrency.max(1)) → join_all(resolve), target order */ }

// run (C6; resolvers C7)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdatePolicy { pub enabled: bool, pub allow_major: bool, pub min_age_days: u32 }   // M5d adds `unknown_age: AgePolicy` (P14)
#[derive(Debug, Clone, Default)] pub struct ResolverEnv { pub home: Option<PathBuf>, pub gitlab_host: String }
impl ResolverEnv { pub fn from_process(gitlab_host: &str) -> Self }   // home = $HOME
pub struct Ctx { pub update: UpdatePolicy, pub now_unix: i64, env: ResolverEnv,
    #[cfg(feature = "oci")] oci: OnceCell<Result<RegistryClient, String>>,
    #[cfg(feature = "gitlab")] gitlab: OnceCell<Result<GitLabHosts, String>> }
impl Ctx { pub fn new(update: UpdatePolicy, now_unix: i64, env: ResolverEnv) -> Self;   // builds nothing
    pub fn env(&self) -> &ResolverEnv; pub fn resolvers_built(&self) -> Vec<&'static str>;   // e.g. ["oci"]
    #[cfg(feature = "oci")] pub fn oci(&self) -> Result<&RegistryClient, &str>;            // get_or_init(build from env)
    #[cfg(feature = "oci")] pub fn with_oci(self, c: Result<RegistryClient, String>) -> Self;
    #[cfg(feature = "gitlab")] pub fn gitlab(&self) -> Result<&GitLabHosts, &str>;
    #[cfg(feature = "gitlab")] pub fn with_gitlab(self, h: Result<GitLabHosts, String>) -> Self; }
pub fn now_unix() -> i64;                                   // moved from gitlab-ci lib.rs:139
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct RunOptions { pub dry_run: bool, pub check: bool, pub concurrency: usize }
#[derive(Debug, Clone, PartialEq, Eq)] pub enum FileError { Read { path: String, msg: String }, Write { path: String, msg: String } } // Display "{path}: {msg}"
pub async fn pin_file(p: &dyn DynPlugin, ctx: &Ctx, path: &Path, on: &KindSet, o: &RunOptions) -> Result<Vec<Row>, FileError>;
```

Builders (C7): oci = `Ok(RegistryClient::new(Creds::load_from(env.home.as_deref()), HashMap::new()))` (today's `client_from_env`);
gitlab = `Ok(GitLabHosts::single(&env.gitlab_host, GitLabClient::from_env(format!("https://{}/api/v4", env.gitlab_host))))` (today's `gitlab_from_env`).

`pin_file` algorithm (C6; C11 inserts `coalesce_comments`):
1. `text = read_to_string(path)` → `FileError::Read { path: path.display().to_string(), msg: e.to_string() }`.
2. `findings = p.plan(ctx, path, &text, on, o.concurrency).await`; stable sort by `line`.
3. Overlap: for splices a, b of **different** findings, overlap ⇔ (both replacements ∧ a.start < b.end ∧ b.start < a.end) ∨ (one insertion at q, one replacement [s,e) ∧ s < q < e). Each finding involved → `status Unresolved`, `reason "internal: overlapping edits"`, `new None`, `edits` cleared.
4. `splices` = all remaining edits in finding order (C11: `coalesce_comments(splices)`).
5. If `!o.dry_run && !o.check && !splices.is_empty()`: write `apply(&text, &splices)` → `FileError::Write {..}` on error.
6. Rows: `Row { plugin: p.id(), kind: f.kind, file: path.display().to_string(), line: f.line + 1, .. }` (`follow_up` is dropped; decided: widen the return when M8b needs it).

### pin-gitlab-ci interface (C9/C10)

```rust
pub mod discover;                                  // verbatim
mod policy;                                        // is_catalog_managed, resolve_component, resolve_include
#[doc(hidden)] pub mod driver;                     // deleted in M5d
#[doc(hidden)] pub use driver::{PinOptions, client_from_env, exit_code, gitlab_from_env, pin_files};
pub const KINDS: [KindSpec; 3] = [
    KindSpec { id: "image", default_on: true, help: "image: and services: references (C1, C2)" },
    KindSpec { id: "component", default_on: true, help: "top-level component: references (C3)" },
    KindSpec { id: "include", default_on: true, help: "include: project/ref mappings (C4)" } ];
pub const DEFAULT_FILES: [&str; 3] = [".gitlab-ci.yml", "**/*.gitlab-ci.yml", ".gitlab/ci/**/*.yml"];
#[derive(Debug, Clone)] pub struct GitlabCi { pub gitlab_hosts: Vec<String> }   // Default: ["gitlab.com"]
impl Plugin for GitlabCi { type Target = PinTarget; /* id "gitlab-ci"; scan = discover(text) ∩ on (Image→"image", Component→"component", IncludeRef→"include") */ }
// driver
pub async fn pin_files(paths: &[PathBuf], opts: &PinOptions, reg: &RegistryClient, gitlab: &GitLabClient) -> Result<Report, String>;
//   ctx = Ctx::new(UpdatePolicy{ enabled: opts.update, allow_major: opts.update_major,
//                  min_age_days: u32::try_from(opts.min_age_days).unwrap_or(u32::MAX) },
//                  now_unix(), ResolverEnv::from_process(&opts.gitlab_host))
//           .with_oci(Ok(reg.clone())).with_gitlab(Ok(GitLabHosts::single(&opts.gitlab_host, gitlab.clone())));
//   plugin = GitlabCi { gitlab_hosts: vec![opts.gitlab_host.clone()] }; on = KindSet::defaults(&KINDS);
//   ∀ path in order: rows.extend(pin_file(&plugin, &ctx, path, &on, &RunOptions{dry_run: opts.dry_run, check: false,
//                    concurrency: opts.concurrency}).await.map_err(|e| e.to_string())?); Ok(Report::new(rows))
```

Policy (behaviour of today's `lib.rs:344-460`, order preserved so no client is built before it is needed):
- component: `parts None` → Skipped `unparseable reference`; `host ∉ gitlab_hosts` → Skipped `host not enabled: {host}`; `is_commit_sha(version)` → Ok; `is_catalog_managed(version)` → Skipped `catalog-managed version`; `ctx.gitlab()` Err(e) → Unresolved e; `.get(host)` None → Unresolved `gitlab: no client for {host}`; `resolve` Err(e) → Unresolved `e.to_string()`; Ok(sha) → Pinned, edits `[replace(start, end, requote(token, "{host}/{project}/{name}@{sha}"))] ⧺ trailing_comment(text, line, end, version)`.
- include: `is_commit_sha(value)` → Ok; `value.contains('$')` → Skipped `templated reference`; `ctx.gitlab()?.primary().resolve(project, value)`; Ok(sha) → Pinned, edits `[replace(.., requote(token, sha))] ⧺ trailing_comment(text, line, end, value)`.
- `Finding.reference` = `t.value()`, `old` = `text[start..end]`, kinds `"component"` / `"include"`.

---

## M5a — snapshot baseline (tests only)

Setup once: `git -c core.excludesFile=/dev/null switch -c m5-workspace-core main`.

### Task A1 — snapshot harness + snapshots of the existing fixtures

**Files:** Modify `Cargo.toml` (`[dev-dependencies] insta = "1"`), `Cargo.lock`. Create `tests/common/mod.rs`, `tests/fixture_snapshots.rs` (starts with `mod common;`, as every later test file using the harness), `tests/snapshots/fixture_snapshots__*.snap` (8 runs × 3 = 24 files).
**Produces:** the harness API above (all items except the checkers).

- [ ] RED: write `tests/fixture_snapshots.rs`:
  - `fixture_snapshots` (`#[tokio::test]`): `fakes = start_fakes()`; ∀ fx ∈ `fixtures()`, ∀ m ∈ `modes(&fx.name)`: `p = scratch_copy(&fx, m, "snap")`; `r = normalized(pin_once(&fakes, &p, m).await, &fx.name)`; `insta::assert_snapshot!(format!("{}__{}__human", fx.stem, m.tag()), redact(&r.render_human()))`; same with `__json` / `redact(&r.to_json())` and `__file` / `visible(&read_to_string(&p))`; remove the scratch dir.
  - `redact_masks_mock_ports`: `redact("error sending request for url (http://127.0.0.1:39373/v2/x)") == "error sending request for url (http://127.0.0.1:[PORT]/v2/x)"`; a string without the pattern is unchanged; `127.0.0.1:` without digits is unchanged.
  - `visible_marks_cr_and_missing_newline`: `visible("a\r\nb") == "a␍\nb\n[no newline at end of file]"`; `visible("a\n") == "a\n"`.
- [ ] Run `cargo test --test fixture_snapshots` → FAIL: new snapshots (insta prints `+new results`).
- [ ] GREEN: implement `tests/common/mod.rs` per the snippet; `INSTA_UPDATE=always cargo test --test fixture_snapshots`; `rm -f tests/snapshots/*.snap.new`.
- [ ] Review every `.snap`: each pin-mode file is fully pinned except `ci-e2e`'s `mystery:1.0` (`unresolved (library/mystery:1.0 not found)`); update-mode files show `alpine:3.20 → alpine:3.24@sha256:…` rows `updated`; no PID path, no `127.0.0.1:<digits>` anywhere (`grep -rE '127\.0\.0\.1:[0-9]' tests/snapshots` → empty).
- [ ] Planted: (a) change `render_human`'s `"{}:{}  {}"` to `"{}:{} {}"` → `fixture_snapshots` FAILS ☐; revert. (b) create `tests/fixtures/zz-planted.yml` = `image: quay.io/x/y:1\n` → panics `unmapped registry host quay.io` ☐; delete it.
- [ ] Check (gate) → passed 153. Commit: `test: M5a fixture snapshot harness (insta) + baseline snapshots`.

### Task A2 — byte-exact and idempotence properties

**Files:** Modify `tests/common/mod.rs` (`Insertion`, `line_diff`, `check_byte_exact`, `check_idempotent`). Create `tests/fixture_properties.rs`.
**Consumes:** A1 harness.

- [ ] RED: tests (each collects all failures as `"{name}/{mode}: {msg}"` and asserts the list is empty):
  - `byte_exact_over_all_fixtures`: ∀ fx, m: run once on a scratch copy; `check_byte_exact(&fx.text, &e1, &r1)` is `Ok`.
  - `idempotent_over_all_fixtures`: ∀ fx, m: run twice on the same copy; `check_idempotent(&r1, &e1, &r2, &e2)` is `Ok`.
  - `checker_rejects_flipped_byte` (no I/O; rows built by a local `fn row(line, old, new) -> RefResult` so C4 edits one place): `o = "a: x\nb: y\n"`, row `{line 1, old "x", new "x@sha256:1", status pinned}`; `e = "a: x@sha256:1\nb: Y\n"` → `Err`; `e = "a: x@sha256:1\nb: y\n"` → `Ok`; `e = "a: x@sha256:1  # c\nb: y\n"` → `Ok` with `Some(Insertion{at: 4, text: "  # c"})` on line 0; a row whose `new` is absent from the line → `Err`.
- [ ] Run → FAIL (checker functions missing: compile error).
- [ ] GREEN: implement the checkers per the snippet.
- [ ] Run → PASS for all 8 fixture/mode runs.
- [ ] Planted: (a) in `src/edit.rs::apply_splices`, prepend `out.insert(0, 'X')` when splices are non-empty → `byte_exact_over_all_fixtures` FAILS ☐; revert. (b) in `src/lib.rs::resolve_image`, make the `r.digest == Some(digest)` branch fall through to pinning → `idempotent_over_all_fixtures` FAILS ☐; revert.
- [ ] Check → passed 156. Commit: `test: M5a byte-exact + idempotence properties over all fixtures`.

### Task A3 — new fixtures (R1, R2, anchors, components, image edge cases) + tag

**Files:** Create `tests/fixtures/ci-r1-flow-include.yml`, `ci-r2-crlf.yml`, `ci-anchors.yml`, `ci-components.yml`, `ci-images-edge.yml`; 15 new `.snap` files.

Exact bytes (`ci-r2-crlf.yml` must be written with `printf`; `ci-components.yml` has **no** final newline):

```text
ci-r1-flow-include.yml:  include: [{ project: g/a, ref: main }, { project: g/b, ref: dev }]\n
ci-r2-crlf.yml:          component: gitlab.com/g/p/name@1.0.0\r\nimage: alpine:3.20\r\ninclude: { project: g/p, ref: main }\r\n
                         include:\r\n  - project: g/p\r\n    ref: dev\r\njob:\r\n  services:\r\n    - redis:7\r\n  image: "node:20"\r\n
ci-anchors.yml:          image: &img alpine:3.20 / .base: &base / "  image: ubuntu:24.04" / job: / "  <<: *base" / "  image: *img" /
                         other: / "  services:" / "    - &redis redis:7" / "    - *redis"            (one item per line, LF)
ci-components.yml:       component: gitlab.com/g/p/name@1.0.0 / include: / "  - project: g/p" / "    ref: main" / "  - project: g/p" /
                         "    ref: v1.2.3" / "  - project: g/missing" / "    ref: main" / "  - project: g/p" / "    ref: $CI_COMMIT_REF_NAME" /
                         "  - project: g/p" / "    ref: e3262fdd0914fa823210cdb79a8c421e2cef79d8" / "  - project: g/p" / "    ref: v2.0.0"   (no final \n)
ci-images-edge.yml:      image: alpine / stale: / "  image: alpine:3.20@sha256:" + 64×"0" / templated: / "  image: ${APP_IMAGE}:${CI_COMMIT_SHORT_SHA}" /
                         quoted: / "  image: 'redis:7'" / "  services: [\"docker:24\", 'redis:7']"
```

- [ ] RED: add the five files; `cargo test --test fixture_snapshots` → FAIL (15 new snapshots); properties stay green (`cargo test --test fixture_properties` → PASS: today's output is byte-exact and idempotent even where wrong).
- [ ] GREEN: `INSTA_UPDATE=always cargo test --test fixture_snapshots`; delete `*.snap.new`.
- [ ] Review the recorded **wrong** output (scratch-verified prediction):
  - `ci-r1-flow-include__pin__file`: `include: [{ project: g/a, ref: <sha> }, { project: g/b, ref: <sha> }]  # dev  # main` (R1).
  - `ci-r2-crlf__pin__file`: line 1 `component: …@<sha>␍  # 1.0.0` (comment inside `\r\n`, R2); line 3 flow include pinned **without** comment (R2); `ref: dev`, `redis:7`, `"node:20"` untouched and absent from the report (R8).
  - `ci-anchors__pin__human`: 4 rows, all `skipped (unparseable reference)`: `&img alpine:3.20`, `*img`, `&redis redis:7`, `*redis` (R7); `ubuntu:24.04` under `.base: &base` absent (R8).
  - `ci-components__pin__*`: pinned `component`, `main`, `v1.2.3`, `v2.0.0` (comment at EOF, then `[no newline at end of file]`); `unresolved (g/missing:main not found (no tag or branch))`; `skipped (templated reference)`; `ok` for the literal SHA.
  - `ci-images-edge__pin__*`: `alpine` → `alpine@sha256:…`; stale digest replaced; templated `skipped (unparseable reference)`; quotes kept on all three quoted refs.
- [ ] Check → passed 156. Commit: `test: M5a fixtures for R1, R2 (CRLF), anchors, components, image edge cases (records current output)`.
- [ ] Tag: `git -c tag.gpgSign=false tag pre-pin-toolkit` (lightweight, on this commit; not pushed).

## M5b — workspace move

### Task B1 — `git mv` into `crates/pin-gitlab-ci/`, root workspace, oracle path

**Files:** Modify `tests/oracle.rs:21` (+1 test). Move `src/` → `crates/pin-gitlab-ci/src/`, `tests/` → `crates/pin-gitlab-ci/tests/`, `Cargo.toml` → `crates/pin-gitlab-ci/Cargo.toml`. Create root `Cargo.toml`. `Cargo.lock` stays at the root.

- [ ] Step 1 (commit 1): add to `tests/oracle.rs` the un-gated `#[test] fn oracle_repo_root_has_script() { assert!(Path::new(REPO).join("pin-ci-images.sh").is_file(), "REPO={REPO}") }` → PASS today. Gate → 157. Commit: `test: oracle repo-root guard`.
- [ ] Step 2: `git mv src crates/pin-gitlab-ci/src && git mv tests crates/pin-gitlab-ci/tests && git mv Cargo.toml crates/pin-gitlab-ci/Cargo.toml`; write the root manifest (B1 part of the snippet); package keeps `name = "pin-ci"`, switches to `version.workspace = true`, `edition.workspace = true`.
- [ ] RED: gate → `oracle_repo_root_has_script` FAILS (REPO now `crates/pin-gitlab-ci`) ☐ — everything else green.
- [ ] GREEN: `oracle.rs:21` → `const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");` (doc comment: "repo root: two levels above the crate").
- [ ] Gate → passed 157, `git status` shows no `.snap` change and no `.snap.new` (insta resolves `crates/pin-gitlab-ci/tests/snapshots/`; metadata `source:` paths are not compared — verified in scratch). `git show --stat -M` lists the moves as renames; `Cargo.lock` unchanged or path-only diff.
- [ ] Commit: `refactor: M5b move pin-ci into a cargo workspace (crates/pin-gitlab-ci)`.

## M5c — pin-core split

### Task C1 — move-only: five modules and five test files into `crates/pin-core`

**Files:** `git mv` `crates/pin-gitlab-ci/src/image_ref.rs` → `crates/pin-core/src/oci/image_ref.rs`; `registry.rs` → `crates/pin-core/src/oci/registry.rs`; `gitlab.rs` → `crates/pin-core/src/gitlab.rs`; `versioning.rs` → `crates/pin-core/src/ver/docker.rs`; `report.rs` → `crates/pin-core/src/report.rs`; tests `mock_registry.rs`, `registry_hosts.rs`, `hub_api.rs`, `gitlab_client.rs`, `live_gitlab.rs` → `crates/pin-core/tests/`. Create `crates/pin-core/{Cargo.toml, src/lib.rs, src/oci/mod.rs, src/ver/mod.rs}`. Modify root `Cargo.toml` (`[workspace.dependencies]`), `crates/pin-gitlab-ci/Cargo.toml`, `crates/pin-gitlab-ci/src/lib.rs:1-7`.

- [ ] pin-core `lib.rs`: `pub mod oci; pub mod report; pub mod ver; #[cfg(feature = "gitlab")] pub mod gitlab;` plus `#[cfg(feature = "oci")] pub(crate) use oci::image_ref; // C1 alias: moved registry.rs says crate::image_ref; removed in C3`. `oci/mod.rs`: `pub mod image_ref; #[cfg(feature = "oci")] pub mod registry;`. `ver/mod.rs`: `pub mod docker;`.
- [ ] Only content edits inside moved files: `registry.rs:852` `pub(crate) fn is_old_enough` → `pub fn` (now used cross-crate). Moved tests: `use` lines only — `pin_ci::registry` → `pin_core::oci::registry`, `pin_ci::image_ref` → `pin_core::oci::image_ref`, `pin_ci::gitlab` → `pin_core::gitlab` (incl. the inline paths in `live_gitlab.rs:101,122,147`).
- [ ] pin-gitlab-ci `lib.rs:1-7` → `pub mod discover; pub mod edit;` + `pub use pin_core::{gitlab, report}; pub use pin_core::oci::{image_ref, registry}; pub use pin_core::ver::docker as versioning;` (keeps every `crate::…`/`pin_ci::…` path valid; removed in C10). Manifests per snippet; gitlab-ci keeps its other deps for now.
- [ ] Gate + feature matrix → passed 157 (unit tests now split across two crates; same total). Planted: drop `#[cfg(feature = "oci")]` from `pub mod registry` → `--no-default-features` clippy FAILS ☐; restore. Run `cargo test -p pin-core` (no features) once and note the lower count — the reason the gate uses `--workspace --all-features` ☐.
- [ ] `git show --stat -M HEAD` after commit: 10 renames, similarity ≥ 95 %.
- [ ] Commit: `refactor: M5c move image_ref, registry, gitlab, versioning, report into pin-core (move-only)`.

### Task C2 — rename package `pin-ci` → `pin-gitlab-ci`, keep `[[bin]] pin-ci`

**Files:** `crates/pin-gitlab-ci/Cargo.toml`, `Cargo.lock`, `crates/pin-gitlab-ci/src/main.rs:4`, `crates/pin-gitlab-ci/tests/*.rs` (`pin_ci::` → `pin_gitlab_ci::`, mechanical), `README.md` (Tests section: `cargo test` → `cargo test --workspace --all-features`; `--test live` → `-p pin-gitlab-ci --test live`; `--test oracle` → `-p pin-gitlab-ci --test oracle`; add `PIN_CI_LIVE=1 cargo test -p pin-core --all-features --test live_gitlab`).

- [ ] RED: add to the gate for this task: `cargo build -p pin-gitlab-ci && test -x target/debug/pin-ci && test ! -e target/debug/pin-gitlab-ci` → FAILS before the rename (`-p pin-gitlab-ci` unknown) ☐.
- [ ] GREEN: `name = "pin-gitlab-ci"`, `[[bin]] name = "pin-ci", path = "src/main.rs"`; `rm -f target/debug/pin-gitlab-ci` if a stale one exists; sed the crate path.
- [ ] Planted: remove `[[bin]]` → `target/debug/pin-gitlab-ci` appears, `pin-ci` absent → check FAILS ☐; restore.
- [ ] Gate → 157. Commit: `refactor: M5c rename package pin-ci → pin-gitlab-ci (bin pin-ci kept until M5d)`.

### Task C3 — pin-core paths, `ver::age`, `gitlab::is_commit_sha`

**Files:** `crates/pin-core/src/{lib.rs, oci/mod.rs, oci/registry.rs, ver/mod.rs, gitlab.rs}`, create `crates/pin-core/src/ver/age.rs`; `crates/pin-gitlab-ci/src/lib.rs` (lines 18-24, 108-111 and the two `is_sha` calls).

- [ ] RED: unit `is_commit_sha_table` in `gitlab.rs`: true for `"e3262fdd0914fa823210cdb79a8c421e2cef79d8"`; false for the same uppercased, 39 and 41 chars, a `g` inside, `""` → fails (function missing).
- [ ] GREEN: `gitlab.rs:335` `fn is_sha` → `pub fn is_commit_sha` (update its call site); delete gitlab-ci `lib.rs:108-111` `is_sha`, call `pin_core::gitlab::is_commit_sha`. Move `days_in_month`, `civil_to_days`, `parse_rfc3339_unix` (now `pub`), `is_old_enough` from `registry.rs:758-855` to `ver/age.rs` with their tests `rfc3339_parse_table`, `is_old_enough_table` (moved verbatim, reason: travel with the function); `registry.rs` imports `crate::ver::age::parse_rfc3339_unix`; gitlab-ci lib.rs imports `pin_core::ver::age::is_old_enough`. Remove the C1 crate-root alias; `registry.rs:21` → `use crate::oci::image_ref::ImageRef;`. `oci/mod.rs`: `pub use image_ref::ImageRef; #[cfg(feature = "oci")] pub use registry::RegistryClient;`.
- [ ] Gate + matrix → 158. Commit: `refactor: M5c pin-core paths, ver::age, gitlab::is_commit_sha (dedup is_sha)`.

### Task C4 — `Status` enum and `Row` (rename of `RefResult`)

**Files:** `crates/pin-core/src/report.rs`; `crates/pin-gitlab-ci/src/lib.rs` (`base_result` → `Row` with `plugin: "gitlab-ci"`, `kind` from the target; every `status = "…".to_string()` → `Status::…`); `crates/pin-gitlab-ci/tests/common/mod.rs` (type name only); `crates/pin-gitlab-ci/tests/fixture_properties.rs` (the `row(line, old, new)` helper from A2 builds a `Row` — struct changed).

- [ ] RED: unit `status_serde_display_and_str_eq`: `serde_json::to_string(&Status::Unresolved) == "\"unresolved\""` for all five; `Status::Pinned.to_string() == "pinned"`; `Status::Ok == "ok"`; `Status::Ok != "skipped"`; `Status::Updated.as_str() == "updated"`.
- [ ] GREEN per the interface snippet. `Report::new` counts by enum. The two report unit tests (`human_render_exact`, `json_render_roundtrip`) get `plugin`/`kind` fields and `Status::…` values — changed because the struct changed; expected strings unchanged. No e2e test changes (P7).
- [ ] Planted: `rename_all = "UPPERCASE"` → `status_serde…` and every `__json` snapshot FAIL ☐; revert.
- [ ] Gate → 159; snapshots unchanged. Commit: `refactor: M5c report Status enum + Row (plugin/kind not serialised until M5d)`.

### Task C5 — core `edit` (`Splice`, `apply`, `requote`, `trailing_comment`) and `oci` image splices

**Files:** create `crates/pin-core/src/edit.rs`, `crates/pin-core/src/oci/splice.rs`; delete `crates/pin-gitlab-ci/src/edit.rs`; modify gitlab-ci `lib.rs` (call sites `:134, :322, :333, :407, :454, :553`).

- [ ] Move tests (reason: functions move; core has no `discover`): 7 image-splice tests → `oci/splice.rs` verbatim; `splice_replace_preserves_quotes` → `requote_preserves_quotes`; `apply_splices_inserts_and_replaces` → `apply_inserts_and_replaces` (same text, `Splice` values, `apply` returns `String`); `comment_offset_{bare_line,existing_comment,flow_line}` → `trailing_comment_*` asserting `Some(Splice::insert(11, "  # main"))` / `None` / `Some(Splice::insert(36, "  # main"))`; the 3 discover-based tests → literal spans from `text.find(..)`: `apply_multiple_spans_end_first`, `apply_equal_value_is_identity`, `apply_preserves_trailing_comment` (same expected strings).
- [ ] RED (new): `apply_equal_start_insertions_reverse_today`: `apply("x\n", &[insert(1, "  # a"), insert(1, "  # b")]) == "x  # b  # a\n"` — characterises R1, flipped in C11. Fails (no core `apply`).
- [ ] GREEN: `apply` = today's `apply_splices` algorithm (sort by `Reverse(start)`, stable, `replace_range`); `trailing_comment` = today's `comment_insert_offset` (`text.lines()` sums — R2 kept, doc: "R2: fixed in C12") mapped to `Splice::insert(pos, format!("  # {orig}"))`; `requote` = `splice_replace`. gitlab-ci call sites 1:1: `comment_insert_offset(..).map(|pos| (pos, …))` → `trailing_comment(..).map(|s| (s.start, s.text))`; `splice_update`/`splice_value` → `pin_core::oci::…`; `splice_replace` → `requote`; `apply_splices(&text, &v).0` → `apply(&text, &v.into_iter().map(|(s, e, t)| Splice::replace(s, e, t)).collect::<Vec<_>>())`.
- [ ] Gate + matrix → 160; snapshots unchanged. Commit: `refactor: M5c core edit (Splice, apply, requote, trailing_comment) + oci image splices`.

### Task C6 — `plugin` + `run::pin_file` (no resolvers yet)

**Files:** create `crates/pin-core/src/plugin.rs`, `crates/pin-core/src/run.rs` (`Ctx` with `update`, `now_unix`, `env` only; `now_unix()` copied from gitlab-ci `lib.rs:139` — gitlab-ci switches to it in C10), `crates/pin-core/tests/run_pin_file.rs`. `join_all` copied verbatim from gitlab-ci `lib.rs:469-500` as `pub(crate)` (gitlab-ci copy deleted in C10).

Test fake (in `run_pin_file.rs`): `struct Fake { findings: Vec<Finding>, in_flight: Cell<usize>, max_in_flight: Cell<usize> }`, `Target = usize`, `scan` → `0..findings.len()` filtered by `on.contains("x")`, `resolve` → bump in_flight, `tokio::task::yield_now().await` ×3, record max, decrement, return `findings[t].clone()`. Files in `.tmp/run-pin-file-{test}-{pid}/`.

- [ ] RED (`#[tokio::test]` unless noted): `kindset_defaults_contains_set` (unit, `plugin.rs`): defaults take `default_on`, `set(id, false)` removes, `contains` on unknown id is false · `rows_sorted_one_based_with_plugin_and_kind`: findings at lines 3, 0 → rows lines 1, 4, `plugin "fake"`, `kind "x"` · `writes_only_when_edits`: set the file's mtime to `UNIX_EPOCH + 1000 s` (`File::set_modified`); findings without edits → bytes and mtime unchanged; one replacement → file = `apply` result · `dry_run_and_check_never_write`: `dry_run` or `check` with edits → bytes unchanged, rows still `pinned` · `partial_write_keeps_other_edits`: one `unresolved` finding without edits + one pinned → pinned edit written · `overlapping_edits_mark_both_unresolved`: replacements [5,10) and [8,12) from two findings → both `unresolved`, reason `internal: overlapping edits`, file unchanged; an insertion at 10 next to [5,10) is **not** an overlap · `read_error_is_file_error_with_path`: missing path → `Err(FileError::Read{path, ..})`, `to_string()` starts with the path · `plan_bounds_concurrency`: 5 targets, concurrency 2 → `max_in_flight == 2` · `plan_concurrency_zero_is_one`: concurrency 0 → `max_in_flight == 1`.
- [ ] GREEN per the snippets (`pin_file` steps 1–6 without coalesce).
- [ ] Planted: (a) Semaphore of `usize::MAX >> 3` → `plan_bounds_concurrency` FAILS ☐; (b) delete the overlap step → `overlapping_edits…` FAILS ☐; (c) ignore `o.check` → `dry_run_and_check_never_write` FAILS ☐. Revert each.
- [ ] Gate + matrix → 169. Commit: `feat: M5c pin-core Plugin/DynPlugin + run::pin_file (bounded join, overlap rule)`.

### Task C7 — lazy, feature-gated resolvers in `Ctx` (D36)

**Files:** `crates/pin-core/src/run.rs`, `crates/pin-core/src/gitlab.rs` (`GitLabHosts`), create `crates/pin-core/tests/lazy_resolvers.rs` (`required-features = ["oci", "gitlab"]`).

- [ ] RED: `no_resolver_needed_builds_none`: `Ctx::new(Default::default(), 0, ResolverEnv { home: None, gitlab_host: "gitlab.com".into() })`; run `pin_file` with a fake whose findings never touch `ctx` → `resolvers_built() == Vec::<&str>::new()` · `oci_target_builds_only_oci`: a fake whose `resolve` calls `ctx.oci()` once → `["oci"]` · `construction_error_is_cached`: `ctx.with_oci(Err("ca: unreadable".into()))` → `ctx.oci() == Err("ca: unreadable")` twice, `resolvers_built() == ["oci"]`.
- [ ] GREEN per the interface snippet: `OnceCell::get_or_init` with the builders; `resolvers_built` lists initialised cells in the order oci, gitlab; `pub fn env()`.
- [ ] Planted: initialise both cells in `Ctx::new` → `no_resolver_needed_builds_none` FAILS ☐; revert.
- [ ] Gate + matrix (the `--features oci` / `--features gitlab` legs now compile the cfg'd fields separately) → 172. Commit: `feat: M5c lazy feature-gated resolvers in Ctx (D36)`.

### Task C8 — `oci::decide_image` + `pin_inline_image`

**Files:** create `crates/pin-core/src/oci/decide.rs`, `crates/pin-core/tests/oci_decide.rs` (`required-features = ["oci"]`; wiremock as in `tests/e2e.rs:15-106`, client via `Ctx::new(..).with_oci(Ok(RegistryClient::new(Default::default(), {docker.io, ghcr.io → mock})))`).

`decide_image` = today's `resolve_image` (`lib.rs:165-341`) from the `digest` call on, with `ctx.oci()` (Err → `Err(e.to_string())`), `ctx.update` for `opts.update`/`update_major`/`min_age_days` (`u64::from`), `ctx.now_unix` for `now_unix()`; guard condition unchanged (`min_age_days > 0 ∧ r.is_docker_hub()`). `pin_inline_image`: parse first (Err → Skipped `unparseable reference`, no client), then map `ImageDecision` → `Finding` (`Pin` → `splice_value`, `Update` → `splice_update`, one `Splice::replace(start, end, token)`; `Current` → Ok; `Skip(r)` → Skipped r; `Err(m)` → Unresolved m). The old `resolve_image` stays in gitlab-ci until C10 (one-commit duplication, deleted there).

- [ ] RED: `decide_pin_unpinned` (alpine:3.20 → `Pin{D1}`) · `decide_current_when_digest_matches` · `decide_update_bumps_to_latest_stable` (tags 3.16/3.20/3.24, enabled → `Update{"3.24", D2}`) · `decide_tag_list_error_is_prefixed` (tags/list 500 → `Err` starting `tag list: `) · `decide_digest_error_is_inner_message` (`mystery:1.0` 404 → `Err("library/mystery:1.0 not found")`) · `inline_unparseable_builds_no_client` (value `${X}:${Y}` on a ctx without presets → Skipped, `resolvers_built()` empty) · `inline_pin_single_splice_keeps_quotes` (text `image: "alpine:3.20"\n`, span 7..20 → `edits == [Splice::replace(7, 20, "\"alpine:3.20@{D1}\"")]`, `new` same) · `inline_construction_error_is_unresolved` (`with_oci(Err("ca: x"))` → Unresolved, reason `ca: x`).
- [ ] GREEN as above.
- [ ] Planted: drop the `tag list: ` prefix → its test FAILS ☐; revert.
- [ ] Gate + matrix → 180. Commit: `feat: M5c oci::decide_image + pin_inline_image (image policy in core)`.

### Task C9 — `GitlabCi: Plugin` + `policy.rs`

**Files:** `crates/pin-gitlab-ci/src/lib.rs` (add `KINDS`, `DEFAULT_FILES`, `GitlabCi`, `impl Plugin`), create `crates/pin-gitlab-ci/src/policy.rs` (new `&Ctx`-based `resolve_component`, `resolve_include`; `is_catalog_managed` moves here with its test in C10), create `crates/pin-gitlab-ci/tests/plugin.rs` (uses `tests/common`).

- [ ] RED: `scan_filters_by_kind` (pure): `ci-rich.yml` with `on = defaults` → 7 targets; with `include` off → 6, none `IncludeRef`; with `image` off → 1 · `ids_kinds_and_files`: `id() == "gitlab-ci"`, kind ids `["image", "component", "include"]` all `default_on`, `default_files() == DEFAULT_FILES` · `component_other_host_builds_no_client`: `component: example.org/g/p/c@1.0.0` → Skipped `host not enabled: example.org`, `resolvers_built()` empty · `include_pin_has_replacement_and_comment`: fakes + `include:\n  project: g/p\n  ref: main\n` → Pinned, `edits == [replace(span, sha), insert(line_end, "  # main")]`.
- [ ] GREEN per the gitlab-ci snippet and policy list.
- [ ] Planted: make `scan` ignore `on` → `scan_filters_by_kind` FAILS ☐; revert.
- [ ] Gate + matrix → 184. Commit: `feat: M5c GitlabCi plugin (pure scan, policy on Ctx)`.

### Task C10 — switch to `pin_file` + driver; delete the old path (refactor)

**Files:** create `crates/pin-gitlab-ci/src/driver.rs`; rewrite `crates/pin-gitlab-ci/src/lib.rs` (delete `PinOptions`…`exit_code` and the old resolve path `:26-575`; `is_catalog_managed_table` → `policy.rs`); `crates/pin-core/src/oci/registry.rs` + `gitlab.rs` (P11: `#[derive(Clone)]`, `tokens`/`tag_lists`/`cache` → `Arc<Mutex<…>>`, `Creds: Clone`); gitlab-ci `Cargo.toml` (drop deps no longer used: reqwest, sha2, serde; serde_json → dev-dep if unused in src); tests' `use` lines (table below); `main.rs` unchanged.

| Old path (tests, main) | New path |
|---|---|
| `pin_gitlab_ci::gitlab::…` | `pin_core::gitlab::…` |
| `pin_gitlab_ci::{ImageRef, image_ref::ImageRef}` | `pin_core::oci::ImageRef` |
| `pin_gitlab_ci::RegistryClient` | `pin_core::oci::RegistryClient` |
| `pin_gitlab_ci::versioning` | `pin_core::ver::docker` (alias `as versioning` in the `use`) |
| `pin_gitlab_ci::report::…` | `pin_core::report::…` |
| `pin_gitlab_ci::{PinOptions, pin_files, exit_code, client_from_env, gitlab_from_env}` | unchanged (driver re-export) |

- [ ] Evidence before: `INSTA_UPDATE=no cargo test --workspace --all-features` log saved as `.tmp/c10-before.log`.
- [ ] Do the switch per the driver snippet; remove the C1 re-exports from lib.rs.
- [ ] Evidence after: same passed count (184), zero snapshot/property change, `diff <(grep '^test ' .tmp/c10-before.log | sort) <(grep '^test ' .tmp/c10-after.log | sort)` lists only renamed module paths of moved unit tests. `git diff --stat` shows lib.rs shrinking by ≥ 400 lines.
- [ ] By hand (coverage gap 3): `cargo run -p pin-gitlab-ci --bin pin-ci -- --dry-run --json crates/pin-gitlab-ci/tests/fixtures/ci-simple.yml` → JSON report, no panic.
- [ ] Gate + matrix → 184. Commit: `refactor: M5c pin-gitlab-ci runs through pin_core::run::pin_file; pin_files kept as doc-hidden driver until M5d`.

### Task C11 — fix R1: one combined trailing comment per line, source order

**Files:** `crates/pin-core/src/edit.rs`, `crates/pin-core/src/run.rs` (step 4), `crates/pin-gitlab-ci/tests/common/mod.rs` + `fixture_properties.rs`, snapshot `ci-r1-flow-include__pin__file.snap`, `README.md`.

- [ ] RED: `coalesce_merges_same_offset_comments` (unit): `[insert(36, "  # main"), replace(0, 3, "abc"), insert(36, "  # dev"), insert(40, "  # x")]` → `[insert(36, "  # main, dev"), replace(0, 3, "abc"), insert(40, "  # x")]`; a same-offset insertion not starting with `"  # "` is left alone · `one_comment_per_line_over_all_fixtures`: ∀ insertions from `check_byte_exact`, `text.matches('#').count() == 1` → FAILS on `ci-r1-flow-include` ☐ · modify `apply_equal_start_insertions_reverse_today` → `apply_equal_start_insertions_keep_source_order` expecting `"x  # a  # b\n"` (the characterised bug is the fix's target) → FAILS ☐.
- [ ] GREEN: `coalesce_comments` (merge group = insertions at equal offset with text starting `"  # "`; merged text `"  # " + suffixes.join(", ")`; position of the first; other splices keep order). `apply` = forward build: copy, stable sort by `(start, end)`, `debug_assert!` non-overlap, concatenate `text[cursor..s.start] + s.text`, cursor = `s.end`. `pin_file` step 4 calls `coalesce_comments`.
- [ ] Snapshot: `INSTA_UPDATE=always cargo test --test fixture_snapshots`; `git diff` must touch **only** `ci-r1-flow-include__pin__file.snap`, line becomes `…, ref: <sha> }]  # main, dev`.
- [ ] README (Components & includes): "Several refs pinned on one line (e.g. a flow `include:` sequence) get **one** trailing comment with the originals in source order: `include: [{…, ref: <sha>}, {…, ref: <sha>}]  # main, dev` (fixed in M5c, R1; before: one comment per ref in reverse order)."
- [ ] Planted (spec §11): revert the `coalesce_comments` call → snapshot + `one_comment_per_line…` FAIL ☐; restore.
- [ ] Gate + matrix → 186. Commit: `fix: R1 one combined trailing comment per line in source order (snapshot updated)`.

### Task C12 — fix R2: `LineIndex`, comment before `\r\n`

**Files:** create `crates/pin-core/src/text.rs`; `crates/pin-core/src/edit.rs` (`trailing_comment`); `crates/pin-gitlab-ci/tests/common/mod.rs` + `fixture_properties.rs`; snapshot `ci-r2-crlf__pin__file.snap` (rows do not change, so human/json stay); `README.md`.

- [ ] RED: `line_index_lf_crlf_and_eof` (unit): text `"a\r\nbb\nc"` → `len 3`, starts `[0, 3, 6]`, ends `[1, 5, 7]`; `line_start(3) == None` · `trailing_comment_before_crlf` (unit): `trailing_comment("x: a\r\ny: b\r\n", 1, 10, "b") == Some(Splice::insert(10, "  # b"))` and line 0 → `insert(4, …)` · `crlf_preserved_over_all_fixtures` (P-crlf) and `pinned_ref_comments_present_over_all_fixtures` (P-comment-present) → both FAIL on `ci-r2-crlf` ☐.
- [ ] GREEN: `LineIndex::new` via `split_inclusive('\n')`, `ends` strip `"\n"` then one `"\r"`; `trailing_comment` uses `LineIndex` (`line_end(line)?`; same `#`-after-token rule on `text[token_end..line_end]`). Doc on `LineIndex`: "shared line model for edits and (M5c-y) `core::yaml`; `discover.rs` keeps its own split until M5c-y".
- [ ] Snapshot: accept; `git diff` touches only `ci-r2-crlf__pin__file.snap`: line 1 `component: …@<sha>  # 1.0.0␍`, line 3 `include: { project: g/p, ref: <sha> }  # main␍`.
- [ ] README: "CRLF files: the trailing comment goes before `\r\n` on every line (fixed in M5c, R2; before: inside `\r\n` on line 1, dropped on later lines). Known gap until M5c-y: block children in CRLF files are not discovered."
- [ ] Planted (spec §11): restore the `lines()` offset in `trailing_comment` → snapshot + both properties FAIL ☐; revert.
- [ ] Gate + matrix → 190. Commit: `fix: R2 CRLF-safe trailing comments via shared LineIndex (snapshot updated)`.

## Close-out (controller, before merge)

- [ ] Full gate + matrix green; `git log --oneline main..` = 17 commits in plan order; `git tag --points-at` A3 commit = `pre-pin-toolkit`.
- [ ] `cargo build --release -p pin-gitlab-ci` → `target/release/pin-ci`.
- [ ] Coverage gap 1: live + oracle runs if network/devenv are available; otherwise record "not run" in the ledger.
- [ ] Merge decision belongs to main/owner (unsigned merge, as M4a/M4b-1).

## Self-review

- **Spec coverage:** §9 M5a (snapshots A1/A3, redaction A1, properties A2, R1/R2/anchor fixtures A3, tag A3); M5b (B1 move, root manifest/lock, `oracle.rs:21`); M5c (move-only C1, `is_sha` dedup C3, `Status`/`Row` C4, `Splice`/edit C5, `Plugin`/`pin_file` C6, lazy `Ctx` C7, `decide_image`/`pin_inline_image` C8, `GitlabCi` C9, `pin_files` as helper C10, rename + `[[bin]] pin-ci` C2, discover verbatim, test split C1, R1 C11, R2 C12, README notes C11/C12). §11 rows mapped in the Gate table.
- **Smallest calls where spec and code differ (noted, not escalated):** (a) `UpdatePolicy.min_age_days` is `u32` (spec) while `--min-age-days` stays `u64` until M5d — driver converts with saturation; (b) `RegistryClient::new` cannot fail today, so the `Result` in the `OnceCell` is exercised only through `with_oci(Err(..))` until F3 adds CA files; (c) spec §6.2 lists `memo` in `Ctx` and `run(routes, ..)` — both belong to M5d per §9, omitted (P9); (d) `Row.plugin`/`kind` exist but are not serialised, because §9 requires unchanged M5c snapshots (P8); (e) `GitLabHosts` and `ResolverEnv` are not specified field-by-field in the spec — shapes chosen to carry exactly today's single-host semantics (P13); (f) `Status` gains `PartialEq<&str>` beyond §6.2 to keep existing tests verbatim (P7); (g) `UpdatePolicy` omits §6.2's `unknown_age` until M5d (main's call, P14).
- **Type consistency:** `Row` (not `RefResult`) from C4 on; `Splice::{replace, insert}` used identically in C5/C8/C9/C11/C12; `GitLabHosts::{single, primary, get}` in C7/C9/C10; `KindSet::defaults(&KINDS)` in C9/C10; `pin_file(p, ctx, path, on, o)` in C6/C7/C10.
- **Planted violations:** every new check has a box; C10 is a pure refactor and carries before/after evidence instead.
- **Proportion:** bodies appear only for the fake-server rules, `line_diff`, `pin_file`, the overlap rule and `apply`'s forward build — algorithms the signatures and tests do not determine.

## Decisions by main (2026-11-10; formerly open questions)

1. **R8:** not fixed in M5c (no change to hand-rolled discover code that M5c-y deletes); recorded in the A3 snapshots and in spec §9 (row R8).
2. **`Finding.follow_up`:** `pin_file -> Result<Vec<Row>, FileError>` stays as specified; widen the return when M8b needs follow-ups.
3. **`Row.plugin`/`kind` in output:** M5d.
4. **`AgePolicy::Block`:** dropped from M5c (P14); arrives with the config switch.

## Handoff note for M5c-y (discovery parity)

The parity test (spec §11) compares parser discovery with `discover.rs`. These fixture cases are **expected to differ** and must be listed as documented exceptions, each with its reviewed snapshot diff:

| Bug | Fixture (A3) | `discover.rs` today | Parser discovery (expected) |
|---|---|---|---|
| R7 | `ci-anchors.yml` lines 1, 9 | targets `&img alpine:3.20`, `&redis redis:7` (anchor inside the value) → `skipped (unparseable reference)` | targets `alpine:3.20`, `redis:7` (properties skipped) → pinned |
| R7 | `ci-anchors.yml` lines 6, 10 | aliases `*img`, `*redis` are targets → skipped | aliases are not targets (no row) |
| R8 | `ci-anchors.yml` line 3 | `image: ubuntu:24.04` under `.base: &base` not discovered | discovered → pinned |
| R8 | `ci-r2-crlf.yml` lines 5–10 | `include:\r` block (`ref: dev`), `services:\r` block (`redis:7`), job-nested `image: "node:20"` not discovered | discovered → pinned (comments before `\r\n`, per C12) |
