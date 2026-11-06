# M3 plan — gitlab.com components & includes pinning (pin-only)

- Date: 2026-11-05
- Source of truth: `docs/superpowers/specs/2026-11-01-pin-ci-design.md` (M3 milestone, pin-only ruling from spec commit d3e90e5) + recon `.superpowers/sdd/2026-11-05-pin-ci-m3-components/recon-gitlab-components.md` (docs citations, endpoints, auth, error shapes)
- Base: `main` (post-M2 merge 543d56a). Branch: `m3-components`.

## Goal

Extend `pin-ci` so that, in the same run as image pinning, it resolves and pins:

1. **New-style component references** — top-level `component: <host>/<group.../project>/<name>@<version>` — replacing `<version>` with the full 40-hex commit SHA (GitLab's highest-precedence version form and its documented "preferred" security pin).
2. **Classic includes with an explicit `ref`** — `include:` mappings (block single mapping, block sequence, flow) carrying `project` + `ref` in any key order — replacing the `ref` value with the commit SHA.

Pin-only: no `--update` for components/includes (M4). Already-SHA refs → `ok`. Everything else per M1/M2 status and exit-code rules.

## Architecture

| Module | Change | Task |
|---|---|---|
| `src/discover.rs` | `Target` struct → `PinTarget` enum (`Image`, `Component`, `IncludeRef`); existing image scanner maps into `PinTarget::Image`; new component scanner (top-level `component:`); new include scanner (top-level `include:` forms) | T1, T2 |
| `src/gitlab.rs` (new) | `GitLabClient`: ref→commit SHA via GitLab REST v4 (tag/branch endpoints), SHA short-circuit, per-`(project, ref)` cache, PAT/job-token auth, tag-then-branch fallback for bare names, job-token branch via list+search | T3 |
| `src/edit.rs` | `splice_replace` (quote-preserving value swap), `apply_splices` (generic `(start, end, new)` splices; existing `apply` delegates), `comment_insert_offset` (trailing-comment placement rules) | T4 |
| `src/lib.rs` | `PinOptions.gitlab_host`; `gitlab_from_env(host)`; `pin_file`/`pin_files` take a `GitLabClient`; `resolve_component` + `resolve_include`; splice collection (replacements + comment insertions) | T4 |
| `src/main.rs` | `--gitlab-host` flag (default `gitlab.com`); both clients constructed and passed to `pin_files` | T5 |
| `tests/live_gitlab.rs` (new) | Opt-in live test: real public gitlab.com tag + branch, cross-checked against `git ls-remote` | T6 |
| `README.md` | M3 section | T5 |

No new dependencies (wiremock already a dev-dep; URL encoding hand-rolled, ~6 lines).

## Global constraints

1. **Pin-only.** Component/include versions are never bumped; `--update`/`--update-major` apply to images only (spec ruling, d3e90e5).
2. **Hosts, day one.** Component pinning enabled when the reference's host equals `opts.gitlab_host` (default `gitlab.com`, `--gitlab-host` for self-hosted). Other hosts → `skipped (host not enabled: <host>)`. Classic includes: `project` is a project path on the same instance — always eligible (no host in the reference).
3. **Byte-exact edits.** Only value spans are replaced; all comments/whitespace otherwise untouched. Idempotency: second run yields all `ok`, zero writes.
4. **Original ref stays visible as a trailing comment** (recon: spec comment format). Placement: block `ref:` line → after the value; component line → after the value; flow include → after the closing bracket (a `#` inside the flow would eat the `}`/`]`). If the line already has a trailing comment (a `#` after the token in the same line), no comment is added.
5. **Statuses / exit codes (M1 rule, extended).** `pinned` (new SHA written), `ok` (already a SHA / no change), `skipped (…)` (host not enabled, catalog-managed version, templated reference, unparseable reference, no explicit ref), `unresolved (…)` (GitLab API 404 for tag AND branch, 401/403, 429, network). Exit 1 iff any `unresolved`; skipped keeps exit 0.
6. **Auth.** `GITLAB_TOKEN` (sent as `PRIVATE-TOKEN`) takes precedence over `CI_JOB_TOKEN` (sent as `JOB-TOKEN`); neither → anonymous (public projects work). When `CI_JOB_TOKEN` is the token, branch resolution MUST use the list endpoint `GET /projects/:id/repository/branches?search=<name>` + exact-name match (the single-branch endpoint is not in the job-token allowlist, per recon §4); PAT/anonymous use the single-branch endpoint.
7. **Resolution flow (recon §6).** 40-hex SHA (lowercase) → `ok`, no API call. `refs/tags/<t>` → tag endpoint. `refs/heads/<b>` → branch. Bare name → tag first (GitLab tag-precedence semantics), 404 → branch, 404 → `unresolved (<host>:<project>:<ref> not found (no tag or branch))`.
8. **Catalog-managed component versions** are skipped, not resolved: version starting with `~` (e.g. `~latest`), numeric-only (`^\d+(\.\d+)*$`), or containing `.x` — these resolve via the CI/CD Catalog at pipeline time (recon §1.4) and have no tag/branch to pin against. Reason: `catalog-managed version`. (Documented edge: a purely-numeric real tag like `2024` is skipped as catalog.)
9. **No 429 retry** day one — report `unresolved (gitlab api: rate limited (429), retry later)`.
10. **Unchanged:** M1/M2 image behavior, statuses, formats, the old pipeline files, report line format (`{file}:{line}  {status}{ (reason)}  {old} -> {new|-}  # {reference}`), JSON schema.
11. **Discovery stays syntactic** (M1 ruling): candidates that fail semantic checks (templated `$…`, unparseable) remain targets and surface as `skipped` at resolve time — the report stays complete.
12. Env/commits: `CARGO_HOME=$PWD/.cargo-home`, rust at `/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin`, unsigned commits (`git -c commit.gpgsign=false`), `git -c core.excludesFile=/dev/null` on adds (nono workaround, session ruling).

## Files

| File | T1 | T2 | T3 | T4 | T5 | T6 |
|---|---|---|---|---|---|---|
| `src/discover.rs` | R | R | | | | |
| `src/edit.rs` | R (PinTarget) | | | R | | |
| `src/gitlab.rs` | | | A | | | |
| `src/lib.rs` | R | R (compile) | | R | | |
| `src/main.rs` | R (compile) | | | R (gitlab_host default) | R | |
| `tests/e2e.rs` (or new `tests/e2e_components.rs`) | | | | A | | |
| `tests/live_gitlab.rs` | | | | | | A |
| `tests/fixtures/ci-include.yml` | | A | | | | |
| `README.md` | | | | | R | |
| `Cargo.toml` | | | (none expected) | | | |

## Interfaces (cross-task contract)

```rust
// src/discover.rs
pub enum PinTarget {
    Image { line: usize, start: usize, end: usize, value: String },
    Component {
        line: usize, start: usize, end: usize,
        value: String,
        parts: Option<ComponentParts>,   // None → skipped (unparseable reference) at resolve time
    },
    IncludeRef {
        line: usize, start: usize, end: usize,   // span of the ref VALUE token
        value: String,                            // ref value, quotes stripped
        project: String,                          // project path, quotes stripped
        flow: bool,                               // true when the item is a flow mapping
    },
}
pub struct ComponentParts { pub host: String, pub project: String, pub name: String, pub version: String }
impl PinTarget {
    pub fn line(&self) -> usize;
    pub fn start(&self) -> usize;
    pub fn end(&self) -> usize;
    pub fn value(&self) -> &str;
}
pub fn discover(text: &str) -> Vec<PinTarget>   // sorted by (line, start), as before

// src/gitlab.rs
pub struct GitLabClient { /* base_url, token: Option<String>, job_token: bool, http, cache */ }
pub enum GitLabError { Unresolved(String) }     // Display = the string
impl GitLabClient {
    pub fn new(base_url: String, token: Option<String>, job_token: bool) -> Self; // 30s timeout
    pub fn from_env(base_url: String) -> Self;   // GITLAB_TOKEN > CI_JOB_TOKEN
    pub async fn resolve(&self, project: &str, ref_name: &str) -> Result<String, GitLabError>;
}
fn url_encode(s: &str) -> String;                // RFC3986: pass A-Z a-z 0-9 - _ . ~, else %XX (upper hex)

// src/edit.rs (additions)
pub fn splice_replace(old_token: &str, new_value: &str) -> String;   // quote-preserving
pub fn apply_splices(text: &str, splices: &[(usize, usize, String)]) -> (String, usize);
pub fn comment_insert_offset(text: &str, line: usize, token_end: usize) -> Option<usize>;

// src/lib.rs (signature changes)
pub struct PinOptions { /* + pub gitlab_host: String */ }
pub fn gitlab_from_env(host: &str) -> GitLabClient;   // base https://{host}/api/v4
pub async fn pin_file(path: &Path, opts: &PinOptions, reg: &RegistryClient, gitlab: &GitLabClient) -> Result<Vec<RefResult>, String>;
pub async fn pin_files(paths: &[PathBuf], opts: &PinOptions, reg: &RegistryClient, gitlab: &GitLabClient) -> Result<Report, String>;
```

## Task 1 — PinTarget enum + component discovery

Files: `src/discover.rs` (primary), `src/edit.rs`, `src/lib.rs`, `src/main.rs` (compile ripple only).

**RED** — rewrite the discover test module against `PinTarget` and add:

```rust
// existing image tests adapt: t.line()/t.value() via the accessor methods;
// image fixture expectations unchanged (ci-simple.yml, ci-rich.yml)
#[test] fn image_targets_survive_enum_migration() { /* ci-simple + ci-rich, as today */ }

#[test] fn component_top_level_target() {
    let text = "component: gitlab.com/g/sub/p/name@1.0.0\n";
    let t = discover(text);
    assert_eq!(t.len(), 1);
    match &t[0] {
        PinTarget::Component { value, parts, start, end, line } => {
            assert_eq!(value, "gitlab.com/g/sub/p/name@1.0.0");
            let p = parts.as_ref().expect("parts");
            assert_eq!((p.host.as_str(), p.project.as_str(), p.name.as_str(), p.version.as_str()),
                ("gitlab.com", "g/sub/p", "name", "1.0.0"));
            assert_eq!(&text[*start..*end], "gitlab.com/g/sub/p/name@1.0.0");
            assert_eq!(*line, 0);
        }
        other => panic!("expected Component, got {other:?}"),
    }
}

#[test] fn component_indented_is_not_a_target() {
    let text = "job:\n  component: gitlab.com/g/p/name@1.0.0\n";
    assert!(discover(text).is_empty());
}

#[test] fn component_unparseable_kept_with_none_parts() {
    // no '@', too few segments, templated value → parts None (skipped at resolve time)
    for v in ["gitlab.com/name@1.0.0", "gitlab.com/g/p/name", "${VAR}@1.0.0", "gitlab.com/g/p/name@"] {
        let text = format!("component: {v}\n");
        let t = discover(&text);
        assert_eq!(t.len(), 1, "{v}");
        assert!(matches!(&t[0], PinTarget::Component { parts: None, .. }), "{v}");
        assert_eq!(t[0].value(), v);
    }
}

#[test] fn component_quoted_and_comment() {
    let text = "component: \"gitlab.com/g/p/name@2.0\"  # from docs\n";
    let t = discover(text);
    let c = t.iter().find_map(|t| if let PinTarget::Component { .. } = t { Some(t) } else { None }).unwrap();
    assert_eq!(c.value(), "gitlab.com/g/p/name@2.0");
    assert_eq!(&text[c.start()..c.end()], "\"gitlab.com/g/p/name@2.0\"");
}

#[test] fn parse_component_ref_table() {
    // (input, expected (host, project, name, version))
    // ("gitlab.com/g/p/name@1.0.0", Some(("gitlab.com","g/p","name","1.0.0")))
    // ("gitlab.com/a/b/c/d/name@v9", Some(("gitlab.com","a/b/c/d","name","v9")))
    // ("gitlab.com/name@1.0.0", None)          // no project segment
    // ("gitlab.com/g/p/name", None)            // no '@'
    // ("gitlab.com/g/p/name@", None)           // empty version
    // ("$CI_SERVER_FQDN/g/p/name@1.0.0", None) // host is a template → unparseable (host must contain '.' and no '$')
}
```

Rules to implement (document in `discover` doc comment):
- Component scan: **indent 0 only**, `component:` followed by space/EOL; value = existing `scalar_token` logic (quotes stripped from value, span includes quotes); empty value or `#`-only → no target.
- `parse_component_ref(value) -> Option<ComponentParts>`: split on the **last** `@`; prefix must have ≥ 3 '/'-separated segments, all non-empty, host contains `.` and no `$`; version non-empty. Store `parts` (may be `None`); the target is still emitted (constraint 11).
- `discover` sorts by `(line, start)`; images first in the same line is natural by offset.

**GREEN** — implement; keep the `Target` struct removed (call sites: `edit::apply` takes `&[PinTarget]` and uses `t.start()`; `lib::resolve_target` uses accessors; e2e/oracle tests compile unchanged in behavior).

**Check:** `export CARGO_HOME="$PWD/.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH"; cargo test && cargo fmt --check`

**Commit:** `feat: component discovery — PinTarget enum and top-level component: references`

## Task 2 — classic include discovery

Files: `src/discover.rs`, `tests/fixtures/ci-include.yml` (new), compile ripple `src/lib.rs`.

**RED** — fixture `tests/fixtures/ci-include.yml` (all forms, with line comments stating intent):

```yaml
include: { project: gitlab-org/example, ref: main }          # flow single mapping → target
include:                                                      # block sequence
  - project: gitlab-org/example2
    ref: v1.2.3
    file: /ci/test.yml                                        # → target (ref value)
  - "local-template.yml"                                      # string item → no target
  - project: gitlab-org/no-ref                                # bare {project} → no target (spec)
  - remote: https://example.com/ci.yml                        # → no target
include:
  project: gitlab-org/single
  ref: develop                                                # block single mapping → target
job:
  include: { project: gitlab-org/indented, ref: main }        # indented → no target
```

Tests:

```rust
#[test] fn include_flow_single_mapping() { /* ref span = `main`, project without quotes, flow=true */ }
#[test] fn include_flow_sequence_mixed() { /* [ {mapping ref}, "local", {project only} ] → exactly 1 target, flow=true */ }
#[test] fn include_block_sequence_item() { /* ref span on the ref: line value; project across lines; flow=false */ }
#[test] fn include_block_single_mapping() { /* project: / ref: lines without dash; any key order (test ref-before-project) */ }
#[test] fn include_string_and_remote_not_targets() { /* "x.yml", remote:/template:/local: → no IncludeRef */ }
#[test] fn include_indented_not_a_target() { /* job-level include → none */ }
#[test] fn include_templated_ref_still_a_target() { /* ref: ${CI_REF} → target, value "${CI_REF}" */ }
#[test] fn include_quoted_values() { /* project: "g/p", ref: "main" → values unquoted, span includes quotes */ }
```

Rules to implement:
- `include:` **indent 0** only. Value shapes: (a) flow on one line (`include: {...}` or `include: [...]`, items split on top-level commas; an item is a scalar → ignore, or a `{...}` mapping with `key: value` pairs, one quote layer stripped, no nested `{`/`[` inside an item — if an item contains nested structure, that item yields no target); (b) block (empty value on the `include:` line): sequence items (`- {...}` flow item on one line, or `- key: val` opening a block mapping whose keys sit at the item indent) and the single-mapping form (keys directly at the block's child indent until dedent).
- A mapping yields an `IncludeRef` **iff** it has both `project` and `ref` keys. `project` value = scalar (quotes stripped). `ref` value = scalar token (span includes quotes, value stripped). Any other keys (`file`, `remote`… — note: `remote`/`template`/`local` items are ignored entirely) are irrelevant.
- Discovery stays syntactic: `ref: ${…}` stays a target (constraint 11).

**GREEN** — implement; keep T1's component scanner intact (add a combined test: component + include + images in one text, sorted by (line, start)).

**Check:** same cargo test + fmt.

**Commit:** `feat: classic include discovery — project/ref mapping forms (block + flow)`

## Task 3 — GitLab client

Files: `src/gitlab.rs` (new), `src/lib.rs` (module decl). No behavior change elsewhere.

**RED** — unit tests (inline `#[cfg(test)]`) + wiremock tests (follow the `tests/mock_registry.rs` helper pattern; new `tests/gitlab_client.rs` or inline — implementer's choice, wiremock 0.6.5 API: `.and(path(...))`, `query_param`, header matchers, `expect()`):

```
tag_200_returns_commit_id        GET /projects/g%2Fp/repository/tags/v1.0.0 → {commit:{id: <40hex>}}; assert URL-encoding of '/'
annotated_uses_commit_id         target != commit.id → commit.id returned
branch_200_returns_commit_id     PAT: GET /projects/g%2Fp/repository/branches/feature%2Fx
bare_name_tag_first              bare "main": tag 200 → no branch request (branch mock expect(0))
bare_name_falls_back_to_branch   bare "main": tag 404 → branch 200 → sha; tag expect(1)
bare_name_both_404               → Err "…not found (no tag or branch)" (message contains project and ref)
sha_short_circuit                resolve("g/p", <40hex>) → Ok, zero HTTP mocks (any request → wiremock fails the test)
refs_qualified_routing           refs/tags/v1 → tag endpoint only; refs/heads/b → branch endpoint only
http_401_unresolved              message contains "401" and a token hint (GITLAB_TOKEN/CI_JOB_TOKEN)
http_429_unresolved              message contains "429"
job_token_branch_via_list        job_token=true: branch resolved via GET /projects/g%2Fp/repository/branches?search=main;
                                 response array has a near-miss "main-x" and exact "main" → exact match wins;
                                 assert JOB-TOKEN header present
pat_branch_single_endpoint       job_token=false: single-branch URL; assert PRIVATE-TOKEN header
cache_hit                        resolve same (project, ref) twice → tag endpoint hit exactly once
env_precedence                   from_env: neither set → no token; CI_JOB_TOKEN only → job_token=true;
                                 both set → GITLAB_TOKEN wins (job_token=false)
url_encode_table                 "a/b c" → "a%2Fb%20c"; "a-b_c.d~e" unchanged; "v1.0.0" unchanged
```

**GREEN** — implement per Interfaces + constraints 6/7/9. Details:
- `reqwest::Client::builder().timeout(30s)` (same pattern as `RegistryClient`); `json()` feature NOT enabled in this crate → `text()` + `serde_json` (M1 lesson).
- 404 from tag/branch endpoints is an internal `NotFound` sentinel (private enum) distinct from surfaced `GitLabError`; only the final fallback surfaces `Unresolved`.
- 401/403 → `Unresolved("gitlab api: {status} unauthorized (check GITLAB_TOKEN/CI_JOB_TOKEN)")`; other statuses → `Unresolved("gitlab api: {status}")`; 429 → `Unresolved("gitlab api: rate limited (429), retry later")`; non-2xx body JSON parse irrelevant (we only parse 200 bodies); 200 with missing/invalid `commit.id` → `Unresolved("gitlab api: malformed response")`.
- `from_env`: `GITLAB_TOKEN` (non-empty) → `token=Some, job_token=false`; else `CI_JOB_TOKEN` (non-empty) → `token=Some, job_token=true`; else none.
- Cache: `tokio::sync::Mutex<HashMap<(String, String), String>>` keyed `(project, ref_name)` — only successful resolutions cached.

**Check:** same cargo test + fmt.

**Commit:** `feat: gitlab client — tag/branch to commit SHA with PAT/job-token auth and cache`

## Task 4 — orchestration: pin components & includes

Files: `src/edit.rs`, `src/lib.rs`, `src/main.rs` (compile), new `tests/e2e_components.rs`.

**RED** — unit (edit.rs) first:

```rust
#[test] fn splice_replace_preserves_quotes()  // "main" → "<sha>" stays quoted; bare stays bare
#[test] fn apply_splices_inserts_and_replaces // (start==end) insertion + replacement, end-first ordering
#[test] fn comment_offset_bare_line           // "  ref: main\n" token_end after main → offset == EOL (before \n)
#[test] fn comment_offset_existing_comment    // "  ref: main  # keep\n" → None
#[test] fn comment_offset_flow_line           // "include: { project: g/p, ref: main }\n" token_end after main → offset == line EOL (after '}')
```

Then e2e (wiremock registry + wiremock gitlab, temp file via `std::env::temp_dir` under the repo's `.tmp/` scratch like M2 e2e):

```
1. mixed_file_all_pinned      image + component + include(block) in one file → 3 pinned;
                              byte-exact expected output:
                                component: gitlab.com/g/p/name@<sha>  # 1.0.0
                                include mapping ref line: "  ref: <sha>  # main"
                              exit 0
2. idempotent_second_run      run twice: second run all ok (component @sha, ref sha, image digest), zero writes
3. skips_keep_exit_zero       component host ghcr.io… (host not enabled), catalog version (1.0.x),
                              templated include ref ($VAR), component without '@' → all skipped, exit 0, no writes
4. unresolved_exit_one        gitlab 404 both for one include; another image pins → file written with image only, exit 1
5. existing_comment_unchanged ref line already has "# keep" → pinned, line == "  ref: <sha>  # keep"
6. flow_include_comment_after_bracket  include: { project: g/p, ref: main }  → "include: { project: g/p, ref: <sha> }  # main"
7. dry_run_no_write           statuses reported, file untouched
```

**GREEN** — implement per Interfaces:
- `resolve_component`: `parts` None → `skipped (unparseable reference)`; `host != opts.gitlab_host` → `skipped (host not enabled: {host})`; version 40-hex lowercase → `ok`; version catalog-managed (constraint 8 regex) → `skipped (catalog-managed version)`; else `gitlab.resolve(project, version)` → Ok(sha): new value = `{host}/{project}/{name}@{sha}` via `splice_replace` on the whole token; Err → `unresolved ({msg})`.
- `resolve_include`: value 40-hex → `ok`; value contains `$` → `skipped (templated reference)`; else `gitlab.resolve(project, ref)` → Ok(sha): `splice_replace` on the ref span; Err → `unresolved ({msg})`.
- Comment insertion: for `pinned` component/include results only, `comment_insert_offset(text, line, token_end)` → Some(pos): push splice `(pos, pos, format!("  # {orig}"))` where `orig` = the original version (component) or original ref value (include). `comment_insert_offset` = byte offset at line end (just before `\n`/EOF); `None` when a `#` occurs after the token on the same line.
- Splice collection: build one `Vec<(usize, usize, String)>` from replacements (BTreeMap dedupe by start, as today) + comment insertions; call `apply_splices`; write file iff ≥ 1 splice and not dry-run.
- GitLab resolves run under the same semaphore as digest lookups.
- `RefResult.reference` for components = full component value; for includes = the ref value (old), consistent with M1 convention (reference = discovered value).
- `build_opts` in main.rs sets `gitlab_host: "gitlab.com".into()` (flag wiring is T5); `main` constructs `gitlab_from_env(&opts.gitlab_host)` and passes both clients.

**Check:** same cargo test + fmt; M1/M2 e2e + oracle tests must stay green unchanged.

**Commit:** `feat: pin components and includes to commit SHA with original-ref comments`

## Task 5 — CLI flag + README

Files: `src/main.rs`, `README.md`.

- `--gitlab-host` (default `gitlab.com`): `build_opts` reads it; doc: "GitLab instance host for component/include resolution (default gitlab.com)".
- README M3 section: what gets pinned (component refs → `@<sha>`; include `ref:` → SHA + trailing comment of the original ref), skip rules (catalog versions, non-matching host, templated, bare `{project}`, local/remote/template includes), auth env vars + job-token behavior, `--gitlab-host`, 429 note, exit codes.
- Test: `cli_gitlab_host_parsed` (default `gitlab.com`; `--gitlab-host gitlab.example.com` overrides).

**Check:** same cargo test + fmt + `cargo clippy --all-targets -- -D warnings`.

**Commit:** `feat: --gitlab-host flag; docs: README M3 section`

## Task 6 — live cross-check test + final gate

Files: `tests/live_gitlab.rs` (new).

- Gated `PIN_CI_LIVE=1` (same gating pattern as the registry live tests); SKIP with a remedy hint otherwise.
- Pick REAL public refs at implementation time and record them as constants with a comment showing the `git ls-remote` command used to verify them. Candidate tag: an old, stable tag of a public gitlab.com project (verify existence first — do NOT guess). Candidate branch: a long-lived public branch (e.g. `main`/`master` of the same project).
- Test flow: `GitLabClient::from_env("https://gitlab.com/api/v4")` → `resolve(project, tag)` and `resolve(project, branch)` → compare against `git ls-remote https://gitlab.com/<project> refs/tags/<tag> refs/heads/<branch>` output (parse `<sha>\t<ref>` lines; SKIP with hint if `git` is not on PATH — git is in the devenv).
- No oracle against renovate (it does not handle components — recon §7); the `git ls-remote` cross-check IS the independent oracle.

**Final gate (controller, before merge):** full suite (incl. live with `PIN_CI_LIVE=1`), `cargo fmt --check`, `clippy -D warnings`, release build; a live run on a scratch copy of `ci/.gitlab-ci.yml` augmented with one public component + one classic include (in `.tmp/`, never the real file) → expected pins + idempotent second run; merge to main (`git -c commit.gpgsign=false`), ledger close-out.

**Commit:** `test: live gitlab resolution with git ls-remote cross-check`

## Review Focus (inline reviews)

1. **Enum migration is behavior-preserving** — M1/M2 e2e, live and oracle tests pass untouched; `ci-simple`/`ci-rich` expectations byte-identical.
2. **Component scanner** — indent-0 only; quoted values; trailing comment excluded from span; parts-None cases all kept as targets.
3. **Include scanner** — every form in the fixture; key order independence; nested-structure flow item → no target; indented include ignored; bare `{project}` produces NO target (spec).
4. **GitLab client** — `url_encode` on project path AND ref name; job-token branch path uses list+search with EXACT name match (near-miss test); tag-before-branch order; SHA short-circuit makes zero requests; cache per `(project, ref)`.
5. **Comment splice** — byte-exact placements (after value / after closing bracket); no double comment when one exists; insertion splices ordered end-first together with replacements.
6. **Exit codes** — skipped (host/catalog/templated/unparseable) keeps exit 0; unresolved (API 404-both/401/429/network) → exit 1 with partial-write semantics (other targets still written).

## Self-review (controller, post-write)

- Task boundaries: T1/T2 = discovery (pure, testable offline); T3 = network client (wiremock); T4 = glue + editor; T5 = CLI/docs; T6 = live. Each has a single commit and a check command. ✓
- Interface consistency: `PinTarget` accessors used by edit/lib; `GitLabClient::resolve` signature matches recon endpoints; `apply_splices` superset of `apply`. ✓
- Spec acceptance coverage: M3 milestone items (GitLab client ref→commit tags+branches ✓ T3; classic includes explicit ref any order incl. file ✓ T2; bare {project} out ✓; component refs pinned to SHA ✓ T1+T4; comment format ✓ T4; pin-only ✓ constraint 1). Testing section: GitLab client vs mock HTTP (tag/branch→SHA, 404) ✓ T3; opt-in integration incl. public gitlab.com component ✓ T6. ✓
- Open item for impl time: exact public (project, tag, branch) constants for T6 — verify via `git ls-remote` before writing the test.
