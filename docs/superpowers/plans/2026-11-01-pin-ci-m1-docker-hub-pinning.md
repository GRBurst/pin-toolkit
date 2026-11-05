# pin-ci M1: Docker Hub image pinning — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Rust CLI (`pin-ci`) that rewrites image references in GitLab CI files from `image:tag` to `image:tag@sha256:<digest>` for Docker Hub images, leaving everything else byte-identical.

**Architecture:** Five single-responsibility modules — `image_ref` (parse/classify image references), `discover` (hand-rolled line scanner that locates image values under `image:`/`services:` keys), `registry` (async v2-registry client: anonymous → 401 → bearer token → digest from `Docker-Content-Digest` header, with GET+sha256 fallback), `edit` (byte-exact span splicing), `report` (human/JSON rendering). CLI orchestration in `lib.rs`/`main.rs`. No YAML parser dependency: the scanner's pattern set is fixed by GitLab CI's grammar for these two keys, and a full parser's position API adds risk without adding capability.

**Tech Stack:** Rust 2021, tokio (rt-multi-thread, macros), reqwest 0.12 (rustls-tls, no default features), clap 4 (derive), serde 1 + serde_json 1, sha2 0.10, wiremock 0.6 (dev).

**Spec:** `docs/superpowers/specs/2026-11-01-pin-ci-design.md` (this plan implements milestone M1; M2/M3/M4 are out of scope here)

## Global Constraints

Copied from the spec — every task's requirements implicitly include these:

- Single static Rust binary, package and binary name `pin-ci`. No runtime dependency.
- M1 pins Docker Hub images only. Any reference whose host is not Docker Hub (`None`/`docker.io`/`index.docker.io`) is reported `skipped(registry not enabled)`, never resolved, never an error.
- Pin-only: tags are never changed. The rewrite is `original-as-written` + `@sha256:<digest>`; an existing `@sha256:<old>` has its digest replaced. Tagless `image: alpine` → `alpine@sha256:<digest>` (no `:latest` inserted).
- Edits are byte-exact: lines without a target are unchanged; quotes, indentation, and trailing comments on target lines are preserved.
- Idempotent: a second run on an already-pinned, unchanged repo produces zero file changes.
- Already-pinned references are verified against the registry: if the registry digest differs (tag re-pushed), the digest is updated (tag unchanged), immediately — no release-age guard.
- Statuses: `pinned` (changed), `ok` (already correct), `skipped(<reason>)`, `unresolved(<reason>)`. Exit code: `0` if no `unresolved` and no IO error; `1` otherwise.
- Credentials: env `DOCKERHUB_USER`/`DOCKERHUB_TOKEN` first, else `~/.docker/config.json` (`auths` keys `https://index.docker.io/v1/`, `https://docker.io`, `docker.io`). Nothing secret written to files or logs.
- The old pipeline files (`pin-ci-images.sh`, `extract-docker-pins.py`, `config.js`) are never modified.
- Unit tests live inline (`#[cfg(test)] mod tests`) in each module; cross-module and wiremock tests live in `tests/`.
- Commits: one per task, messages `feat(...)` / `test(...)` / `docs(...)`.

## Review Focus

Failure modes the spec implies but whose tests must be pinned explicitly:

1. **Trailing comment on a target line** (`image: alpine:3.20   # base`) — the comment must survive the rewrite. → Task 3 test `trailing_comment_preserved`, Task 4 test `edit_preserves_trailing_comment`.
2. **Same image on multiple lines** — every occurrence gets the same digest in one run. → Task 6 e2e test `same_image_multiple_lines`.
3. **Re-run on pinned file** — zero changes, all `ok`. → Task 6 e2e test `second_run_is_noop`.
4. **Unresolvable reference mixed with resolvable ones** — resolvable ones are still written to the file, unresolved reported, exit code 1, no panic. → Task 6 e2e test `unresolved_mixes_with_pinned`.
5. **Non-hub host (`ghcr.io/...`, `registry:5000/...`)** — `skipped`, not `unresolved`, exit code stays 0. → Task 6 e2e test `non_hub_is_skipped_not_error`.

## Files

| File | Task | Responsibility |
|---|---|---|
| `Cargo.toml` | 1 | package `pin-ci`, deps |
| `src/main.rs` | 1 stub, 6 real | CLI (clap), exit code |
| `src/lib.rs` | 1–6 | module tree; `PinOptions`, `pin_file`, `pin_files`, `exit_code`, `client_from_env` |
| `src/image_ref.rs` | 1 | `ImageRef` parse / classify / format |
| `src/discover.rs` | 2–3 | `Target`, `discover(text) -> Vec<Target>` |
| `src/edit.rs` | 4 | `apply(text, targets, new_by_start) -> (String, usize)` |
| `src/report.rs` | 4 | `RefResult`, `Report`, human + JSON render |
| `src/registry.rs` | 5 | `Creds`, `RegistryClient`, `RegError` |
| `tests/mock_registry.rs` | 5 | wiremock tests for the registry client |
| `tests/e2e.rs` | 6 | file-level tests with mock registry |
| `tests/fixtures/*.yml` | 2,3,6 | CI fixtures loaded via `include_str!` |
| `tests/live.rs` | 7 | opt-in real Docker Hub tests |
| `tests/oracle.rs` | 7 | opt-in differential test vs old pipeline |
| `README.md` | 7 | usage, env vars, exit codes, M1 scope |

---

### Task 1: Crate scaffold + `image_ref`

**Files:**
- Create: `Cargo.toml`, `src/lib.rs`, `src/main.rs`, `src/image_ref.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct ImageRef {
      pub host: Option<String>,   // None | "docker.io" | "ghcr.io" | "reg:5000" ...
      pub repo: String,           // as written after host: "alpine", "library/alpine", "org/img"
      pub tag: Option<String>,    // None => latest
      pub digest: Option<String>, // "sha256:<64 hex>"
  }
  impl ImageRef {
      pub fn parse(input: &str) -> Result<ImageRef, String>;
      pub fn host_str(&self) -> &str;          // "docker.io" when host is None
      pub fn is_docker_hub(&self) -> bool;     // host None, "docker.io", "index.docker.io"
      pub fn effective_tag(&self) -> &str;     // "latest" when tag is None
      pub fn api_repo(&self) -> String;        // hub short name "alpine" => "library/alpine"; else repo
      pub fn base_url_key(&self) -> &str;      // always host_str() for M1
  }
  ```
  `src/lib.rs`: `pub mod image_ref;` (later tasks append their mods).
  `src/main.rs` (stub): `fn main() { eprintln!("pin-ci: not implemented yet"); std::process::exit(1); }`

- [ ] **Step 1: Initialize the crate**

```bash
cargo init --name pin-ci .
```
This creates `Cargo.toml` and `src/main.rs` in the repo root. Add to `Cargo.toml` `[dependencies]`: `clap = { version = "4", features = ["derive"] }`, `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`, `sha2 = "0.10"`, `tokio = { version = "1", features = ["rt-multi-thread", "macros"] }`, `reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }`. Add `[dev-dependencies]`: `wiremock = "0.6"`. Remove the auto-generated hello-world body from `src/main.rs` (replace with the stub above). Create `src/lib.rs` containing `pub mod image_ref;`.

- [ ] **Step 2: Write the failing tests** — inline `#[cfg(test)] mod tests` in `src/image_ref.rs`:

```rust
const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn parse_table() {
    let cases: &[(&str, Option<&str>, &str, Option<&str>, Option<&str>)] = &[
        ("alpine", None, "alpine", None, None),
        ("alpine:3.20", None, "alpine", Some("3.20"), None),
        ("docker.io/alpine:3.20", Some("docker.io"), "alpine", Some("3.20"), None),
        ("library/alpine:3.20", None, "library/alpine", Some("3.20"), None),
        ("ghcr.io/org/img:v1", Some("ghcr.io"), "org/img", Some("v1"), None),
        ("reg.example.com:5000/a/b:1", Some("reg.example.com:5000"), "a/b", Some("1"), None),
        ("alpine:3.20@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", None, "alpine", Some("3.20"), Some(DIGEST)),
    ];
    for (input, host, repo, tag, digest) in cases {
        let r = ImageRef::parse(input).unwrap();
        assert_eq!(r.host.as_deref(), *host, "host: {input}");
        assert_eq!(r.repo, *repo, "repo: {input}");
        assert_eq!(r.tag.as_deref(), *tag, "tag: {input}");
        assert_eq!(r.digest.as_deref(), *digest, "digest: {input}");
    }
}

#[test]
fn parse_rejects_garbage() {
    for bad in ["", "a b", "alpine:3.20@notadigest", "alpine:3.20@sha256:zzz", "@sha256:0000000000000000000000000000000000000000000000000000000000000000"] {
        assert!(ImageRef::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn classification() {
    assert!(ImageRef::parse("alpine").unwrap().is_docker_hub());
    assert!(ImageRef::parse("docker.io/x/y:1").unwrap().is_docker_hub());
    assert!(!ImageRef::parse("ghcr.io/x/y:1").unwrap().is_docker_hub());
    assert!(!ImageRef::parse("reg:5000/x/y:1").unwrap().is_docker_hub());
    assert_eq!(ImageRef::parse("alpine").unwrap().api_repo(), "library/alpine");
    assert_eq!(ImageRef::parse("library/alpine").unwrap().api_repo(), "library/alpine");
    assert_eq!(ImageRef::parse("org/img").unwrap().api_repo(), "org/img");
    assert_eq!(ImageRef::parse("alpine").unwrap().effective_tag(), "latest");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib image_ref`
Expected: compile FAIL — `cannot find struct ImageRef`.

- [ ] **Step 4: Implement `ImageRef` in `src/image_ref.rs`**

Parse algorithm (exact):
1. Reject empty input or any whitespace character in input.
2. If input contains `@`: split at the LAST `@`. The part after must start with `sha256:` followed by exactly 64 lowercase hex chars, else `Err("invalid digest")`. The part before is parsed as host/repo:tag.
3. Registry host: take the first `/`-separated component of the remainder. It is a host iff it contains `.` or `:` or equals `localhost`; otherwise host = `None` and the component is part of the repo.
4. Tag: in the remainder after the host, find the last `:` that appears after the last `/`; the text after it is the tag, else tag = `None`.
5. Reject if repo is empty.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib image_ref`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/main.rs src/image_ref.rs
git commit -m "feat: scaffold pin-ci crate with ImageRef parsing"
```

---

### Task 2: `discover` — scalar `image:` / `services:` lines

**Files:**
- Create: `src/discover.rs`, `tests/fixtures/ci-simple.yml`
- Modify: `src/lib.rs` (append `pub mod discover;`)

**Interfaces:**
- Consumes: `ImageRef::parse` (Task 1)
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct Target {
      pub line: usize,    // 0-based line index in the file text
      pub start: usize,   // byte offset where the value token begins (incl. quotes)
      pub end: usize,     // byte offset where the value token ends (excl. comments)
      pub value: String,  // the value with quotes stripped
  }
  pub fn discover(text: &str) -> Vec<Target>;  // sorted by (line, start)
  ```

  Fixture `tests/fixtures/ci-simple.yml` (exact content):
  ```yaml
  # comment line
  image: alpine:3.20

  default:
    image: ubuntu:24.04

  build:
    image: "node:20"
    services: redis:7

  test:
    image: alpine:3.20
  ```

- [ ] **Step 1: Write the failing tests** — inline in `src/discover.rs`:

```rust
const SIMPLE: &str = include_str!("../tests/fixtures/ci-simple.yml");

#[test]
fn discovers_scalar_image_and_services() {
    let t = discover(SIMPLE);
    let got: Vec<(usize, &str)> = t.iter().map(|x| (x.line, x.value.as_str())).collect();
    assert_eq!(got, vec![
        (1, "alpine:3.20"),    // top-level image
        (4, "ubuntu:24.04"),   // default.image
        (7, "node:20"),        // build.image (quoted; value unquoted)
        (8, "redis:7"),        // build.services scalar
        (11, "alpine:3.20"),   // test.image (same image twice)
    ]);
    // spans must cover the exact written token (quotes included in span, not value)
    for x in &t {
        let token = &SIMPLE[x.start..x.end];
        let unquoted = token.trim_matches(|c| c == '"' || c == '\'');
        assert_eq!(unquoted, x.value);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib discover`
Expected: compile FAIL — `cannot find function discover`.

- [ ] **Step 3: Implement the scanner (scalar subset) in `src/discover.rs`**

Algorithm (exact):
1. Precompute for each line: its byte offset in `text`, its indentation (count of leading spaces — reject/ignore tabs: a line indented with tabs yields no targets), and whether it is a comment line (first non-space char is `#`).
2. Walk lines maintaining `top: Option<&str>` (the current top-level key, from an indent-0 `key:` line) and `in_block: bool`.
   - An indent-0 line `key:` or `key: value` sets `top` (for `key:` with no value, `in_block = true` until the next indent-0 line; for `key: value`, `in_block = false`).
   - `top` is `Some("default")` or any other name; only the *name* matters (any non-`default` top-level key is treated as a job block).
3. A line is a candidate if it matches `^(\s*)(image|services):\s*(.*)$` (or `^(\s*)(image|services):$` for empty value) AND:
   - its indentation is > 0, it is not a comment line, and
   - (`top == Some("default")` or `top` is some non-default key) — i.e. the line is one level inside a top-level block, OR its indentation is 0 (top-level `image:`).
   Deeper nesting (indent beyond the first child level of the block, e.g. inside `rules:`/`needs:`/`script:`) must NOT be a candidate. Track `child_indent: Option<usize>` = indentation of the first non-empty line inside the current top-level block; candidates are only lines whose indentation equals `child_indent` (for `default`/job blocks) or 0 (top-level).
4. Value handling for Task 2 (scalar subset): take the rest-of-line after `key:`. If empty → no target (sequences are Task 3). Otherwise:
   - If it starts with `"` or `'`: the token runs to the matching closing quote on the same line (no escaped-quote handling in M1); if no closing quote, no target.
   - If it starts with `[` or `|` or `>` → no target (Task 3 handles `[` flow; block scalars are never targets).
   - Else (bare): the token runs from the first non-space char to end-of-line, cut at the first occurrence of ` #` (space + hash); trim trailing spaces.
   - Strip inline comment per the rules above; `value` = token with quotes stripped.
5. Keep the target iff `ImageRef::parse(&value)` succeeds; otherwise no target (silently — a non-image scalar under `image:` is not M1's concern; it is still not an error).
6. Return targets sorted by `(line, start)`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib discover`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/discover.rs src/lib.rs tests/fixtures/ci-simple.yml
git commit -m "feat: discover scalar image/services references in CI files"
```

---

### Task 3: `discover` — sequences, flow lists, comments, non-targets

**Files:**
- Modify: `src/discover.rs`
- Create: `tests/fixtures/ci-rich.yml`

**Interfaces:**
- Consumes: Task 2 scanner
- Produces: `discover` now also returns targets for: block-sequence items under `services:`/`image:` (`- value` and `- name: value`, dash at block indent or one level deeper), flow sequences (`services: [a:1, b:2]`), quoted items, `image:` as a mapping (`name:` child). Comment lines and `include:` blocks are never targets.

  Fixture `tests/fixtures/ci-rich.yml` (exact content):
  ```yaml
  # rich fixture
  default:
    image: alpine:3.20   # trailing comment
    services: [docker:24, redis:7]

  build:
    image:
      name: "node:20"
      entrypoint: node
    services:
      - docker:24
      - name: redis:7
        entrypoint: /x

  include:
    - project: 'group/proj'
      ref: v1.0.0

  no-image-job:
    script:
      - echo hi
  ```

- [ ] **Step 1: Write the failing tests** — append to `src/discover.rs` tests:

```rust
const RICH: &str = include_str!("../tests/fixtures/ci-rich.yml");

#[test]
fn discovers_sequences_flow_and_mapping_forms() {
    let t = discover(RICH);
    let got: Vec<(usize, &str)> = t.iter().map(|x| (x.line, x.value.as_str())).collect();
    assert_eq!(got, vec![
        (2, "alpine:3.20"),   // default.image with trailing comment
        (3, "docker:24"),     // flow item 1
        (3, "redis:7"),       // flow item 2 (same line, later span)
        (7, "node:20"),       // image mapping name
        (10, "docker:24"),    // sequence scalar
        (11, "redis:7"),      // sequence mapping name
    ]);
}

#[test]
fn include_block_and_script_items_are_not_targets() {
    let t = discover(RICH);
    assert!(t.iter().all(|x| x.value != "v1.0.0"));
    assert!(t.iter().all(|x| x.value != "echo hi"));
}

#[test]
fn trailing_comment_preserved_in_spans() {
    let t = discover(RICH);
    let first = &t[0];
    assert_eq!(&RICH[first.start..first.end], "alpine:3.20");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib discover`
Expected: FAIL — sequence/flow/mapping items missing from results.

- [ ] **Step 3: Extend the scanner**

Extensions (exact):
1. **Block sequences.** When a candidate `key:` line has an empty value and the key is `services` or `image`, enter sequence mode for the block: subsequent lines matching `^(\s*)-\s+(.*)$` whose indentation is ≥ the key's indentation and < the next top-level key's indentation are items.
   - Item rest is a bare/quoted scalar that parses as `ImageRef` → target.
   - Item rest is `name:\s*(scalar)` (bare or quoted) → target if it parses.
   - Deeper lines under an item (`entrypoint: ...`) are ignored.
   - Sequence mode ends at the next line with indentation ≤ the key's indentation (that is not itself an item), or at the next top-level key.
2. **`image:` as mapping.** Same mechanism as `default:`/job blocks: if `image:` has an empty value, its child block's `name:` scalar (at child indent) is a target when it parses. (`entrypoint:` and other keys are ignored.)
3. **Flow sequences.** A candidate value starting with `[` and containing `]`: split the inside on top-level commas (M1: no nested braces/brackets inside a flow list — if any `{`, `}`, or nested `[`/`]` appears inside, no targets for the whole line). Each comma item: trim, strip one layer of quotes, must parse as `ImageRef` to be a target; spans point at the item's position within the original line (quotes included in span when quoted).
4. **Quoted scalars everywhere** (already in Task 2 value handling) must also apply to sequence items and flow items.
5. Comment lines (`#` first non-space char) never produce targets; a line whose key is preceded by `#` (e.g. `# image: alpine:3.20`) is a comment line and produces none.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib discover`
Expected: PASS (3 tests in the discover module total ≥ 3, including Task 2's).

- [ ] **Step 5: Commit**

```bash
git add src/discover.rs tests/fixtures/ci-rich.yml
git commit -m "feat: discover sequence, flow and mapping image forms"
```

---

### Task 4: `edit` + `report`

**Files:**
- Create: `src/edit.rs`, `src/report.rs`
- Modify: `src/lib.rs` (append `pub mod edit; pub mod report;`)

**Interfaces:**
- Consumes: `Target` (Task 2/3), `ImageRef` (Task 1)
- Produces:
  ```rust
  // edit.rs
  pub fn apply(text: &str, targets: &[Target], new_by_start: &std::collections::BTreeMap<usize, String>) -> (String, usize);
  // new_by_start: map Target.start -> replacement token (quotes preserved).
  // Returns (new_text, number of replacements actually applied).
  // Replacements with new == old token count 0. Offsets are applied end-first.
  pub fn splice_value(old_token: &str, ref_: &ImageRef, digest: &str) -> String;
  // old_token: as written (quotes optional). If old_token already ends with
  // "@sha256:<64hex>" the digest part is replaced; otherwise "@<digest>" is
  // appended to the quote-stripped value and re-quoted with the original style.
  // ref_ is used only to detect an existing digest via ref_.digest.is_some().

  // report.rs
  #[derive(Debug, Clone, serde::Serialize, PartialEq)]
  pub struct RefResult {
      pub file: String,   // as passed on the CLI
      pub line: usize,    // 1-based
      pub reference: String,
      pub old: String,
      pub new: Option<String>,
      pub status: String, // "pinned" | "ok" | "skipped" | "unresolved"
      #[serde(skip_serializing_if = "Option::is_none")]
      pub reason: Option<String>, // present for skipped/unresolved
  }
  #[derive(Debug, Clone, serde::Serialize)]
  pub struct Report {
      pub results: Vec<RefResult>,
      pub summary: Summary,
  }
  #[derive(Debug, Clone, serde::Serialize)]
  pub struct Summary { pub pinned: usize, pub ok: usize, pub skipped: usize, pub unresolved: usize }
  impl Report {
      pub fn new(results: Vec<RefResult>) -> Self;   // fills summary
      pub fn render_human(&self) -> String;
      pub fn to_json(&self) -> String;               // serde_json::to_string_pretty
  }
  ```
  Human format, one line per result, then a summary line (exact):
  ```
  ci/.gitlab-ci.yml:2  pinned  alpine:3.20 -> alpine:3.20@sha256:abc  # alpine:3.20
  ci/.gitlab-ci.yml:5  skipped(registry not enabled: ghcr.io)  ghcr.io/o/i:v1  # ghcr.io/o/i:v1
  ```
  i.e. `{file}:{line}  {status}{reason: (reason)}  {old} -> {new or "-"}  # {reference}`.
  Summary line: `pinned: {n}, ok: {n}, skipped: {n}, unresolved: {n}`.

- [ ] **Step 1: Write the failing tests** — inline in each module:

```rust
// edit.rs tests
#[test]
fn splice_appends_digest_bare() {
    let r = ImageRef::parse("alpine:3.20").unwrap();
    assert_eq!(splice_value("alpine:3.20", &r, DIGEST), format!("alpine:3.20@{DIGEST}"));
}

#[test]
fn splice_appends_digest_tagless() {
    let r = ImageRef::parse("alpine").unwrap();
    assert_eq!(splice_value("alpine", &r, DIGEST), format!("alpine@{DIGEST}"));
}

#[test]
fn splice_preserves_quotes() {
    let r = ImageRef::parse("node:20").unwrap();
    assert_eq!(splice_value("\"node:20\"", &r, DIGEST), format!("\"node:20@{DIGEST}\""));
}

#[test]
fn splice_replaces_existing_digest() {
    let old = format!("alpine:3.20@{DIGEST}");
    let r = ImageRef::parse(&old).unwrap();
    let other = "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    assert_eq!(splice_value(&old, &r, other), format!("alpine:3.20@{other}"));
}

#[test]
fn apply_splices_multiple_spans_end_first() {
    let text = "image: alpine:3.20\nservices: [alpine:3.20, redis:7]\n";
    let t = discover(text);
    let mut m = std::collections::BTreeMap::new();
    for x in &t { m.insert(x.start, splice_value(&text[x.start..x.end], &ImageRef::parse(&x.value).unwrap(), DIGEST)); }
    let (out, n) = apply(text, &t, &m);
    assert_eq!(n, 3);
    assert_eq!(out, format!("image: alpine:3.20@{DIGEST}\nservices: [alpine:3.20@{DIGEST}, redis:7@{DIGEST}]\n"));
}

#[test]
fn apply_is_noop_when_values_equal() {
    let text = "image: alpine:3.20\n";
    let t = discover(text);
    let m = std::collections::BTreeMap::from([(t[0].start, t[0].value.clone())]);
    let (out, n) = apply(text, &t, &m);
    assert_eq!(n, 0);
    assert_eq!(out, text);
}

#[test]
fn apply_preserves_trailing_comment() {
    let text = "image: alpine:3.20   # base image\nother: keep\n";
    let t = discover(text);
    let m = std::collections::BTreeMap::from([(t[0].start, format!("alpine:3.20@{DIGEST}"))]);
    let (out, _) = apply(text, &t, &m);
    assert_eq!(out, format!("image: alpine:3.20@{DIGEST}   # base image\nother: keep\n"));
}
```

`report.rs` tests:
```rust
#[test]
fn human_render_exact() {
    let r = Report::new(vec![
        RefResult { file: "ci/.gitlab-ci.yml".into(), line: 2, reference: "alpine:3.20".into(), old: "alpine:3.20".into(), new: Some("alpine:3.20@sha256:abc".into()), status: "pinned".into(), reason: None },
        RefResult { file: "ci/.gitlab-ci.yml".into(), line: 5, reference: "ghcr.io/o/i:v1".into(), old: "ghcr.io/o/i:v1".into(), new: None, status: "skipped".into(), reason: Some("registry not enabled: ghcr.io".into()) },
    ]);
    let expected = "ci/.gitlab-ci.yml:2  pinned  alpine:3.20 -> alpine:3.20@sha256:abc  # alpine:3.20\n\
                    ci/.gitlab-ci.yml:5  skipped (registry not enabled: ghcr.io)  ghcr.io/o/i:v1 -> -  # ghcr.io/o/i:v1\n\
                    pinned: 1, ok: 0, skipped: 1, unresolved: 0\n";
    assert_eq!(r.render_human(), expected);
}

#[test]
fn json_render_roundtrip() {
    let r = Report::new(vec![RefResult { file: "f".into(), line: 1, reference: "x".into(), old: "x".into(), new: None, status: "unresolved".into(), reason: Some("boom".into()) }]);
    let v: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
    assert_eq!(v["summary"]["unresolved"], 1);
    assert_eq!(v["results"][0]["reason"], "boom");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib`
Expected: compile FAIL — `cannot find function apply / splice_value / struct RefResult`.

- [ ] **Step 3: Implement `edit.rs`**

`apply`: collect `(start, end, new)` for targets whose replacement differs from the current token, sort by `start` descending, and splice each into the text. `splice_value` per the interface contract (detect existing digest by `ImageRef::parse(token_stripped).digest.is_some()` or by a trailing `@sha256:<64hex>` regex — use the parse, it is already exact).

- [ ] **Step 4: Implement `report.rs`** per the exact format above (`-> -` when `new` is `None`; reason rendered as ` (reason)` after the status).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS (all module tests: image_ref, discover, edit, report).

- [ ] **Step 6: Commit**

```bash
git add src/edit.rs src/report.rs src/lib.rs
git commit -m "feat: byte-exact edit splicing and report rendering"
```

---

### Task 5: `registry` — Docker Hub v2 client with bearer auth

**Files:**
- Create: `src/registry.rs`, `tests/mock_registry.rs`
- Modify: `src/lib.rs` (append `pub mod registry;`)

**Interfaces:**
- Consumes: `ImageRef` (Task 1)
- Produces:
  ```rust
  #[derive(Debug, Clone, Default)]
  pub struct Creds { pub dockerhub: Option<(String, String)> } // (user, pass)
  impl Creds {
      pub fn none() -> Self;
      // env user/pass take precedence over config.json; config.json read from
      // home/.docker/config.json when home is Some and exists.
      pub fn load_from(env_user: Option<String>, env_pass: Option<String>, home: Option<&std::path::Path>) -> Self;
  }

  pub struct RegistryClient { /* reqwest::Client + Creds + base URLs + token cache */ }
  impl RegistryClient {
      // base_urls: registry host key ("docker.io") -> base URL.
      // The default map (used by client_from_env) has "docker.io" -> "https://registry-1.docker.io".
      pub fn new(creds: Creds, base_urls: std::collections::HashMap<String, String>) -> Self;
      pub async fn digest(&self, r: &ImageRef) -> Result<String, RegError>; // "sha256:<hex>"
  }
  #[derive(Debug)]
  pub enum RegError {  // plain enum + manual Display impl; no thiserror dep
      Unresolved(String),  // 404, bad token response, ...
      Network(String),
  }
  ```
  Behavior contract (exact):
  1. `base = base_urls[r.host_str()]` (default map when absent: `docker.io` → `https://registry-1.docker.io`; other hosts have no base in M1 and are never sent here).
  2. `path = /v2/{api_repo}/manifests/{effective_tag}`; header `Accept: application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.index.v1+json`.
  3. `HEAD {base}{path}` anonymous. On 200: if the `Docker-Content-Digest` header is present → return it (must start with `sha256:`, else fall through to the hash fallback). If absent → `GET` the same path, return `sha256:` + sha256-hex of the body.
  4. On 401: parse `WWW-Authenticate` of form `Bearer realm="R",service="S",scope="SC"` (split on commas outside quotes; keys `realm`, `service`, `scope`; values are quoted strings). `GET {R}` with query `service={S}` and `scope={SC}` (use the scope from the header verbatim; assert it is `repository:{api_repo}:pull` — on mismatch, `Unresolved`), plus HTTP Basic auth when `creds.dockerhub` is `Some`. Expect JSON `{"token": ...}` or `{"access_token": ...}`; otherwise `Unresolved`. Retry the HEAD with `Authorization: Bearer {token}`. Cache the token in memory keyed by `scope` (a later call with the same scope must NOT re-fetch it — enforce in a test).
  5. Retry 200 → step 3's header/hash rules. Retry 401 again → `Unresolved("authentication failed")`.
  6. Any 404 (initial or retry) → `Unresolved("{api_repo}:{tag} not found")`. Other statuses → `Network("{status} at {path}")`.

- [ ] **Step 1: Write the failing tests** — `tests/mock_registry.rs`:

```rust
use pin_ci::image_ref::ImageRef;
use pin_ci::registry::{Creds, RegError, RegistryClient};
use wiremock::matchers::{header, header_exists, method, path, query};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn client(server: &MockServer, creds: Creds) -> RegistryClient {
    RegistryClient::new(creds, std::collections::HashMap::from([("docker.io".to_string(), server.uri())]))
}

fn alpine() -> ImageRef {
    ImageRef::parse("alpine:3.20").unwrap()
}

async fn mount_alpine_digest(server: &MockServer, code: u16, with_auth: bool) {
    let mut m = Mock::given(method("HEAD")).and_path("/v2/library/alpine/manifests/3.20");
    if with_auth {
        m = m.and(header("authorization", "Bearer tok123"));
    } else {
        m = m.and(header_exists("authorization", false));
    }
    m.respond_with(ResponseTemplate::new(code).insert_header("docker-content-digest", DIGEST))
        .mount(server).await;
}

#[tokio::test]
async fn anon_digest_from_header() {
    let server = MockServer::start().await;
    mount_alpine_digest(&server, 200, false).await;
    let c = client(&server, Creds::none());
    assert_eq!(c.digest(&alpine()).await.unwrap(), DIGEST);
}

#[tokio::test]
async fn bearer_flow_on_401() {
    let server = MockServer::start().await;
    mount_alpine_digest(&server, 200, true).await;
    Mock::given(method("HEAD"))
        .and_path("/v2/library/alpine/manifests/3.20")
        .and(header_exists("authorization", false))
        .respond_with(ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            format!("Bearer realm=\"{}\",service=\"registry.mock\",scope=\"repository:library/alpine:pull\"", format!("{}/token", server.uri())),
        ))
        .mount(&server).await;
    let token_calls = Mock::given(method("GET"))
        .and_path("/token")
        .and(query("service", "registry.mock"))
        .and(query("scope", "repository:library/alpine:pull"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "tok123"})))
        .expect(1)
        .mount(&server).await;
    let c = client(&server, Creds::none());
    assert_eq!(c.digest(&alpine()).await.unwrap(), DIGEST);
    let _ = token_calls;
}

#[tokio::test]
async fn creds_send_basic_auth_to_token_endpoint() {
    let server = MockServer::start().await;
    mount_alpine_digest(&server, 200, true).await;
    Mock::given(method("HEAD"))
        .and_path("/v2/library/alpine/manifests/3.20")
        .and(header_exists("authorization", false))
        .respond_with(ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            format!("Bearer realm=\"{}\",service=\"registry.mock\",scope=\"repository:library/alpine:pull\"", format!("{}/token", server.uri())),
        ))
        .mount(&server).await;
    Mock::given(method("GET"))
        .and_path("/token")
        .and(header("authorization", "Basic dXNlcjpwYXNz")) // base64("user:pass")
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "tok123"})))
        .expect(1)
        .mount(&server).await;
    let c = client(&server, Creds { dockerhub: Some(("user".into(), "pass".into())) });
    assert_eq!(c.digest(&alpine()).await.unwrap(), DIGEST);
}

#[tokio::test]
async fn token_is_cached_per_scope() {
    let server = MockServer::start().await;
    mount_alpine_digest(&server, 200, true).await;
    Mock::given(method("HEAD"))
        .and_path("/v2/library/alpine/manifests/3.20")
        .and(header_exists("authorization", false))
        .respond_with(ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            format!("Bearer realm=\"{}\",service=\"registry.mock\",scope=\"repository:library/alpine:pull\"", format!("{}/token", server.uri())),
        ))
        .expect(1)
        .mount(&server).await;
    let _tok = Mock::given(method("GET"))
        .and_path("/token")
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "tok123"})))
        .expect(1)
        .mount(&server).await;
    let c = client(&server, Creds::none());
    let a = ImageRef::parse("alpine:3.20").unwrap();
    let b = ImageRef::parse("alpine:3.21").unwrap(); // different tag, same scope
    Mock::given(method("HEAD"))
        .and_path("/v2/library/alpine/manifests/3.21")
        .and(header("authorization", "Bearer tok123"))
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", DIGEST))
        .mount(&server).await;
    assert_eq!(c.digest(&a).await.unwrap(), DIGEST);
    assert_eq!(c.digest(&b).await.unwrap(), DIGEST); // must reuse tok123, no 2nd /token
}

#[tokio::test]
async fn falls_back_to_body_sha256_when_header_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and_path("/v2/library/alpine/manifests/3.20")
        .respond_with(ResponseTemplate::new(200).set_body_raw("hello", "application/octet-stream"))
        .mount(&server).await;
    Mock::given(method("HEAD"))
        .and_path("/v2/library/alpine/manifests/3.20")
        .respond_with(ResponseTemplate::new(200)) // no docker-content-digest header
        .mount(&server).await;
    let c = client(&server, Creds::none());
    assert_eq!(c.digest(&alpine()).await.unwrap(), "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
}

#[tokio::test]
async fn not_found_is_unresolved() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and_path("/v2/library/mystery/manifests/1.0")
        .respond_with(ResponseTemplate::new(404))
        .mount(&server).await;
    let c = client(&server, Creds::none());
    let err = c.digest(&ImageRef::parse("mystery:1.0").unwrap()).await.unwrap_err();
    match err {
        RegError::Unresolved(msg) => assert!(msg.contains("library/mystery")),
        other => panic!("expected Unresolved, got {other:?}"),
    }
}

#[test]
fn creds_env_precedence_and_config_json() {
    let home = tempfile_dir(); // helper: std::env::temp_dir().join("pin-ci-creds-test-..."), created fresh
    std::fs::create_dir_all(home.join(".docker")).unwrap();
    std::fs::write(home.join(".docker/config.json"),
        r#"{"auths":{"https://index.docker.io/v1/":{"username":"u","password":"p"}}}"#).unwrap();
    let c = Creds::load_from(None, None, Some(&home));
    assert_eq!(c.dockerhub.as_ref().unwrap(), &("u".to_string(), "p".to_string()));
    let c = Creds::load_from(Some("eu".into()), Some("ep".into()), Some(&home));
    assert_eq!(c.dockerhub.as_ref().unwrap(), &("eu".to_string(), "ep".to_string()));
    std::fs::remove_dir_all(&home).ok();
}
```
(`tempfile_dir()`: small local helper in the test file — `std::env::temp_dir().join(format!("pin-ci-creds-{}", std::process::id()))` + `std::fs::create_dir_all`; add it, the test above is the contract.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test mock_registry`
Expected: compile FAIL — `cannot find crate/module registry` (or unresolved imports).

- [ ] **Step 3: Implement `src/registry.rs`** per the behavior contract above.

Implementation notes: reqwest `Client` with `danger_accept_invalid_certs` off, timeout 30s; token cache = `tokio::sync::Mutex<HashMap<String, String>>` (client is used concurrently from M1's orchestrator); `Creds::load_from` reads config.json with `serde_json`, checks the three auths key variants in order, and returns `dockerhub: None` when nothing is found.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test mock_registry && cargo test --lib`
Expected: PASS (7 tests in mock_registry, all lib tests still green).

- [ ] **Step 5: Commit**

```bash
git add src/registry.rs src/lib.rs tests/mock_registry.rs
git commit -m "feat: registry v2 client with bearer auth and digest fallback"
```

---

### Task 6: Orchestration + CLI

**Files:**
- Modify: `src/lib.rs` (add orchestration), `src/main.rs` (real CLI)
- Create: `tests/e2e.rs`, `tests/fixtures/ci-e2e.yml`

**Interfaces:**
- Consumes: all of Tasks 1–5
- Produces (in `src/lib.rs`):
  ```rust
  #[derive(Debug, Clone)]
  pub struct PinOptions { pub dry_run: bool, pub json: bool, pub concurrency: usize }

  pub fn client_from_env() -> RegistryClient;  // Creds::load_from(env vars, $HOME) + default base map

  // Resolves all targets in one file against client, applies edits (unless
  // dry_run), writes the file only when ≥1 edit was applied.
  pub async fn pin_file(path: &std::path::Path, opts: &PinOptions, client: &RegistryClient) -> Result<Vec<RefResult>, String>;

  pub async fn pin_files(paths: &[std::path::PathBuf], opts: &PinOptions, client: &RegistryClient) -> Result<Report, String>;
  // Missing file => Err(String) (main maps that to exit 1).

  pub fn exit_code(report: &Report) -> i32;  // 1 if summary.unresolved > 0 else 0
  ```
  Per-target resolution logic in `pin_file` (exact):
  1. `ImageRef::parse(value)` fails → `skipped(unparseable reference)`.
  2. `!is_docker_hub()` → `skipped(registry not enabled: {host_str})`.
  3. `client.digest(&r)` → `Err(RegError::Unresolved(m))` → `unresolved(m)`; `Err(RegError::Network(m))` → `unresolved(m)`.
  4. Success: if `r.digest == Some(digest)` → `ok` (no edit); else new token = `edit::splice_value(token, &r, digest)`, status `pinned`.
  5. Resolutions for one file run concurrently (tokio semaphore = `opts.concurrency`); results sorted by line.
  - `main.rs`: clap — `files: Vec<String>` (default `["ci/.gitlab-ci.yml"]`), `--dry-run`, `--json`, `--concurrency` (default 8). Runs `pin_files`, prints `render_human()` or `to_json()`, `std::process::exit(exit_code(&report))` (or 1 on `Err`, message to stderr).

  Fixture `tests/fixtures/ci-e2e.yml` (exact content):
  ```yaml
  image: alpine:3.20
  ghcr-job:
    image: ghcr.io/org/img:v1
  missing-job:
    image: mystery:1.0
  dup:
    image: alpine:3.20
  ```

- [ ] **Step 1: Write the failing tests** — `tests/e2e.rs`:

```rust
use std::path::PathBuf;
use pin_ci::{exit_code, pin_files, PinOptions, RegistryClient};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const D1: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const D2: &str = "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
const FIXTURE: &str = include_str!("fixtures/ci-e2e.yml");

async fn mock_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("HEAD")).and_path("/v2/library/alpine/manifests/3.20")
        .respond_with(ResponseTemplate::new(200).insert_header("docker-content-digest", D1)).mount(&s).await;
    Mock::given(method("HEAD")).and_path("/v2/library/mystery/manifests/1.0")
        .respond_with(ResponseTemplate::new(404)).mount(&s).await;
    s
}

fn write_fixture(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("ci-e2e.yml");
    std::fs::write(&p, FIXTURE).unwrap();
    p
}

#[tokio::test]
async fn pins_and_reports_all_states() {
    let s = mock_server().await;
    let client = RegistryClient::new(Default::default(), std::collections::HashMap::from([("docker.io".to_string(), s.uri())]));
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = write_fixture(&dir);
    let opts = PinOptions { dry_run: false, json: false, concurrency: 4 };
    let report = pin_files(&[p.clone()], &opts, &client).await.unwrap();
    let out = std::fs::read_to_string(&p).unwrap();
    let expected = format!("image: alpine:3.20@{D1}\nghcr-job:\n  image: ghcr.io/org/img:v1\nmissing-job:\n  image: mystery:1.0\ndup:\n  image: alpine:3.20@{D1}\n");
    assert_eq!(out, expected);
    let by_status: std::collections::HashMap<&str, usize> = report.results.iter().map(|r| (r.status.as_str(), 1)).fold(std::collections::HashMap::new(), |mut m, (s, n)| { *m.entry(s).or_insert(0) += n; m });
    assert_eq!(by_status.get("pinned"), Some(&2));   // two alpine lines, same digest
    assert_eq!(by_status.get("skipped"), Some(&1));  // ghcr
    assert_eq!(by_status.get("unresolved"), Some(&1)); // mystery 404
    assert_eq!(exit_code(&report), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn second_run_is_noop() {
    let s = mock_server().await;
    let client = RegistryClient::new(Default::default(), std::collections::HashMap::from([("docker.io".to_string(), s.uri())]));
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = write_fixture(&dir);
    let opts = PinOptions { dry_run: false, json: false, concurrency: 4 };
    let _ = pin_files(&[p.clone()], &opts, &client).await.unwrap();
    let after_first = std::fs::read_to_string(&p).unwrap();
    let report2 = pin_files(&[p.clone()], &opts, &client).await.unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), after_first);
    assert!(report2.results.iter().all(|r| r.status == "ok" || r.status == "skipped" || r.status == "unresolved"));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn dry_run_writes_nothing() {
    let s = mock_server().await;
    let client = RegistryClient::new(Default::default(), std::collections::HashMap::from([("docker.io".to_string(), s.uri())]));
    let dir = std::env::temp_dir().join(format!("pin-ci-e2e3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = write_fixture(&dir);
    let opts = PinOptions { dry_run: true, json: false, concurrency: 4 };
    let report = pin_files(&[p.clone()], &opts, &client).await.unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), FIXTURE);
    assert_eq!(report.summary.pinned, 2);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn missing_file_is_error() {
    let s = mock_server().await;
    let client = RegistryClient::new(Default::default(), std::collections::HashMap::from([("docker.io".to_string(), s.uri())]));
    let opts = PinOptions { dry_run: false, json: false, concurrency: 4 };
    let r = pin_files(&[PathBuf::from("/nonexistent/ci.yml")], &opts, &client).await;
    assert!(r.is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test e2e`
Expected: compile FAIL — `cannot find pin_files / PinOptions / exit_code`.

- [ ] **Step 3: Implement `pin_file`, `pin_files`, `exit_code`, `client_from_env` in `src/lib.rs` and the real `src/main.rs`** per the contract above.

- [ ] **Step 4: Run all tests**

Run: `cargo test`
Expected: PASS (lib + mock_registry + e2e).

- [ ] **Step 5: Manual smoke**

```bash
cargo run -- --help
```
Expected: usage shows `FILES` (default `ci/.gitlab-ci.yml`), `--dry-run`, `--json`, `--concurrency`.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/main.rs tests/e2e.rs tests/fixtures/ci-e2e.yml
git commit -m "feat: pin-ci CLI with orchestration, dry-run, json report, exit codes"
```

---

### Task 7: Live tests, oracle harness, README

**Files:**
- Create: `tests/live.rs`, `tests/oracle.rs`, `README.md`

**Interfaces:**
- Consumes: everything; the old pipeline `pin-ci-images.sh` (run as-is from a temp cwd)
- Produces: opt-in test gates + documentation. No production code changes.

- [ ] **Step 1: Write `tests/live.rs`** (env-gated; early-return with a printed notice when the gate env is absent):

```rust
// Gate: run with `PIN_CI_LIVE=1 cargo test --test live`
#[tokio::test]
async fn live_docker_hub_alpine() {
    if std::env::var("PIN_CI_LIVE").is_err() { eprintln!("SKIP live (set PIN_CI_LIVE=1)"); return; }
    use pin_ci::image_ref::ImageRef;
    let c = client_from_env();
    let d1 = c.digest(&ImageRef::parse("alpine:3.20").unwrap()).await.unwrap();
    let d2 = c.digest(&ImageRef::parse("alpine:3.20").unwrap()).await.unwrap();
    assert_eq!(d1, d2);
    assert!(d1.starts_with("sha256:") && d1.len() == 7 + 64, "{d1}");
}

#[tokio::test]
async fn live_docker_hub_ubuntu() {
    if std::env::var("PIN_CI_LIVE").is_err() { eprintln!("SKIP live (set PIN_CI_LIVE=1)"); return; }
    use pin_ci::image_ref::ImageRef;
    let d = client_from_env().digest(&ImageRef::parse("ubuntu:24.04").unwrap()).await.unwrap();
    assert!(d.starts_with("sha256:") && d.len() == 71, "{d}");
}
```

- [ ] **Step 2: Write `tests/oracle.rs`** — the differential test against the old pipeline (the owner's required gate):

Behavior (exact):
1. Gate: env `PIN_CI_ORACLE=1`, else print + return. Also require `renovate` and `crane` on `PATH` (check with `which`-style std::process::Command("sh", ["-c", "command -v renovate"]); on absence print + return).
2. Build a temp dir containing `ci/.gitlab-ci.yml` with content:
   ```yaml
   image: alpine:3.20
   build:
     image: ubuntu:24.04
     services:
       - docker:24
   ```
   (all Docker Hub tags old enough that the old tool's 14-day `minimumReleaseAge` cannot skew it — if digests ever disagree, first suspect a re-pushed tag, per the spec's M4 note).
3. Copy the file to a second temp dir. Run the OLD pipeline on the first: `sh -c "cd <dir1> && bash <REPO>/pin-ci-images.sh"` where `<REPO>` is the repo root (compute from `CARGO_MANIFEST_DIR` at compile time via `env!`); capture stdout; on non-zero exit → `panic!` with the output.
4. Run the NEW tool on the second dir's file via `pin_files` with `client_from_env()`.
5. Extract all `[\w./-]+@sha256:[0-9a-f]{64}` occurrences from both files (simple regex or manual scan) and assert the sets are equal and non-empty.
6. Cleanup temp dirs.

- [ ] **Step 3: Run**

Run: `cargo test --test live --test oracle` (gates absent → both print SKIP and pass) and `PIN_CI_LIVE=1 cargo test --test live` (only if Docker Hub is reachable from this machine; if the network is blocked, note it in the report and move on — the mock + oracle tests carry correctness).

- [ ] **Step 4: Write `README.md`**

Sections (exact headers): `# pin-ci`, `What it does` (M1: Docker Hub image pinning; components and --update are later milestones), `Usage` (the `pin-ci [FILES...]` command, `--dry-run`, `--json`, `--concurrency`; example output), `Environment` (`DOCKERHUB_USER`, `DOCKERHUB_TOKEN`, `~/.docker/config.json` fallback), `Exit codes` (0 / 1, per Global Constraints), `Tests` (`cargo test`, `PIN_CI_LIVE=1`, `PIN_CI_ORACLE=1` and what the oracle needs), `Old pipeline` (pin-ci-images.sh et al. are kept unmodified as the correctness oracle; retirement is a later decision).

- [ ] **Step 5: Full verification**

Run: `cargo fmt --all && cargo test && cargo build --release`
Expected: all green; `target/release/pin-ci` exists.

- [ ] **Step 6: Commit**

```bash
git add tests/live.rs tests/oracle.rs README.md
git commit -m "test: live + differential oracle harness; docs: README"
```

---

## Self-Review (controller, done at plan time)

- Spec coverage M1: G1 (Tasks 2,3 discovery; 5,6 resolution), G4 (pin-only everywhere), G5 (Task 4 byte-exact + idempotency e2e), G6 (Task 6 report/exit codes), G7 (Task 7 oracle). Non-hub skip (Task 6 e2e). Re-pin-stale (splice_value digest replacement + Task 4 test; end-to-end via second_run variant is implicit — the `pinned` path covers it).
- Steps without a test: none — every task begins with a failing check.
- Types consistent across tasks: `Target{line,start,end,value}`, `ImageRef{host,repo,tag,digest}`, `RegistryClient::new(Creds, HashMap)`, `pin_files(&[PathBuf], &PinOptions, &RegistryClient)`, `Report/RefResult/Summary` — defined once, reused verbatim.
- Known accepted limitations (M1, all in spec or rulings): no escaped-quote handling in YAML strings; flow lists containing nested braces yield no targets; block-scalar image values are not targets; tabs in indentation are ignored.
