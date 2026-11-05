# M2: `--update` for Docker Hub images

**Goal.** Add `--update` (and `--update-major`) to `pin-ci`: for **unpinned**
Docker Hub image references whose tag is a docker version, bump the tag to the
highest *stable* docker-versioning tag (same major by default;
`--update-major` lifts the restriction) and pin the resulting digest.
Already-pinned references re-pin only (tag never changes). Non-semver tags
(`latest`, `main`, `stable`, `rolling`, ...) never bump — pin only.

**Source of truth.** `docs/superpowers/specs/2026-11-01-pin-ci-design.md`
(Update policy, Re-pin semantics, Pin format, Milestones M2). M1 is merged to
main (merge commit `ad42721`); all M1 global constraints stay in force.

**Why this is safe.** The versioning module is a minimal, testable port of
Renovate's `docker` versioning (the same engine the old pipeline uses). The
differential oracle gains an M2 mode: old pipeline (which bumps by default) vs
`pin-ci --update --update-major` — under `--update-major` both sides share the
same selection policy, so outputs must match per reference (modulo the old
config's `minimumReleaseAge: 14d`, which can only make the old pipeline's pick
older — a *documented divergence* rule, see Task 5).

## Architecture

New module `src/versioning.rs` (pure, no deps, no I/O):

```
parse(tag) -> Option<Version { release: Vec<u64>, prerelease: String, suffix: String }>
Version::is_stable / is_compatible / major
compare(a, b) -> Option<Ordering>          // docker flavor: shorter release is BIGGER
pick_latest_stable(current, tags, allow_major) -> Option<String>
```

Faithful port of Renovate `lib/modules/versioning/docker/index.ts` (+ generic
base), verified against the raw sources 2026-11-02. Deliberate, documented
divergence: Renovate uses `localeCompare` for prerelease/suffix ties; we use
bytewise `str::cmp` (deterministic, no ICU dependency; only affects ties that
change nothing in practice).

`src/registry.rs` gains `list_tags`:

```
GET {base}/v2/{repo}/tags/list?n=1000   (anonymous probe → 401 → bearer → retry)
follow `Link: <url>; rel="next"` until exhausted
404 → Unresolved("{repo} not found")
cached per api_repo for the client's lifetime (tokio::sync::Mutex<HashMap<...>>)
```

`src/lib.rs` orchestration: `PinOptions { update, update_major }`; in
`resolve_target`, after the digest resolves for an unpinned Docker Hub ref:
`update && tag parses as version` → `list_tags` (error → `unresolved`) →
`pick_latest_stable` → if a better tag: digest the new tag, splice
`repo:new_tag@digest`, status `updated`. Pinned refs, unparsable tags, tagless
refs: M1 pin-only behavior unchanged.

`src/report.rs`: new status `updated`, `Summary.updated`, human + JSON.

`src/main.rs`: `--update`, `--update-major` flags.

**Pin format (spec-decided, no comments):** `alpine:3.24@sha256:...`. The old
tag is visible in the report (`old -> new`) and git history — same as the old
pipeline's sed output. This is exactly what makes the M2 oracle a byte-comparison.

## Tech stack / global constraints

Unchanged from M1 (single crate, reqwest + serde + sha2 + tokio + clap +
wiremock in dev; no runtime deps). **No new dependencies** — the versioning
parse is hand-rolled (tag character set is already validated by
`ImageRef::parse`: `[A-Za-z0-9._-]`, which bounds the parse).

Carried-over M1 constraints that this milestone must not break:

- Pin-only is the default; non-Hub hosts are detected and `skipped`, never touched.
- Byte-exact edits: only the reference token changes; untouched lines byte-identical.
- Idempotency: second run reports zero changes (`ok` / no `updated`).
- Exit code `1` iff any `unresolved`; `updated` does not affect exit.
- Old pipeline files (`pin-ci-images.sh`, `extract-docker-pins.py`, `config.js`)
  are never modified — they are the oracle.
- Rust env (devenv session; rust is NOT on the env PATH by design):
  `export CARGO_HOME="$PWD/.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH"`
  Scratch in `.tmp/` (gitignored). Oracle-only tools (renovate, crane,
  python3, git) come from the active devenv shell.
- Commits unsigned: `git -c commit.gpgsign=false commit ...` (nono blocks ~/.gnupg).
- TDD: RED (failing test) → GREEN (implement) → REFACTOR, per task.
- `cargo fmt` clean, `cargo clippy --all-targets -- -D warnings` clean after every task.

## Files

| Task | Create | Modify |
|---|---|---|
| T1 | `src/versioning.rs` | `src/lib.rs` (mod decl) |
| T2 | — | `src/registry.rs`, `tests/mock_registry.rs` (helpers) |
| T3 | `tests/fixtures/ci-update.yml` | `src/lib.rs`, `src/edit.rs`, `src/report.rs` |
| T4 | — | `src/main.rs`, `tests/e2e.rs`, `tests/fixtures/ci-update.yml` |
| T5 | `tests/fixtures/ci-update-clean.yml` | `tests/live.rs`, `tests/oracle.rs`, `README.md` |

Per-task Interfaces blocks are the contract; later tasks may rely on them but
must not change them.

---

## Task 1: versioning module

**Files:** create `src/versioning.rs`; modify `src/lib.rs` (add
`pub mod versioning;` after `pub mod registry;`).

### Interface

```rust
pub struct Version { pub release: Vec<u64>, pub prerelease: String, pub suffix: String }
impl Version {
    pub fn is_stable(&self) -> bool;                    // prerelease empty
    pub fn is_compatible(&self, other: &Version) -> bool; // same suffix AND same release length
    pub fn major(&self) -> Option<u64>;
}
pub fn parse(tag: &str) -> Option<Version>;
pub fn compare(a: &str, b: &str) -> Option<Ordering>;   // None if either unparseable
pub fn pick_latest_stable(current: &str, tags: &[String], allow_major: bool) -> Option<String>;
```

Semantics (Renovate docker flavor — port exactly, tests encode these):

- `parse`: reject empty; reject commit hashes (`[a-f0-9]{7,40}` with ≥1 letter;
  pure-digit "1234567" is NOT rejected — it's a valid version); strip one
  leading `v`; split at the FIRST `-` → `(prefix, suffix)` (suffix may be `""`);
  the numeric core of `prefix` is the longest `\d+(\.\d+)*` run starting at
  position 0 (leading zeros OK — `"24.04"` → `[24, 4]`); the remainder of
  `prefix` is the prerelease and must be all `[A-Za-z0-9_]` (`""` allowed);
  release parts parse to `u64`. A dot not followed by a digit is dangling and
  invalidates the whole parse (the anchored regex `\d+(?:\.\d+)*` + `\w*`
  cannot match it — e.g. `"1.2.x"` → `None`), while `"1.2x"` → core `1.2`,
  prerelease `x`.
- `compare`: compare release part-by-part up to the longer length; a missing
  part on either side means the SHORTER version is BIGGER (`2.1 > 2.1.1`);
  then prerelease: absent > present, both present → bytewise `cmp`;
  then suffix: reversed order (the lexicographically SMALLER suffix is the
  BIGGER version — Renovate returns `suffix2.localeCompare(suffix1)`).
- `pick_latest_stable`: candidates = tags where `parse` ok, `is_stable`,
  `is_compatible` with current, and (`allow_major` OR same major). Return the
  candidate that beats all others, only if it is `compare`-greater than
  `current`; else `None` (no bump). `None` also when `current` is unparseable.

### Steps

1. **RED** — write `src/versioning.rs` containing only the test module
   (items above as `todo!()` bodies so it compiles). Test table:

```rust
#[test]
fn parse_table() {
    type Case = (&'static str, Option<(Vec<u64>, &'static str, &'static str)>);
    let cases: &[Case] = &[
        ("3.20", Some((vec![3, 20], "", ""))),
        ("24", Some((vec![24], "", ""))),
        ("24.04", Some((vec![24, 4], "", ""))),
        ("1.2.3", Some((vec![1, 2, 3], "", ""))),
        ("v1.2.3", Some((vec![1, 2, 3], "", ""))),
        ("1.2.3-rc1", Some((vec![1, 2, 3], "", "rc1"))), // dash → suffix (stable), not prerelease
        ("3.20-alpine", Some((vec![3, 20], "", "alpine"))),
        ("1.2.3rc1", Some((vec![1, 2, 3], "rc1", ""))),
        ("1.2.3-rc-1", Some((vec![1, 2, 3], "", "rc-1"))),
        ("1.2.x", None), // dot before the prerelease is unmatchable by the anchored regex
        ("1234567", Some((vec![1234567], "", ""))),
        ("", None),
        ("latest", None),
        ("main", None),
        ("stable", None),
        ("1..2", None),
        ("abc", None),
        ("abcdef1", None), // commit hash
        (".1", None),
        ("1.", None),
    ];
    for (tag, expected) in cases {
        let got = parse(tag).map(|v| (v.release.clone(), v.prerelease.as_str(), v.suffix.as_str()));
        assert_eq!(got, *expected, "parse({tag:?})");
    }
}

#[test]
fn stability() {
    assert!(parse("3.20").unwrap().is_stable());
    assert!(parse("3.20-alpine").unwrap().is_stable()); // suffix does NOT destabilize (Renovate generic isStable)
    assert!(parse("3.20.5-rc0").unwrap().is_stable()); // dash-suffix is stable in Renovate docker versioning
    assert!(!parse("1.2.3rc1").unwrap().is_stable());
}

#[test]
fn compatibility() {
    let a = parse("3.20").unwrap();
    assert!(a.is_compatible(&parse("3.24").unwrap()));
    assert!(!a.is_compatible(&parse("3.24.0").unwrap())); // release length
    assert!(!a.is_compatible(&parse("3.20-alpine").unwrap())); // suffix
    assert!(parse("3.20-alpine").unwrap().is_compatible(&parse("3.24-alpine").unwrap()));
}

#[test]
fn compare_table() {
    use std::cmp::Ordering::*;
    type Case = (&'static str, &'static str, Ordering);
    let cases: &[Case] = &[
        ("2.1", "2.1.1", Greater),  // shorter is bigger (docker flavor)
        ("2.1.1", "2.1", Less),
        ("2.1", "2.2", Less),
        ("3.20.5", "3.20", Less),
        ("1.2.3-rc1", "1.2.3", Less), // suffixed < plain (smaller suffix wins)
        ("1.2.3", "1.2.3-rc1", Greater),
        ("1.2.3-rc1", "1.2.3-rc2", Greater), // suffix order (rc1 < rc2 lexically)
        ("1.2.3rc1", "1.2.3rc2", Less), // glued prereleases compare directly
        ("3.20-ubuntu", "3.20-alpine", Less), // smaller suffix wins (Renovate: suffix2.localeCompare(suffix1))
        ("3.20", "3.20", Equal),
    ];
    for (a, b, want) in cases {
        assert_eq!(compare(a, b), Some(*want), "{a} vs {b}");
    }
    assert_eq!(compare("latest", "3.20"), None);
}

#[test]
fn pick_latest_stable_table() {
    let t = |s: &str| s.to_string();
    // same-major: 4.x and 3.x-pre and incompatible shapes must not win
    assert_eq!(
        pick_latest_stable("3.20", &[t("3.16"), t("3.20"), t("3.24"), t("3.24.0"), t("4.0"), t("3.25-rc1"), t("3.20-alpine")], false),
        Some("3.24".to_string())
    );
    // --update-major: 4.2 wins
    assert_eq!(
        pick_latest_stable("3.20", &[t("3.24"), t("4.2"), t("4.0")], true),
        Some("4.2".to_string())
    );
    // single-component current: only 1-part candidates; nothing newer → no bump
    assert_eq!(pick_latest_stable("24", &[t("24"), t("24.0.1"), t("29")], false), None);
    assert_eq!(pick_latest_stable("24", &[t("24"), t("24.0.1"), t("29")], true), Some("29".to_string()));
    // nothing above current
    assert_eq!(pick_latest_stable("3.20", &[t("3.19"), t("3.20")], true), None);
    // non-semver current → never bumps
    assert_eq!(pick_latest_stable("latest", &[t("3.24")], true), None);
    // empty pool
    assert_eq!(pick_latest_stable("3.20", &[], true), None);
}
```

   Run: `cargo test --release versioning` → RED (compiles, all fail on `todo!()`).
   Record the exact RED output in the commit body or the ledger.
2. **GREEN** — implement `parse`/`compare`/`pick_latest_stable` exactly per the
   semantics above (hand-rolled, no regex crate). Add `pub mod versioning;` to
   `src/lib.rs`. Run: `cargo test --release versioning` → 5/5 pass.
3. **REFACTOR** — `cargo fmt`; `cargo clippy --all-targets -- -D warnings` clean.
4. **Commit** — `git -c commit.gpgsign=false commit -m "feat: docker versioning module (minimal Renovate port)"`
   touching only `src/versioning.rs` + `src/lib.rs`.

**Interfaces for later tasks:** the five public items above, exactly as
declared. `pick_latest_stable` takes `&[String]` (callers own the tag list).

---

## Task 2: registry `list_tags` with Link pagination + per-repo cache

**Files:** modify `src/registry.rs`, `tests/mock_registry.rs`.

### Interface

```rust
impl RegistryClient {
    /// All tags of the repo, following `Link: <url>; rel="next"` until
    /// exhausted. First request carries `?n=1000`. Anonymous probe first;
    /// 401 → WWW-Authenticate → bearer token (scope `repository:{repo}:pull`)
    /// → retry. 404 → `Unresolved("{repo} not found")`. Cached per
    /// `api_repo()` for the client's lifetime.
    pub async fn list_tags(&self, r: &ImageRef) -> Result<Vec<String>, RegError>;
}

/// Parse a `Link` header, returning the URL whose `rel` is `"next"`.
fn parse_link_next(header: Option<&reqwest::header::HeaderValue>) -> Option<String>;
```

Implementation notes:

- Extract the shared request dance from `digest` into a private helper
  `get_authed(&self, url: &str, repo: &str, accept: Option<&str>) ->
  Result<reqwest::Response, RegError>`: anonymous GET; on 401 parse
  `WWW-Authenticate`, verify scope `repository:{repo}:pull`, fetch cached
  bearer token, retry once with `Authorization: Bearer`. `digest` keeps its
  HEAD-first flow but may reuse the helper for its GET fallback; **all
  existing wiremock tests must pass unchanged** (behavior identical).
- Body: `{"tags": [...]}` via `resp.text()` + `serde_json` (the `json()`
  reqwest feature is intentionally not enabled). A missing/`null` `tags`
  array is an empty list.
- `parse_link_next` splits on `,`, then on the first `;`; the URL must be
  `<>`-quoted and the params must contain `rel="next"` (case as spec'd).
- New client field: `tag_lists: tokio::sync::Mutex<HashMap<String, Vec<String>>>`
  (initialized empty in `new()`); check-then-insert under the lock; on
  success store the full list, on error store nothing.

### Steps

1. **RED** — add unit tests (registry `tests` mod or wiremock `#[tokio::test]`
   in `tests/mock_registry.rs`, matching the file's existing style):

```rust
// wiremock, client pointed at the mock base (see existing helper patterns):
#[tokio::test]
async fn list_tags_single_page() { /* {"tags":["a","b"]} → ["a","b"]; first URL has query n=1000 */ }
#[tokio::test]
async fn list_tags_follows_link_next() { /* page1 + Link <next>; rel="next" → concatenated order */ }
#[tokio::test]
async fn list_tags_404() { /* Unresolved("{repo} not found") */ }
#[tokio::test]
async fn list_tags_bearer_flow() { /* 401 + WWW-Authenticate → token endpoint (expect 1) → 200 with tags */ }
#[tokio::test]
async fn list_tags_caches_per_repo() { /* two calls, mock expect(1) → second call served from cache */ }
#[test]
fn parse_link_next_unit() {
    // None → None; `<u1>; rel="prev", <u2>; rel="next"` → u2; junk → None
}
```

   Run: `cargo test --release list_tags` → RED (E0599 no method / compile
   fail is acceptable RED; record it).
2. **GREEN** — implement per the notes. Run: `cargo test --release registry`
   AND the full existing suite (`cargo test --release`) → all pass (no
   regressions in the 39 M1 tests).
3. **REFACTOR** — fmt + clippy clean; verify `digest` wiremock tests still
   green explicitly.
4. **Commit** — `git -c commit.gpgsign=false commit -m "feat: registry tag listing with Link pagination and per-repo cache"`.

**Interfaces for later tasks:** `list_tags` exactly as above. Cache key is
`api_repo()`.

---

## Task 3: `--update` orchestration (library level)

**Files:** modify `src/lib.rs`, `src/edit.rs`, `src/report.rs`; create
`tests/fixtures/ci-update.yml`.

### Interface

```rust
pub struct PinOptions {
    pub dry_run: bool,
    pub json: bool,
    pub concurrency: usize,
    pub update: bool,        // NEW
    pub update_major: bool,  // NEW (meaningful only with update)
}

// src/edit.rs
/// Tag bump + pin: `alpine:3.20` → `alpine:3.24@sha256:...`. Quote-preserving
/// like `splice_value`. Callers guarantee: ref has a tag and no digest.
pub fn splice_update(old_token: &str, new_tag: &str, digest: &str) -> String;

// src/report.rs
// RefResult.status gains "updated"; Summary gains `pub updated: usize`;
// render_human summary line becomes:
//   pinned: {n}, ok: {n}, updated: {n}, skipped: {n}, unresolved: {n}
```

`resolve_target` flow (new branch, after the existing digest resolution, only
when `opts.update && r.digest.is_none()`):

```
match r.tag {
    Some(tag) if !tag.is_empty() => {
        if versioning::parse(tag).is_some() {
            let tags = client.list_tags(&r).await
                → Err(e) ⇒ status "unresolved", reason format!("tag list: {e}"), return
            match versioning::pick_latest_stable(tag, &tags, opts.update_major) {
                Some(new_tag) if new_tag != tag => {
                    let mut bumped = r.clone(); bumped.tag = Some(new_tag.clone());
                    let new_digest = client.digest(&bumped).await (same error handling as above);
                    if r.digest... (impossible here: unpinned) →
                    status "updated"; new = splice_value result of splice_update(token, &new_tag, &new_digest)
                    replacement Some((t.start, new_token))
                    return
                }
                _ => {} // no bump: fall through to M1 pin path
            }
        }
    }
    _ => {} // tagless / non-semver: M1 pin path
}
```

Rules (global): pinned refs NEVER bump (re-pin only — spec re-pin semantics);
`update_major` without `update` is a no-op (CLI may warn, not required);
`updated` refs count in `Summary.updated` and do NOT affect `exit_code`
(still `1` iff `unresolved > 0`).

### Steps

1. **RED** — add library-level integration tests in `tests/e2e.rs` using the
   existing wiremock pattern (the e2e file already builds a `RegistryClient`
   against a mock base; extend its mock setup for T3's needs):

   - `update_bumps_scalar_and_pins_digest`: mock repo `alpine`
     tags `[3.16, 3.20, 3.24]`, manifests for `3.20` and `3.24` (two
     different `Docker-Content-Digest` headers); fixture
     `tests/fixtures/ci-update.yml`:
     ```yaml
     build:
       image: alpine:3.20
     sidecars:
       services:
         - alpine:3.20
     plain:
       image: nginx:stable
     ```
     Run `pin_file` with `PinOptions { update: true, update_major: false, .. }`
     → alpine refs become `alpine:3.24@sha256:<digest-of-3.24>` (status
     `updated`); `nginx:stable` becomes `nginx:stable@sha256:...` (status
     `pinned`); file written; report summary `updated: 2, pinned: 1`.
   - `update_no_bump_when_already_highest`: tags `[3.20]` only → status
     `pinned`, token `alpine:3.20@sha256:...`.
   - `update_same_major_restriction`: repo `docker`, current `24`, tags
     `[24, 24.0.1, 29]` → stays `docker:24@...` (pinned); with
     `update_major: true` → `docker:29@...` (updated).
   - `update_pinned_never_bumps`: pre-pinned `alpine:3.20@sha256:<digest-of-3.20>`
     with tags `[3.20, 3.24]` → status `ok`, file unchanged.
   - `update_tag_list_failure_is_unresolved`: tags endpoint returns 500 →
     status `unresolved`, reason starts with `tag list:`, `exit_code` == 1,
     file unchanged.
   - `update_off_keeps_m1_behavior`: `update: false` → statuses exactly as
     M1 (`pinned` only).
2. **GREEN** — implement: `splice_update` (mirror `splice_value`'s quote
   handling; `inner` split at the last `:` after the last `/`; output
   `format!("{repo_part}:{new_tag}@{digest}")` re-quoted); `PinOptions`
   fields; `resolve_target` branch (it must now receive `opts`); report
   changes. Run: `cargo test --release` → new tests green, all M1 tests green
   (update the M1 tests that construct `PinOptions` — add the two fields).
3. **REFACTOR** — fmt + clippy clean.
4. **Commit** — `git -c commit.gpgsign=false commit -m "feat: --update orchestration (same-major stable bump + pin)"`.

**Interfaces for later tasks:** `PinOptions { update, update_major, ... }`;
`splice_update`; status string `"updated"`; `Summary.updated`.

---

## Task 4: CLI flags + e2e report/dry-run/idempotency

**Files:** modify `src/main.rs`, `tests/e2e.rs`, `tests/fixtures/ci-update.yml`.

### Interface

```
pin-ci [FILES...] [--update] [--update-major] [--dry-run] [--json] [--concurrency N]
```

`--update-major` implies nothing (documented as meaningful only with
`--update`). `main` passes the flags into `PinOptions`.

### Steps

1. **RED** — e2e tests:

   - `cli_update_flags_parsed`: clap parse (via `Cli::try_parse_from` if
     made testable, else assert `PinOptions` wiring in a thin
     `opts_from_cli` helper — **preferred**: extract
     `fn opts_from_cli(cli: &Cli) -> PinOptions`? Keep it simple: test the
     full binary is out of scope; instead add a small unit test in `main.rs`
     behind `#[cfg(test)]` that calls a new `pub fn build_opts` — if that
     feels too invasive, SKIP the unit test and cover flag wiring via the
     `--json` e2e below plus the README; record the choice in the ledger).
   - `update_json_report`: run `pin_files` with `update: true`, `--json`
     payload: `summary.updated` present, `results[*].status` includes
     `updated`, `new` shows `alpine:3.24@sha256:...`.
   - `update_human_report_line`: human line
     `…  updated  alpine:3.20 -> alpine:3.24@sha256:…  # alpine:3.20`.
   - `update_dry_run_does_not_write`: `dry_run: true, update: true` → file
     bytes unchanged, statuses as before.
   - `update_idempotent_second_run`: write file once with `update: true`;
     run again → all `ok` (pinned refs), `updated: 0`, file unchanged.
   - `update_exit_code`: mix of `updated` + `skipped` → exit 0; add a
     failing tags endpoint → exit 1.
2. **GREEN** — implement flags in `main.rs`; wire into `PinOptions`.
3. **REFACTOR** — fmt + clippy; full suite.
4. **Commit** — `git -c commit.gpgsign=false commit -m "feat: --update and --update-major CLI flags"`.

---

## Task 5: live update test + M2 oracle + README

**Files:** create `tests/fixtures/ci-update-clean.yml`; modify
`tests/live.rs`, `tests/oracle.rs`, `README.md`.

### The M2 oracle (differential, opt-in `#[ignore]`)

Design (verified against `pin-ci-images.sh` 2026-11-02):

- **Fixture** `tests/fixtures/ci-update-clean.yml` — Docker Hub images ONLY
  (no templated refs, no other registries, no tagless images — the old
  pipeline's sed cannot address tagless refs, so exclude them from
  strict comparison):
  ```yaml
  build:
    image: alpine:3.20
  app:
    image: docker:24
  ubuntu:
    image: ubuntu:24.04
  sidecars:
    services:
      - alpine:3.20
  ```
- **Setup** (reuse M1 oracle's temp-repo scaffolding: git init,
  `TMPDIR=$PWD/.tmp`, `/tmp/renovate.log` symlink, crane `DOCKER_CONFIG`
  probe, skip-with-remedy when tools/env missing):
  temp repo with `ci/.gitlab-ci.yml` = fixture. Two runs:
  1. **Old pipeline:** `bash <repo>/pin-ci-images.sh` with CWD = temp repo
     (script hardcodes `TARGET_FILE=ci/.gitlab-ci.yml` relative to CWD and
     locates `extract-docker-pins.py`/`config.js` via its own
     `SCRIPT_DIR`). Output file = temp `ci/.gitlab-ci.yml` (call it A).
  2. **New tool:** copy fixture back to temp `ci/.gitlab-ci.yml`, run
     `pin_files` with `PinOptions { update: true, update_major: true,
     dry_run: false, .. }` and a real `client_from_env()`. Output = B.
- **Comparison** per original reference position (key: 1-based line + repo
  name, from the fixture): parse `repo:tag@sha256:hex` from both A and B
  (M1's scanner helpers already exist in oracle.rs).
  - Tag equal → digest MUST be byte-identical (else FAIL).
  - Tag differs → **documented divergence**, allowed ONLY if: both tags
    `versioning::parse`-valid + `is_stable` + `is_compatible`, and
    `compare(new_tag, old_tag) == Greater`. (The old config's
    `minimumReleaseAge: 14d` can only make the old pick older; anything else
    is a real bug.) In that case, sanity-check BOTH digests with `crane
    digest` (existing helper) instead of cross-equality.
  - Otherwise FAIL with both tokens printed.
- **Note:** `--update-major` is used so both sides share the selection
  policy (old pipeline has no same-major restriction; observed bumps:
  alpine 3.20→3.24, ubuntu 24.04→26.04, docker 24→29). Default `--update`
  (same-major) is intentionally NOT oracle-compared — it is by design more
  conservative (spec: same-major default is our policy choice, not the old
  pipeline's).

### Live update test (opt-in `#[ignore]`, real Docker Hub)

- Temp copy of the clean fixture; run `pin_files` with
  `update: true, update_major: true`; for each bumped ref, independently:
  `tags = client.list_tags(ref)`, `want =
  versioning::pick_latest_stable(current, &tags, true)` → assert the tool's
  chosen tag == `want` (or current when `want` is None) and the written
  digest == `crane digest repo:tag`. Second run → all `ok`, no changes.

### README

Document: `--update` / `--update-major`, the selection policy (stable,
compatible = same suffix + same component count, same-major default),
never-bump list (non-semver tags, pinned refs, tagless refs), tag-list
failure → `unresolved`, and the two deliberate divergences from the old
pipeline (same-major default; no release-age guard — M4 follow-up).

### Steps

1. **RED** — implement the fixture + the M2 oracle test + live test with
   assertions per design; run `cargo test --release -- --ignored
   m2_oracle` → RED (logic not implemented / comparison mismatch expected).
2. **GREEN** — implement comparison per the rule above; run the oracle
   (needs devenv tools + network): PASS (or documented-divergence notes —
   print every note, zero failures). Also run the live test; if the live
   selection differs from policy, that's a T1/T2 bug → fix there.
3. **README** — write the sections above.
4. **Full gate** — `cargo test --release` (all, incl. M1 oracle),
   `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
   `cargo build --release`.
5. **Commit** — `git -c commit.gpgsign=false commit -m "test: live --update test + M2 differential oracle; docs: update README"`.

---

## Review Focus (controller checklist, inline reviews)

1. **Faithful versioning port** — compare `src/versioning.rs` against the
   semantics listed in T1 (especially: shorter-release-is-BIGGER, suffix
   doesn't destabilize, isCompatible = suffix+length, commit-hash exclusion
   with the pure-digit exception).
2. **No bump of pinned/tagless/non-semver refs** — walk `resolve_target`;
   confirm the update branch is unreachable for `r.digest.is_some()`,
   tagless, and unparsable tags; confirm re-pin behavior unchanged.
3. **Fail-loud tag listing** — `list_tags` error under `--update` ⇒
   `unresolved` + exit 1; the failing ref stays as written while other
   resolvable refs in the file are still written (M1 partial-write
   semantics, adjudicated 2026-11-05 — the implementer flagged this against
   an earlier absolute "file untouched" wording); dry-run writes nothing.
4. **Cache correctness** — per-`api_repo()` key; error path stores nothing;
   `digest` flow untouched (all M1 wiremock tests green unmodified).
5. **Byte-exact output** — updated lines change only the token; trailing
   comments/quotes preserved (e2e fixture assertion with a trailing
   comment line); second run zero changes.
6. **Oracle divergence rule is one-directional** — only `new_tag > old_tag`
   with both stable+compatible is excused; equality and `new < old` are
   hard failures.

## Self-review (controller, before dispatch)

- Spec conformance: Update policy ✓ (highest stable, docker versioning,
  same-major default, `--update-major`), Re-pin semantics ✓ (pinned never
  bump), Pin format ✓ (tag in value, no comment — spec-decided), exit codes
  ✓ (only `unresolved` ⇒ 1), Docker-Hub-only ✓ (other hosts skip as in M1).
- Owner decisions honored: no minimum release age (M4) ✓ — oracle rule
  documents it as the one excused divergence; small steps ✓ (5 bounded
  tasks, each TDD, each commits independently).
- Dependency discipline: no new crates ✓ (manual parse; `serde_json`
  already a dep; no regex/futures).
- Old pipeline untouched ✓; M2 oracle reuses M1 scaffolding ✓.
- Known risk: live tag drift between the old and new run (seconds apart) —
  the divergence rule absorbs same-direction drift; opposite-direction
  drift would indicate a bug and fails loudly.
