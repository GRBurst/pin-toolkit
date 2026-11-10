# M5c-y plan — gitlab-ci discovery on `core::yaml` (pin-toolkit)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

- Date: 2026-11-10
- Source of truth: `docs/superpowers/specs/2026-11-09-pin-toolkit-design.md` — §6.9 (YAML discovery), §9 (M5c-y row, R7, R8), §11 (discovery parity, planted violations), §12 D32/D36, §14.1 (review carry-overs L1/L7)
- Inputs: spike `.superpowers/sdd/2026-11-10-yaml-spike/result.md` + `src/main.rs` (`token_bytes` at L140, frame walker); M5a–c plan "Handoff note for M5c-y"; code review `.superpowers/sdd/2026-11-10-m5abc/code-review.md` (L1, L3, L7).
- Base: `main` after the M5a–c merge (branch `m5-workspace-core` @ 0578856 or later). Branch: `m5cy-yaml-discovery`.
- Out of scope: G1 (`include: - component:`), M5d (exit 2, toolkit binary), M8a (helm consumer of masking). Deferred to M5d: review L3 (one `LineIndex` per file in `trailing_comment`) plus L2, L5, L6, L8–L10 as listed in spec §14.1.
- Tech stack: Rust 1.95, edition 2024, saphyr-parser 0.3.3 (+arraydeque 0.5.1), insta 1.49, wiremock.

## Goal

Replace the hand-rolled, line-based YAML scanning in `crates/pin-gitlab-ci/src/discover.rs` with path matching over
`pin_core::yaml::scan` (saphyr-parser events + byte-span fix-ups), behind pin-core feature `yaml`. Prove parity with
the old `discover()` on every fixture except the documented R7/R8/R9 cases, then delete the old code.

## Motivation (spec §6.9, §9, D32)

The line scanner mis-reads anchors and aliases (R7), misses block children under CRLF and under anchored keys (R8),
and cannot grow to helm values or helmfile. A span-preserving event parser gives real structure (flow, anchors,
multi-document) while edits stay byte splices on the original text. The spike reproduced 37/37 old targets at
identical bytes once four fix-ups are applied. A file that does not parse becomes a per-file error with no fallback (D32).

## Approach (shape of the diff)

| Task | Diff shape |
|---|---|
| Y1 | tests only: 6 fixtures, 18 snapshots, frozen `tests/parity/legacy-discover.txt` + generator test |
| Y2 | tests only (review L7): span-based edit check over all fixtures |
| Y3 | pin-core `run::pin_file` splice order (review L1) + 4 regression tests (review L7) |
| Y4–Y6 | new `crates/pin-core/src/yaml.rs` [feature `yaml`], `LineIndex::line_of`, 2 new crates in `Cargo.lock` |
| Y7 | `PinTarget::Unsupported` → `skipped (block scalar)` |
| Y8–Y9 | `discover_yaml` beside the old `discover` (old still wired): parity + token properties |
| Y10 | switch: rename (`discover` = yaml, `legacy_discover` = old), `scan` returns parse errors; 12 snapshot diffs (R7/R8/R9); README |
| Y11 | delete `legacy_discover` and every hand-rolled primitive (refactor, ≈ −850 lines) |

## Global constraints

1. **No behaviour change except Y10.** Proof per task: `INSTA_UPDATE=no` gate green and `git status` shows no `*.snap` change. Y10 changes exactly the 12 snapshot files it names.
2. **Existing tests are preserved.** Allowed edits, each stated in its task: Y7 one match arm in `combined_targets_sorted_by_line_then_start` (new enum variant); Y10 `discover(x)` → `discover(x).unwrap()` in the 20 `discover.rs` unit tests (return type becomes `Result`), `.expect(..)` in `tests/common/mod.rs`, `discover` → `legacy_discover` in the generator test, `discover_yaml` → `discover` in the parity tests; Y3 adds a field to the test-local resolver `Fake` in `run.rs`; Y11 deletes the legacy generator test (its subject is deleted).
3. **Features (D36):** pin-core `yaml = ["dep:saphyr-parser"]`; `saphyr-parser` is optional; `pub mod yaml` behind `#[cfg(feature = "yaml")]`. pin-gitlab-ci enables `["oci", "gitlab", "yaml"]`. Net new crates: exactly `saphyr-parser`, `arraydeque` (`thiserror` 2.0.21 is already locked).
4. **No panics on any input** in `core::yaml`: marker lookups use `.get()`, the quoted re-scan is bounded by the parser span; an impossible span is `ScanError { msg: "internal: bad span at byte {n}" }`.
5. **Texts users read (pinned literally):** parse error `msg = "invalid YAML: {saphyr info}"`, `line = Some(marker.line())` (1-based) → `FileError::Parse` Display `"{path}:{line}: invalid YAML: …"`; skip reason `block scalar`. All M5a–c texts unchanged.
6. **Comment detection stays hand-rolled** (`edit::trailing_comment`, `requote`, image splices): the parser never writes.
7. Env/commits: `export CARGO_HOME="$PWD/.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH"`; unsigned commits `git -c commit.gpgsign=false -c core.excludesFile=/dev/null commit`; every `git add`/`status` with `-c core.excludesFile=/dev/null`; scratch only under `.tmp/`. saphyr-parser 0.3.3 and arraydeque 0.5.1 are already in `.cargo-home/registry` (spike).

### Check (run after every task; "the gate")

```bash
export CARGO_HOME="$PWD/.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH"
INSTA_UPDATE=no cargo test --workspace --all-features > .tmp/m5cy-test.log 2>&1; echo "cargo exit $?"
grep -E '^test result' .tmp/m5cy-test.log | awk '{p+=$4; f+=$6} END {print "passed", p, "failed", f}'
cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings
for f in "" "--features oci" "--features gitlab" "--features yaml"; do   # "yaml" leg from Y4 on
  cargo clippy -p pin-core --no-default-features $f --all-targets -- -D warnings || echo "FAIL [$f]"; done
git -c core.excludesFile=/dev/null status --porcelain     # only this task's files; never *.snap.new
```

Baseline: M5a–c result, **passed 192 failed 0** (code review run). Expected passed after each task:
Y1 193 · Y2 195 · Y3 199 · Y4 206 · Y5 212 · Y6 215 · Y7 216 · Y8 222 · Y9 223 · Y10 225 · Y11 224.
A lower number is a failure (silently skipped tests) unless the task explains it (Y11: −1).

## Decisions

| # | Decision | Rationale | Rejected |
|---|---|---|---|
| Y-D1 | `yaml::scan` returns `pin_core::plugin::ScanError` | identical fields (`line`, `msg`); `GitlabCi::scan` forwards with `?` | a separate `YamlError` + `From` (spec §6.9 name; no behaviour) — Open for main 3 |
| Y-D2 | char→byte = one `Vec<usize>` per file (`char_indices` + `len`); lines via new `LineIndex::line_of`; `Marker::col()` never used | spike §1: `index()` counts chars, BOM is char 0, `col` skips the BOM on line 1 | `Marker::index()` as byte (wrong after any non-ASCII char) |
| Y-D3 | `ScalarRef` adds `line`, `parent`, `in_flow` to the spec shape | `parent` (collection id) groups mapping siblings although `ci-include.yml` repeats `include:` 3×, M8a needs it for `image:{repository,tag}`; `in_flow` keeps `IncludeRef.flow`; `line` saves every consumer a `LineIndex` | grouping by path (merges duplicate keys; planted in Y9) |
| Y-D4 | A node with a non-core tag (`!reference`, `!x`) is one `Node::Tagged`; its subtree emits nothing. Core tags (`!!str`, `!!seq`) are transparent | `!reference [.base, image]` items must not become image targets; spec: `!reference` is not a target | emitting children (two bogus `skipped` rows per `!reference`) |
| Y-D5 | Keys are not nodes; a non-scalar or alias key is `PathSeg::Complex` and its subtree is skipped | consumers match value paths only | — |
| Y-D6 | Property skip (`&a `, `!t `) only when the event carries an anchor or tag, never for block scalars; it also skips line breaks and `#` comments between property and token | block content may start with `!`/`&`; `key: &a\n  v` is valid YAML | spike's unconditional `&`/`!` skip |
| Y-D7 | Template mask: every `{{…}}` (non-greedy, may span lines; unclosed `{{` not masked) → spaces, **`\n`/`\r` kept**; `templated` ⇔ non-empty span ∩ mask ≠ ∅ | byte offsets and line numbers (error lines!) stay exact | spaces for newlines too (merges lines); flagging empty spans (position before/after the mask is parser-defined) — M8a handoff |
| Y-D8 | gitlab-ci calls `scan(text, ScanOpts { mask_templates: false })` | GitLab CI is not Go-templated | — |
| Y-D9 | gitlab-ci matcher (below): uniform `image`/`services` rule, root `component`, root `include` mappings grouped by `parent`; empty span = no value; block scalar → `PinTarget::Unsupported` (`skipped (block scalar)`) | a superset of today's shapes, GitLab semantics; spec §6.9 block-scalar rule; never splice a block scalar (span = content incl. newline) | an exact copy of the line scanner's accidents |
| Y-D10 | Parity against a **frozen** record file written by the old code (Y1) + an exact exception table in the test | the old code is deleted (spec §9), so its output must exist as data; exact lists catch stale exceptions | insta snapshot of legacy output (re-accepted, record lost); keeping the old discoverer |
| Y-D11 | Switch (Y10, behaviour) and deletion (Y11, refactor) are separate commits; Y10 renames so the 20 old unit tests run against the new code | strongest evidence lands in the behaviour commit; the large deletion reviews as pure removal | one commit |
| Y-D12 | Review L1: `pin_file` collects splices in `(line, min edit start)` order (stable); rows keep finding order | combined comment in byte order whatever the scan order; no row reorder → no snapshot churn | sorting rows too (mixed-status lines reorder) |
| Y-D13 | Review L7: span-based check ties every replacement to exactly one discovered span (injective) and the comment to byte order; + 3 pin-core regression tests + gitlab-only lazy test | `line_diff` reverts the first occurrence only; one `#` ≠ every original present | — |
| Y-D14 | Review L3 deferred to M5d | one index per file needs a per-file context in `Plugin::resolve` or in the target; M5d reshapes `run` | `&LineIndex` param rebuilt per call (API churn, no gain) |
| Y-D15 | `saphyr-parser = "0.3.3"` in `[workspace.dependencies]` | spike version; `Cargo.lock` pins it; the re-scan stays correct after an upstream fix | git dependency |

### gitlab-ci matcher (Y8/Y9), over `Node::Scalar` with non-empty span, paths relative to each document root

- `image` kind ⇔ ∃ q ∈ {p} ∪ {p[1..] | p[0] is `Key`}: q[0] ∈ {`Key("image")`, `Key("services")`} ∧ q[1..] ∈ {[], [`Key("name")`], [`Index`], [`Index`, `Key("name")`]}.
- `component` kind ⇔ p = [`Key("component")`]; `parts = parse_component_ref(value)` (kept verbatim).
- `include` kind: for each mapping with path m ∈ {[`Key("include")`], [`Key("include")`, `Index`]}, keyed by `parent`: the first scalar child `project` and the first scalar child `ref` (any order). Target iff both exist: `IncludeRef { line, start, end, value } = ref`, `project = project.value`, `flow = ref.in_flow`.
- Any match whose scalar has `style == Block` → `PinTarget::Unsupported { kind, …, reason: "block scalar" }` instead.
- Output sorted by `(line, start)`.

Known supersets vs the old scanner (no fixture contains them; unit-tested in Y8/Y9): flow job mappings `job: {image: x}`, quoted or non-ASCII job keys, `image: [a]`, `services: {name: x}`, flow include items with nested `inputs: {…}`.

## Gate (spec §11; no principles document in the repo)

| Property | Status | Where |
|---|---|---|
| Byte-exact | PASS | P-bytes (M5a) + P-span Y2 `edits_touch_only_reference_spans_over_all_fixtures`, now over 17 fixtures |
| Idempotent | PASS | `idempotent_over_all_fixtures` (unchanged; covers R7 definitions vs aliases after Y10) |
| Snapshot review | PASS | Y1 baseline (18 new); Y10 the only diffs (12 named files) |
| Discovery parity | PASS | Y8/Y9 `yaml_discovery_matches_legacy_except_documented` + `spans_are_tokens_over_all_fixtures` |
| Lazy resolvers | PASS | unchanged + Y3 `gitlab_target_builds_only_gitlab`; Y10 parse error builds no client |
| Plugin isolation | PASS | `scan` stays pure (no network in any Y test); `--features yaml` alone in the matrix |
| Fail loud | PASS (part) | Y10 invalid YAML → `ScanError`/`FileError::Parse`, nothing written; exit 2 is M5d |
| Planted violations | PASS once every box is ticked | spec's two (`Marker::index()` as byte, drop re-scan) in Y8 |

## Files to read first (implementer step 0)

Spec §6.9, §9 (M5c-y, R7, R8), §11. Spike `result.md` and `src/main.rs:27-160`. `crates/pin-gitlab-ci/src/discover.rs:1-160` and its
`mod tests` (`:964-1367`), `src/lib.rs`, `src/policy.rs:30-65`, `crates/pin-core/src/{text.rs, plugin.rs:55-130, run.rs:255-340, edit.rs:30-75}`,
`crates/pin-core/tests/run_pin_file.rs:1-90`, `crates/pin-gitlab-ci/tests/common/mod.rs` (whole), `tests/fixture_properties.rs`, `tests/plugin.rs`.
saphyr API (local source `.cargo-home/registry/src/*/saphyr-parser-0.3.3/src/`): `parser.rs:52-125` (`Event`, `Tag`), `:862-962` (node span = properties start … token end),
`scanner.rs:64-150` (`Marker`, `Span`, `ScanError::{marker, info}`).

## Type-checking strategy

Gate clippy (`-D warnings`, workspace, all features) + the pin-core matrix incl. `--features yaml`. Expected rejections that prove structure:
- `--no-default-features`: any `use saphyr_parser` outside the gated module; `--features yaml` alone: any accidental use of `oci`/`gitlab` items in `yaml.rs`.
- Y7: adding `PinTarget::Unsupported` makes every non-exhaustive match fail (`PinTarget::{line,start,end,value}`, `target_kind`, `GitlabCi::resolve`, the test above) — the compiler lists every site.
- Y8: `pin_core::yaml` unresolved in pin-gitlab-ci until its dependency enables `yaml` (E0433).
- Y10: `discover` returning `Result` rejects every un-adapted caller (tests, `common/mod.rs`, `lib.rs`).
- `clippy::single_range_in_vec_init`/`Range` not `Copy` → `.clone()`; `dead_code` on `Node` fields read only by tests → they are `pub`.

## Testing strategy (per layer)

| Layer | Covers | Needs |
|---|---|---|
| Unit (`yaml.rs`, `text.rs`) | walker paths, char→byte, CRLF/BOM/multi-doc, fix-ups, aliases/tags, block, masking, parse errors, no-panic prefixes | inline texts only |
| Unit (`discover.rs`) | gitlab-ci matcher rules incl. supersets; 20 legacy tests re-pointed in Y10 | inline texts, fixtures via `include_str!` |
| Core integration (`run_pin_file.rs`) | L1 order, overlap regressions (L7) | fake plugin, `.tmp/` files |
| Plugin integration (`plugin.rs`) | `Unsupported` → skipped; parse error → `ScanError` / `FileError::Parse`, nothing written | `.tmp/` files, no network |
| Parity (`discovery_parity.rs`) | frozen legacy record vs `yaml` discovery, exceptions; token property | fixtures + record file |
| Snapshot + property (M5a) | output, P-bytes, P-idem, P-one-comment, P-crlf, P-comment-present, P-span (Y2) | fake servers |
| E2E (`e2e*.rs`) | existing behaviour through the driver — must stay green unchanged in Y10 | wiremock |
| Live / oracle | real registries / old pipeline | env-gated, close-out only |

## Properties (derived from the SUT)

𝔽 = `tests/fixtures/*.yml` (17 after Y1); 𝔽ᵣ = fixtures named in the legacy record; disc = the discoverer under test.
- **P-parity (Y8, Y9):** ∀ f ∈ 𝔽ᵣ: hash(f) = recorded hash ∧ legacy(f) ∖ R(f) = render(disc(f)) ∖ A(f) as ordered lists ∧ every e ∈ R(f) matches exactly one legacy line ∧ every e ∈ A(f) exactly one new line (R/A = exception table, matched on `(line, kind, token)`).
- **P-token (Y8):** ∀ f ∈ 𝔽, ∀ t ∈ disc(f), t not `Unsupported`: tok = f[t.start..t.end] ∈ {v, `'`v`'`, `"`v`"`} with v = t.value(), and `line_of(t.start) = t.line`, `t.end ≤ line_end(t.line)`.
- **P-span (Y2):** ∀ f, mode, F = findings of `plan`: each edited finding has exactly one replacement r, [r.start, r.end) = the span of a discovered target on line `f.line`, `O[r] = f.old`, the map finding ↦ target is injective; each insertion sits at `line_end(f.line)` and reads `"  # " ⧺ orig`, no `\n`; `apply(O, coalesce_comments(edits ordered by (line, replacement start))) = E₁`.
- **P-nopanic (Y5):** ∀ sample s, ∀ char-boundary prefix p of s, ∀ mask ∈ {off, on}: `scan(p)` returns; `Ok` ⇒ every span has `start ≤ end ≤ |p|` on char boundaries of p.
- **P-mask (Y6):** |mask(t)| = |t|; ∀ i: t[i] ∈ {`\n`, `\r`} ⇒ mask[i] = t[i]; ∀ i ∉ ⋃M: mask[i] = t[i]; ∀ i ∈ ⋃M ∖ breaks: mask[i] = `' '`.
- Literals pinned only where they are the criterion: `invalid YAML: mapping values are not allowed in this context`, `block scalar`, the exception table.

## Scenario coverage (spec → check)

| Spec scenario | Check |
|---|---|
| §6.9 `index()` counts chars, BOM char 0, never `col()` | Y4 `scan_char_to_byte_non_ascii_and_bom`; parity on `ci-unicode`, `ci-bom`; Y8 planted (a) |
| §6.9 quoted span overshoot (comment, flow `,`/`]`) | Y5 `quoted_spans_end_at_closing_quote`; parity `ci-quoted-comment`; Y8 planted (b) |
| §6.9 spans include `&anchor`/`!tag` | Y5 `properties_are_skipped`; parity `ci-anchors`, `ci-tags` |
| §6.9 aliases, `<<:`, `!reference` not targets | Y5 `aliases_are_alias_nodes`, `local_tags_are_opaque_core_tags_transparent`; Y8 `yaml_aliases_and_references_are_not_targets`; parity |
| §6.9 sticky `ScanError` → per-file error, no partial targets, no fallback (D32) | Y4 `scan_error_has_line_and_msg`; Y10 `scan_invalid_yaml_is_scan_error`, `invalid_yaml_file_is_parse_error_and_untouched` |
| §6.9 `{{…}}` masking, `templated` | Y6 (3 tests) |
| §6.9 block scalars never spliced → `skipped (block scalar)` | Y5 `block_scalars_keep_style_and_content_span`; Y7; Y8 `yaml_block_scalar_is_unsupported` |
| §6.9 CRLF, multi-document spans | Y4 `scan_crlf_spans_exclude_cr`, `scan_documents_reset_paths`; fixtures `ci-r2-crlf`, `ci-multidoc` |
| §9 gitlab-ci discovery = path matching over nodes | Y8, Y9 unit tests + parity |
| §9 delete the `discover.rs` YAML primitives | Y11 |
| §9 R7, R8 fixed as reviewed snapshot diffs | Y10 (12 named snapshots) |
| §11 parity ∀ fixture, `text[span]` = token incl. quotes; fixture list (non-ASCII, BOM, CRLF, quoted + comment, anchor/tag, alias, `<<:`, multi-doc) | Y1 fixtures + record; Y8/Y9 P-parity, P-token. Masked `{{ }}`: Y6 unit (no gitlab-ci consumer) |
| §6.1 +2 crates, feature-gated | Y4 `Cargo.lock` check + matrix |
| §14.1 L1, L7 | Y3, Y2 |

## Coverage gap (what a human runs)

1. Live and oracle suites (network, renovate/crane): at close-out `PIN_CI_LIVE=1 cargo test -p pin-gitlab-ci --test live`, and in devenv `PIN_CI_ORACLE=1 cargo test -p pin-gitlab-ci --test oracle`.
2. The `pin-ci` binary on an invalid file: `printf 'job:\n\timage: alpine\n' > .tmp/bad.yml; cargo run -p pin-gitlab-ci --bin pin-ci -- --dry-run .tmp/bad.yml; echo $?` → message `….tmp/bad.yml:2: invalid YAML: …`, exit **1** (driver maps every `FileError` to exit 1; exit 2 and continuing with other files arrive in M5d).
3. Template masking has no consumer until M8a; an unquoted `key: {{ x }}` yields an empty, non-templated scalar (Y6 characterises it) — M8a must report it.
4. Multi-line flow scalars at target paths (escaped line continuation): no test; `trailing_comment` already returns `None` when the token ends past its line.
5. Large files: no performance test (char table O(n), one pass); spot-check by hand on a 10k-line generated file if desired.

## Review Focus

1. **Invalid YAML** (tab indentation, `a: b: c`): was partially pinned, is now a file error with its line and nothing written — Y10 `scan_invalid_yaml_is_scan_error` (tab case + mapping case), `invalid_yaml_file_is_parse_error_and_untouched`.
2. **Anchored job templates and aliases** (`.base: &base`, `<<: *base`, `image: *img`): the definition is pinned once, aliases are untouched and have no row, a second run is all `ok` — `ci-anchors` snapshots (Y10) + `idempotent_over_all_fixtures`.
3. **Quoted values followed by comments or flow padding** (`"x" # c`, `[ "a" , 'b' ]`): the splice ends at the closing quote — Y5 unit, `ci-quoted-comment` parity and P-span.
4. **Non-ASCII text or a BOM before a target**: offsets are bytes, not chars — Y4 unit, `ci-unicode`/`ci-bom` parity, P-span; Y8 planted (a).
5. **Unusual value shapes at target paths** (`image: >-`, `!reference [...]`, `!!str x`, flow job mapping): block → `skipped (block scalar)`, local tag → no target, core tag → pinned, flow job → target — Y8 unit tests.

---

## Snippets (shared contracts)

### pin-core (Y4–Y6)

```rust
// Cargo.toml (root)  [workspace.dependencies]  saphyr-parser = "0.3.3"
// crates/pin-core/Cargo.toml  [features] yaml = ["dep:saphyr-parser"]   [dependencies] saphyr-parser = { workspace = true, optional = true }
// crates/pin-core/src/lib.rs  #[cfg(feature = "yaml")] pub mod yaml;

// text.rs (ungated)
impl LineIndex { /// 0-based line containing `byte`; past the last line start → last line; 0 for "".
                 pub fn line_of(&self, byte: usize) -> usize; }   // starts.partition_point(|&s| s <= byte).saturating_sub(1)

// yaml.rs [feature yaml]
use std::ops::Range;
use crate::plugin::ScanError;
#[derive(Debug, Clone, PartialEq, Eq)] pub enum PathSeg { Key(String), Index(usize), Complex }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Style { Plain, Single, Double, Block }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarRef {
    pub path: Vec<PathSeg>,     // value path from the document root; keys are not nodes
    pub doc: usize,             // 0-based document index
    pub line: usize,            // 0-based, LineIndex::line_of(span.start)
    pub span: Range<usize>,     // BYTE range of the token incl. quotes; properties excluded; block: content only
    pub style: Style,
    pub value: String,          // decoded (from the masked text when masking)
    pub anchor: Option<usize>,  // anchor id when this scalar defines one
    pub parent: usize,          // id of the enclosing collection (ids from 1, in event order); 0 = document is this scalar
    pub in_flow: bool,          // the enclosing collection is a flow collection (or nested in one)
    pub templated: bool,        // non-empty span overlaps a masked {{…}}
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Scalar(ScalarRef),
    Alias { path: Vec<PathSeg>, id: usize, span: Range<usize> },   // `*name`; never a target
    Tagged { path: Vec<PathSeg>, tag: String },                    // non-core tag (`tag.to_string()`, e.g. "!reference"); subtree not emitted
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)] pub struct ScanOpts { pub mask_templates: bool }
/// Every value node in event order. Err on the first parser error (sticky upstream): no partial result.
pub fn scan(text: &str, opts: ScanOpts) -> Result<Vec<Node>, ScanError>;
```

Walker (Y4; tag/skip parts Y5). `src` = masked text or `text`; `c2b` = char table of `src`; `lines = LineIndex::new(text)`.
Frames: `Map { id, flow, key: Option<PathSeg>, want_key }`, `Seq { id, flow, idx }`. `advance()`: top `Map` → `want_key = true`; `Seq` → `idx += 1`.
`path()` = per frame: `Map` → its `key` (always `Some` at value position), `Seq` → `Index(idx)`. State `skip: Option<(depth, was_key)>`.

| Event | Action |
|---|---|
| `Err(e)` | `return Err(ScanError { line: Some(e.marker().line()), msg: format!("invalid YAML: {}", e.info()) })` |
| any, while `skip = Some((d, k))` | `*Start` → d+1; `*End` → d−1, at 0: `skip = None`, then `advance()` unless k; others ignored |
| `DocumentStart(_)` | `doc = documents seen − 1`; `stack.clear()` |
| `Scalar(v, style, anchor, tag)` at key position | top `Map.key = Some(Key(v))`, `want_key = false` |
| `Scalar` at value position | non-core tag → push `Tagged`; else span = `token_span(..)` (Y5; Y4: raw span) → push `Scalar(ScalarRef{..})`; then `advance()` |
| `Alias(id)` | key position → `key = Some(Complex)`, `want_key = false`; else push `Alias { path, id, span }`, `advance()` |
| `MappingStart`/`SequenceStart(anchor, tag)` | key position → `key = Some(Complex)`, `want_key = false`, `skip = Some((1, true))`; non-core tag → push `Tagged`, `skip = Some((1, false))`; else push frame `{ id: next_id++, flow: parent.flow ∨ (after property skip, s' < e ∧ src[s'] ∈ {'{','['}) }` |
| `MappingEnd`/`SequenceEnd` | pop; `advance()` |
| others | ignore |

Byte span of an event: `(c2b.get(span.start.index()), c2b.get(span.end.index()))`, both `Some` and `text.is_char_boundary` at both ends, else internal `ScanError`.

`token_span(src: &[u8], s, e, style, has_props) -> Option<Range<usize>>` (Y5):
1. `has_props` (anchor id > 0 or tag present) and style ≠ Block: repeat while `s < e` and `src[s] ∈ {b'&', b'!'}`: skip non-whitespace bytes; then skip spaces, tabs, `\r`, `\n`, and from a `#` to the next `\n`.
2. `Single`: from `s+1` find the first `'` not followed by `'` (skip `''` pairs), index < e → `s..i+1`; `Double`: from `s+1`, `\\` skips 2 bytes, first `"` < e → `s..i+1`; not found → `None`.
3. otherwise `s..e`.

`mask_templates(text) -> (String, Vec<Range<usize>>)` (Y6, private): scan for `{{`; find the next `}}`; none → stop; mask = `open..close+2`; every byte in it except `\n`/`\r` becomes `b' '` (a whole UTF-8 sequence lies inside, so the result stays UTF-8); continue after the mask.

### pin-gitlab-ci (Y7–Y11)

```rust
// discover.rs (final, after Y11): PinTarget, ComponentParts, impl PinTarget, parse_component_ref (verbatim), discover, matcher helpers
pub enum PinTarget { Image{..}, Component{..}, IncludeRef{..},                       // unchanged
    /// A reference at a target path that cannot be spliced (block scalar): reported `skipped (<reason>)`.
    Unsupported { kind: &'static str, line: usize, start: usize, end: usize, value: String, reason: &'static str } }   // Y7
pub fn discover(text: &str) -> Result<Vec<PinTarget>, ScanError>;   // Y8 as `discover_yaml`, renamed Y10; sorted (line, start)
fn is_image_path(p: &[PathSeg]) -> bool; fn is_component_path(p: &[PathSeg]) -> bool; fn is_include_mapping(p: &[PathSeg]) -> bool;
#[doc(hidden)] pub fn legacy_discover(text: &str) -> Vec<PinTarget>;   // Y10 only (old body); deleted Y11
// policy.rs (Y7)
pub(crate) fn unsupported(text: &str, t: &PinTarget) -> Finding;      // base(text, t, kind) + Skipped + Some(reason)
// lib.rs: target_kind(Unsupported{kind,..}) = kind; resolve → policy::unsupported;
//         scan (Y10) = Ok(discover(text)?.into_iter().filter(|t| on.contains(target_kind(t))).collect())
```

### Test helpers

```rust
// tests/common/mod.rs (Y2)
pub async fn plan_once(f: &Fakes, name: &str, text: &str, mode: Mode) -> Vec<Finding>;
//   ctx = Ctx::new(UpdatePolicy { enabled: matches!(mode, Mode::Update), ..Default::default() }, now_unix(),
//                  ResolverEnv { gitlab_host: "gitlab.com".into(), ..Default::default() })
//         .with_oci(Ok(registry_client(f))).with_gitlab("gitlab.com", Ok(gitlab_client(f)));
//   GitlabCi::default().plan(&ctx, Path::new(name), text, &KindSet::defaults(&KINDS), 4).await.unwrap()
pub fn check_spans(o: &str, e1: &str, findings: &[Finding]) -> Result<(), String>;   // P-span; targets = discover(o)

// tests/discovery_parity.rs (Y1; Y8)
const RECORD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/parity/legacy-discover.txt");
fn render(name: &str, text: &str, ts: &[PinTarget]) -> Vec<String>;
//   "{name}:{line+1}\t{kind}\t{start}..{end}\t{token:?}\t{detail}", token = &text[start..end]; detail:
//   Image `value={v:?}` · Component `value={v:?} parts={host}|{project}|{name}|{version}` or `parts=-`
//   IncludeRef `value={v:?} project={p:?} flow={flow}` · Unsupported `value={v:?} reason={r}`
// record file: line 1 "# legacy discover() output, recorded in M5c-y Y1 before its deletion; never regenerate"
//              per fixture (sorted): "## {name} bytes={len} hash={fake_hex(&text, 16)}" then its render lines
type Exc = (&'static str, usize /*1-based*/, &'static str /*kind*/, &'static str /*token*/);
```

Exception table (Y8; exact — any other difference fails):

```rust
/// legacy lines that disappear
const REMOVED: &[Exc] = &[
    ("ci-anchors.yml", 1, "image", "&img alpine:3.20"),   // R7: anchor inside the value
    ("ci-anchors.yml", 6, "image", "*img"),               // R7: alias is not a target
    ("ci-anchors.yml", 9, "image", "&redis redis:7"),     // R7
    ("ci-anchors.yml", 10, "image", "*redis"),            // R7
    ("ci-tags.yml", 5, "image", "!reference [.base, image]"),      // R7: !reference is not a target
    ("ci-tags.yml", 6, "image", "!reference [.base, services]"),   // R7
    ("ci-tags.yml", 8, "image", "!!str node:20"),                  // R7: tag inside the value
];
/// new lines that appear
const ADDED: &[Exc] = &[
    ("ci-anchors.yml", 1, "image", "alpine:3.20"),        // R7
    ("ci-anchors.yml", 3, "image", "ubuntu:24.04"),       // R8: child of `.base: &base`
    ("ci-anchors.yml", 9, "image", "redis:7"),            // R7
    ("ci-r2-crlf.yml", 6, "include", "dev"),              // R8: CRLF block include
    ("ci-r2-crlf.yml", 9, "image", "redis:7"),            // R8: CRLF block services
    ("ci-r2-crlf.yml", 10, "image", "\"node:20\""),       // R8: CRLF job-nested image
    ("ci-tags.yml", 8, "image", "node:20"),               // R7
    ("ci-bom.yml", 2, "image", "alpine:3.20"),            // R9: BOM hid the first key's children
];
```

---

## Setup

`git -c core.excludesFile=/dev/null switch -c m5cy-yaml-discovery main` (after the M5a–c merge). Gate → passed 192.

## Task Y1 — new fixtures + frozen legacy discovery record (tests only)

**Files:** Create `crates/pin-gitlab-ci/tests/fixtures/ci-{unicode,quoted-comment,tags,bom,multidoc,pipeline}.yml`, 18 `tests/snapshots/fixture_snapshots__ci-*__pin__*.snap`, `tests/parity/legacy-discover.txt`, `tests/discovery_parity.rs`.
**Produces:** fixtures (𝔽 = 17), `render`, the record file (frozen from here on).

Exact fixture bytes (LF, final newline). `ci-pipeline.yml` = `cp ci/.gitlab-ci.yml` (realistic file, the spike's 37/37 input). `ci-bom.yml` via
`printf '\xef\xbb\xbfdefault:\n  image: alpine:3.20\njob:\n  image: "node:20"  # \xc3\xbc\n'`.

```text
ci-unicode.yml:          # Grüße 🚀 — pipeline / variables: / "  NOTE: \"naïve café ☕\"" / image: alpine:3.20  # ✓ / job: /
                         "  image: 'node:20'  # ö" / "  services: [\"ü-sidecar\", docker:24]" / include: { project: g/ü, ref: main }
ci-quoted-comment.yml:   image: "alpine:3.20" # double-quoted, comment / job: / "  image: 'node:20'   # single-quoted, comment" /
                         "  services: [ \"docker:24\" , 'redis:7' ]  # flow: space before , and ]" / twice: /
                         "  services: [docker:24, docker:24]" / include: { project: 'g/p', ref: "main" }  # flow include, quoted
ci-tags.yml:             .base: / "  image: alpine:3.20" / "  services: [docker:24]" / job: / "  image: !reference [.base, image]" /
                         "  services: !reference [.base, services]" / tagged: / "  image: !!str node:20"
ci-multidoc.yml:         spec: / "  inputs:" / "    tag:" / "      default: \"3.20\"" / --- / image: $[[ inputs.tag ]] / job: /
                         "  image: alpine:3.20" / ... (document-end marker, 3 dots) / --- / other: / "  services:" / "    - redis:7"
```

- [ ] RED: add the fixtures; write `discovery_parity.rs` with `render` and `legacy_record_matches_discover` (`#[test]`): build the expected record text from `discover()` over `fixtures()` (format above); `PIN_PARITY_RECORD=1` → write `RECORD` and pass; otherwise `assert_eq!(read_to_string(RECORD), expected)`. Run gate → FAIL: 18 new snapshots and the missing record file.
- [ ] GREEN: `INSTA_UPDATE=always cargo test -p pin-gitlab-ci --test fixture_snapshots`; delete `*.snap.new`; `PIN_PARITY_RECORD=1 cargo test -p pin-gitlab-ci --test discovery_parity`.
- [ ] Review the recorded output (old behaviour, partly wrong on purpose): record lines per new fixture — unicode 5, quoted-comment 7, tags 5, bom 1, multidoc 3, pipeline 10. Snapshots: `ci-unicode` `ü-sidecar` `skipped (unparseable reference)`, the rest pinned, include gets `  # main`; `ci-quoted-comment` all pinned, quotes kept, include without comment (line has `#`), both `docker:24` on line 6 pinned; `ci-tags` lines 5, 6, 8 `skipped (unparseable reference)` (R7); `ci-bom` only line 4 (R9: line 2 missing), BOM bytes kept; `ci-multidoc` line 6 skipped, 8 and 13 pinned; `ci-pipeline` `${APP_IMAGE}…` skipped, 9 pinned. All existing properties green on the new fixtures.
- [ ] Planted: edit one character of a recorded token in `legacy-discover.txt` → `legacy_record_matches_discover` FAILS ☐; `git checkout` the file.
- [ ] Gate → 193. Commit: `test: M5c-y fixtures (unicode, BOM, quoted+comment, tags, multi-doc, pipeline) + frozen legacy discover() record`.

## Task Y2 — span-based edit check (review L7; tests only)

**Files:** `crates/pin-gitlab-ci/tests/common/mod.rs` (`plan_once`, `check_spans`), `tests/fixture_properties.rs`.
**Consumes:** Y1 fixtures; `GitlabCi`, `DynPlugin::plan`, `Ctx` presets, `edit::{apply, coalesce_comments}`, `text::LineIndex`.

- [ ] RED: `edits_touch_only_reference_spans_over_all_fixtures` (`#[tokio::test]`; ∀ fx, m: `e1` from `pin_once` on a scratch copy, `check_spans(&fx.text, &e1, &plan_once(..).await)`; collect `"{name}/{mode}: {msg}"`, assert empty) and `span_checker_rejects_wrong_occurrence` (no I/O; local `fn finding(line, old, edits) -> Finding`, status `Pinned`): `o = "services: [a:1, a:1]\n"` (targets 11..14, 16..19); finding `old "a:1"`, `[replace(16, 19, "a:1@x")]` with `e1 = "services: [a:1, a:1@x]\n"` → `Ok`; same finding with `e1 = "services: [a:1@x, a:1]\n"` → `Err`; replacement `12..15` → `Err` (not a target span); two findings both at `11..14` → `Err` (not injective); an extra `insert(5, "  # a:1")` → `Err` (not at line end). Run → FAIL (helpers missing).
- [ ] GREEN: implement per P-span; messages name the fixture line and the rule.
- [ ] Run → PASS on all 17 fixtures × modes.
- [ ] Planted: (a) in `run::pin_file` collect splices with `.rev()` → `edits_touch_only…` FAILS on `ci-r1-flow-include` ☐ (note: P-bytes and P-one-comment stay green — the gap L7 names); (b) in `policy::pinned` insert the comment at `t.end()` instead of `trailing_comment(..)` → FAILS ☐. Revert each.
- [ ] Gate → 195. Commit: `test: span-based edit check over all fixtures (review L7)`.

## Task Y3 — combined comment in byte order (review L1) + core regression tests (review L7)

**Files:** `crates/pin-core/src/run.rs` (`pin_file` splice collection; `resolver_tests::Fake` gains `touch_gitlab: bool`), `crates/pin-core/tests/run_pin_file.rs`.

- [ ] RED (`run_pin_file.rs`, existing fake): `combined_comment_in_byte_order`: text `"k: [p, q]\n"`, findings in this order `[line 0: replace(7, 8, "Q") + insert(9, "  # q"), line 0: replace(4, 5, "P") + insert(9, "  # p")]` → file `"k: [P, Q]  # p, q\n"`, rows keep finding order (`q` row first) → FAILS (today `"  # q, p"`).
- [ ] GREEN (Y-D12): in `pin_file`, after `mark_overlaps`, splices = `coalesce_comments` over findings stably ordered by `(f.line, f.edits.iter().map(|s| s.start).min().unwrap_or(usize::MAX))`; `findings`/rows untouched.
- [ ] Regression tests (green on arrival; the branch they guard is planted): `insertion_inside_other_replacement_marks_both_unresolved` (text `"abcdefghij\n"`, `replace(2, 6, "X")` vs `insert(4, "  # y")` → both `unresolved`, reason `internal: overlapping edits`, file unchanged) · `independent_finding_written_next_to_overlap` (`replace(0, 3, "A")`, `replace(2, 5, "B")`, `replace(7, 9, "C")` → first two `unresolved`, third pinned, file = text with `[7, 9)` → `"C"`) · `run.rs` `gitlab_target_builds_only_gitlab` (fake calls `ctx.gitlab()` once → `resolvers_built() == ["gitlab"]`).
- [ ] Planted: (a) drop the insertion-inside branch of `overlaps` → `insertion_inside…` FAILS ☐; (b) build the gitlab cell in `Ctx::new` → `gitlab_target_builds_only_gitlab` FAILS ☐. Revert each.
- [ ] Gate → 199; snapshots unchanged (GitlabCi scan order = byte order). Commit: `fix: combined trailing comment in byte order, not scan order (review L1) + overlap/laziness regression tests (L7)`.

## Task Y4 — `core::yaml` scan: dependency, walker, byte spans, parse errors

**Files:** root `Cargo.toml`, `Cargo.lock`, `crates/pin-core/{Cargo.toml, src/lib.rs, src/text.rs}`, create `crates/pin-core/src/yaml.rs`.
**Produces:** `LineIndex::line_of`; `yaml::{PathSeg, Style, ScalarRef, Node, ScanOpts, scan}` (fix-ups Y5, masking Y6).

Tests use a local `fn scalars(t: &str) -> Vec<(String /*path "job.services[1].name"*/, String /*value*/, String /*text[span]*/, usize /*line*/)>`.

- [ ] RED: `line_of_maps_bytes_to_lines` (`text.rs`): `"a\r\nbb\nc"` → `line_of` 0→0, 2→0, 3→1, 6→2, 7→2; `LineIndex::new("").line_of(0) == 0` · `scan_paths_values_and_plain_spans`: `"image: alpine:3.20  # c\njob:\n  services:\n    - redis:7\n    - name: node:20\n"` → exactly `[("image", "alpine:3.20", "alpine:3.20", 0), ("job.services[0]", "redis:7", "redis:7", 3), ("job.services[1].name", "node:20", "node:20", 4)]` · `scan_documents_reset_paths`: `"a: 1\n---\nb: 2\n...\n---\n- c\n"` → `(doc, path, value)` = `(0, a, 1)`, `(1, b, 2)`, `(2, [0], c)`; `scan("")` and `scan("# only\n")` → `Ok(vec![])` · `scan_char_to_byte_non_ascii_and_bom`: `"\u{FEFF}k: x\nä: [y, 🚀z]\nimg: alpine:3.20\n"` → tokens `x`, `y`, `🚀z`, `alpine:3.20`; `alpine` span = `text.find("alpine:3.20")` .. +11; path `ä[1]` · `scan_crlf_spans_exclude_cr`: `"a: x\r\nb:\r\n  - y\r\n"` → `("a","x","x",0)`, `("b[0]","y","y",2)` · `scan_parent_ids_and_flow_flag`: `"include: {project: a, ref: b}\ninclude:\n  - project: c\n    ref: d\n"` → `parent(a) == parent(b) != parent(c) == parent(d)`, `in_flow` true for a, b, false for c, d · `scan_error_has_line_and_msg`: `"a: 1\nb: c: d\n"` → `Err(ScanError { line: Some(2), msg: "invalid YAML: mapping values are not allowed in this context".into() })`. Run → FAIL (module missing).
- [ ] GREEN: manifests (snippet), `line_of`, walker per the table **without** the tag/skip rows and with raw spans (Y5 adds them); `Style` from `ScalarStyle` (`Literal | Folded` → `Block`).
- [ ] Deps evidence: `git diff Cargo.lock` adds exactly the packages `saphyr-parser 0.3.3` and `arraydeque 0.5.1` (+ dependency lines); `cargo tree -p pin-core --no-default-features -e normal | grep -c saphyr` → 0.
- [ ] Planted: build the table from `bytes()` instead of `char_indices()` (i.e. treat `index()` as byte) → `scan_char_to_byte…` FAILS ☐; revert.
- [ ] Gate + matrix (now with `--features yaml`) → 206. Commit: `feat: M5c-y core::yaml scan (saphyr-parser behind feature yaml): paths, byte spans, parse errors`.

## Task Y5 — `core::yaml` fix-ups: quoted re-scan, properties, aliases, tags, block scalars

**Files:** `crates/pin-core/src/yaml.rs`.

- [ ] RED: `quoted_spans_end_at_closing_quote`: `"a: \"x:1\" # c\nb: 'it''s:1'   # c\nc: [\"d:1\" , 'e:1' ]\nd: \"q\\\"q\"  \n"` → tokens `"x:1"`, `'it''s:1'`, `"d:1"`, `'e:1'`, `"q\"q"`; values `x:1`, `it's:1`, `d:1`, `e:1`, `q"q` · `properties_are_skipped`: `"a: &x v1\nb: !!str v2\nc: &y !!str \"v3\"\nd: !!str\n  v4\n"` → tokens `v1`, `v2`, `"v3"`, `v4`; `anchor` `Some(_)` for a, c, `None` for b, d (if saphyr rejects line `d`, drop it and say so in the commit body) · `aliases_are_alias_nodes`: `"a: &x v\nb: *x\nc:\n  <<: *x\n"` → `[Scalar a, Alias{path [b], span "*x"}, Alias{path [c, <<], span "*x"}]` · `local_tags_are_opaque_core_tags_transparent`: `"a: !reference [.b, c]\nd: !!seq [e]\nf: !custom g\n"` → `[Tagged{[a], "!reference"}, Scalar d[0] "e", Tagged{[f], "!custom"}]` and no node under `a` · `block_scalars_keep_style_and_content_span`: `"a: |\n  x\nb: >-\n  y\nc: |\n  !x\n"` → all `Style::Block`, values `"x\n"`, `"y"`, `"!x\n"`, `text[span]` starts with `x` / `y` / `!x`, lines 1, 3, 5 · `scan_never_panics_on_prefixes` (P-nopanic over the texts of every `yaml.rs` test, both mask settings). Run → FAIL.
- [ ] GREEN: `token_span`, the tag and skip rows of the walker, flow detection after property skip.
- [ ] Planted: (a) remove step 2 of `token_span` → `quoted_spans…` FAILS ☐; (b) apply the property skip unconditionally (spike behaviour) → `block_scalars…` FAILS on `c` ☐. Revert each.
- [ ] Gate + matrix → 212. Commit: `feat: M5c-y core::yaml span fix-ups (quoted end, properties), aliases, opaque local tags, block scalars`.

## Task Y6 — `core::yaml` template masking

**Files:** `crates/pin-core/src/yaml.rs`.

- [ ] RED: `mask_keeps_offsets_and_line_breaks` (unit on `mask_templates`): `"a: {{ .x }}\n{{- if y\n }}\nb: \"r:{{ ü }}\"\nc: {{ open\n"` → P-mask holds, ranges = the three closed `{{…}}`, the unclosed one untouched · `masked_scalars_are_templated`: mask on, `"img: \"r:{{ .t }}\"\nplain: r:1\n{{- if .x }}\nother: v\n{{- end }}\n"` → `Ok`; `img` `templated`, `text[span] == "\"r:{{ .t }}\""` (original text), `plain`/`other` not templated · `unmasked_template_is_not_an_error` (characterisation, doc: "M8a decides"): `"image: {{ .x }}\n"` mask off → `Ok`, no scalar at `[image]` with a non-empty value; mask on → scalar at `[image]` with value `""` and `templated == false`. Run → FAIL.
- [ ] GREEN: `mask_templates`; `scan` parses `src` = masked text when `opts.mask_templates`; `templated` per Y-D7.
- [ ] Planted: mask newlines too → `mask_keeps…` FAILS ☐; revert.
- [ ] Gate + matrix → 215. Commit: `feat: M5c-y core::yaml {{ }} masking with templated flag (offsets and lines kept)`.

## Task Y7 — `PinTarget::Unsupported` → `skipped (<reason>)`

**Files:** `crates/pin-gitlab-ci/src/{discover.rs (variant + accessors + one test arm), lib.rs, policy.rs}`, `tests/plugin.rs`.

- [ ] RED: `unsupported_target_is_skipped_without_client`: text `"image: >-\n  alpine:3.20\n"`, `t = PinTarget::Unsupported { kind: "image", line: 1, start: 12, end: 24, value: "alpine:3.20".into(), reason: "block scalar" }` → `resolve` gives `Skipped`, reason `block scalar`, `kind "image"`, `old == text[12..24]`, no edits, `resolvers_built()` empty. Fails to compile.
- [ ] GREEN: variant, the four accessors, `target_kind`, `resolve` arm, `policy::unsupported`. Existing test `combined_targets_sorted_by_line_then_start` gets `PinTarget::Unsupported { .. } => unreachable!("not produced")` (reason: exhaustive match over a grown enum).
- [ ] Planted: map `Unsupported` to `Status::Ok` → test FAILS ☐; revert.
- [ ] Gate → 216; snapshots unchanged. Commit: `feat: M5c-y PinTarget::Unsupported reported as skipped`.

## Task Y8 — `discover_yaml`: images and components; parity (image, component)

**Files:** `crates/pin-gitlab-ci/Cargo.toml` (`features = ["oci", "gitlab", "yaml"]`), `src/discover.rs` (`discover_yaml`, `is_image_path`, `is_component_path`, unit tests), `tests/discovery_parity.rs`.
**Consumes:** Y4–Y6 `yaml::scan`, Y7 variant, Y1 record.

- [ ] RED (unit, `discover.rs`): `yaml_image_paths`: text below → values in order `a:1, b:1, c:1, d:1, e:1, f:1, j:1, k:1, l:1`, all `Image` (not `h:1`, `i:1`, `g`, `""`):
  `image: a:1 / services: [b:1, {name: c:1}] / default: / "  image: {name: d:1, entrypoint: [\"\"]}" / job: / "  services:" / "    - e:1" / "    - name: f:1" / "      alias: g" / "  script: [h:1]" / "  deep:" / "    image: i:1" / bäckend: / "  image: j:1" / "\"quoted\":" / "  image: k:1" / "flowjob: {image: l:1}"`
  · `yaml_component_root_only`: `"component: gitlab.com/g/p/n@1.0.0\njob:\n  component: gitlab.com/g/p/n@2.0.0\n"` → one `Component`, parts `(gitlab.com, g/p, n, 1.0.0)` · `yaml_block_scalar_is_unsupported`: `"image: >-\n  alpine:3.20\ncomponent: |\n  gitlab.com/g/p/n@1.0.0\n"` → `[Unsupported{kind "image", line 1}, Unsupported{kind "component", line 3}]`, reason `block scalar` · `yaml_aliases_and_references_are_not_targets`: `".x: &x alpine:3.20\na:\n  image: *x\nb:\n  image: !reference [.base, image]\n  services: !reference [.base, services]\n"` → `Ok(vec![])`.
- [ ] RED (`discovery_parity.rs`): `yaml_discovery_matches_legacy_except_documented` (P-parity with `const PARITY_KINDS: &[&str] = &["image", "component"]` applied to legacy lines, new lines and both exception lists; header hash mismatch → failure "fixture {name} changed after the legacy record") · `spans_are_tokens_over_all_fixtures` (P-token). Run → FAIL (`discover_yaml` missing).
- [ ] GREEN: `discover_yaml` per the matcher (image + component only; include in Y9).
- [ ] Run → PASS. The exception table matches exactly (Y8 sees the image entries only).
- [ ] Planted (spec §11): (a) in `yaml.rs` use `m.index()` as the byte offset → `scan_char_to_byte…` and parity on `ci-unicode`/`ci-bom` FAIL ☐; (b) drop the quoted re-scan → parity on `ci-quoted-comment` + `quoted_spans…` FAIL ☐; (c) delete the `ci-anchors` line-6 entry from `REMOVED` → parity FAILS "unmatched legacy line" ☐. Revert each.
- [ ] Gate + matrix → 222; snapshots unchanged (old `discover` still wired). Commit: `feat: M5c-y gitlab-ci image/component discovery over core::yaml (parity vs legacy record)`.

## Task Y9 — `discover_yaml`: includes; parity complete

**Files:** `crates/pin-gitlab-ci/src/discover.rs` (`is_include_mapping`, grouping by `parent`, unit test), `tests/discovery_parity.rs` (`PARITY_KINDS` + `"include"`).

- [ ] RED: widen `PARITY_KINDS` → parity FAILS on every include line ☐ (expected red) · unit `yaml_include_forms` (each case → expected `(line, value, project, flow)` or no target): flow single `include: { project: g/p, ref: main }` → `(0, main, g/p, true)`; flow seq `[ {project: g/p, ref: main}, "x.yml", {project: g/q} ]` → one; block seq item with `file:` and nested `inputs:\n      a: 1` → one, flow false; block single, `ref` before `project` → one; two root `include:` keys in one document → one target each; `ref:` empty → none; `project: "g/p"` + `ref: 'main'` → span `'main'`; `job:\n  include: {project: g/p, ref: main}` → none; `include:\n  - component: gitlab.com/g/p/c@1.0` → none (G1); `include: [{project: g/p, ref: main, inputs: {a: 1}}]` → one (superset); `include:\n  project: g/p\n  ref: >-\n    main\n` → `Unsupported{kind "include"}`.
- [ ] GREEN: include grouping per the matcher.
- [ ] Planted: group by `path` instead of `parent` → parity FAILS on `ci-include.yml` (three `include:` keys) ☐; revert.
- [ ] Gate → 223; snapshots unchanged. Commit: `feat: M5c-y gitlab-ci include discovery over core::yaml (parity complete)`.

## Task Y10 — switch: yaml discovery live; parse errors are file errors; R7/R8/R9 snapshot diffs

**Files:** `crates/pin-gitlab-ci/src/{discover.rs, lib.rs}`, `tests/{common/mod.rs, discovery_parity.rs, plugin.rs}`, 12 snapshots, `README.md`.

- [ ] RED (`plugin.rs`): `scan_invalid_yaml_is_scan_error`: `GitlabCi::default().scan(path, "a: 1\nb: c: d\n", &defaults)` → `Err(ScanError { line: Some(2), msg: "invalid YAML: mapping values are not allowed in this context".into() })`; `"job:\n\timage: alpine:3.20\n"` → `Err` with `line == Some(2)` and `msg.starts_with("invalid YAML: ")` · `invalid_yaml_file_is_parse_error_and_untouched`: `.tmp/plugin-badyaml-{pid}/ci.yml` = `"image: alpine:3.20\nb: c: d\n"`; `pin_file(&GitlabCi::default(), &ctx, &p, &defaults, &RunOptions { dry_run: false, check: false, concurrency: 4 })` → `Err(FileError::Parse { path, line: Some(2), msg })` with the same `msg`; file bytes unchanged; `ctx.resolvers_built()` empty. Both FAIL (old scan never errs).
- [ ] GREEN: rename old `discover` → `#[doc(hidden)] pub fn legacy_discover`, `discover_yaml` → `discover`; `GitlabCi::scan` = `Ok(discover(text)?…filter…)`, doc "`Err` = the file is not valid YAML (§6.9)". Mechanical test edits (constraint 2): 20 unit tests `discover(x)` → `discover(x).unwrap()`; `common::pin_once` and `check_spans` → `.expect("fixture parses")`; generator test → `legacy_discover`; parity tests → `discover`.
- [ ] Snapshots: `INSTA_UPDATE=always cargo test -p pin-gitlab-ci --test fixture_snapshots`; `git status` must list exactly `fixture_snapshots__ci-{anchors,r2-crlf,tags,bom}__pin__{human,json,file}.snap`. Expected: `ci-anchors` 3 rows pinned (lines 1, 3, 9; was 4 skipped), file `image: &img alpine:3.20@sha256:…`, `  image: ubuntu:24.04@sha256:…`, `    - &redis redis:7@sha256:…`, alias lines unchanged · `ci-r2-crlf` + 3 pinned rows (lines 6, 9, 10), file `    ref: <sha>  # dev␍`, `    - redis:7@sha256:…␍`, `  image: "node:20@sha256:…"␍` · `ci-tags` rows for lines 5, 6 gone, line 8 pinned, file `  image: !!str node:20@sha256:…` · `ci-bom` + row line 2 pinned. Every other snapshot, all e2e tests and all properties unchanged.
- [ ] README (`Components & includes` and `What gets skipped`, `Exit codes`): replace "Known gap until M5c-y: block children in CRLF files are not discovered." by: "Discovery reads the YAML structure (M5c-y): anchored values (`&a`, `!!str`) are pinned at their definition; aliases (`*a`), merge keys (`<<:`) and `!reference [...]` are not targets. CRLF files, children of anchored keys (`.base: &base`) and a first key after a BOM are discovered (R7, R8, R9; before: missed or `skipped (unparseable reference)`)." Add skip reason `block scalar` (a `|`/`>` value at an image/component/include path is never rewritten). Add: "A file that is not valid YAML (e.g. tab indentation) is an error for that file (`<path>:<line>: invalid YAML: …`); nothing in it is changed. `pin-ci` exits 1 on it."
- [ ] Planted: point `GitlabCi::scan` back to `legacy_discover` → the 12 snapshots and both new tests FAIL ☐; revert.
- [ ] Gate + matrix → 225. Commit: `fix: R7/R8/R9 gitlab-ci discovery on core::yaml; invalid YAML is a file error (snapshots updated)`, body lists the 12 snapshot files.

## Task Y11 — delete the hand-rolled discoverer (refactor)

**Files:** `crates/pin-gitlab-ci/src/discover.rs`, `tests/discovery_parity.rs`.

- [ ] Evidence before: `INSTA_UPDATE=no cargo test --workspace --all-features > .tmp/y11-before.log 2>&1`.
- [ ] Delete `legacy_discover` and every primitive only it uses (`component_key`, `push_component_target`, `image_key`, `image_key_colon`, `top_key`, `push_target`, `push_flow_targets`, `push_flow_item`, `dash_item_rest`, `push_item_targets`, `name_value_offset`, `push_name_child_target`, `push_if_parses`, `scalar_token`, `IncludeBlock`, `PendingInclude`, `include_key`, `push_include_target`, `flow_close`, `split_flow_items`, `key_value`, `push_include_flow_pairs`, `push_include_flow_mapping`, `push_include_flow_sequence`, `handle_include_inner`, `record_include_key`) and the generator test `legacy_record_matches_discover` (its subject is gone; the record file stays, frozen). Rewrite the module doc: "Discovery over `pin_core::yaml` (M5c-y): path rules …" (the matcher list).
- [ ] Evidence after: `diff <(grep '^test ' .tmp/y11-before.log | sort) <(grep '^test ' .tmp/y11-after.log | sort)` = exactly the removed `legacy_record_matches_discover`; no `*.snap` change; `git diff --stat` shows `discover.rs` −≥ 750 lines; `grep -nE 'split_inclusive|trim_start_matches' crates/pin-gitlab-ci/src/discover.rs` → empty.
- [ ] Gate + matrix → 224. Commit: `refactor: M5c-y delete the hand-rolled YAML discoverer (legacy record kept for parity)`.

## Close-out (controller, before merge)

- [ ] Gate + matrix green; `git log --oneline main..` = 11 commits Y1–Y11 in order.
- [ ] `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --all-features` → 0 warnings (old intra-doc links are gone with Y11).
- [ ] Coverage gap 1 and 2 by hand; record "not run" if network/devenv is unavailable.
- [ ] If main approves (Open for main 1, 2, 4): one docs commit on the spec — §9 R-table row R9 (BOM), R7 wording incl. `!tag`/`!reference`, §6.9 `core::yaml` as built (`ScanError`, `line`/`parent`/`in_flow`, opaque local tags, newline-keeping mask), §10 M5c-y done.

## Self-review

- **Spec coverage:** §6.9 table rows: char→byte (Y4), quoted re-scan (Y5), properties (Y5), comments/aliases/merge/`!reference` (Y5, Y8), sticky error + masking (Y4, Y6, Y10), block scalars (Y5, Y7, Y8), CRLF/multi-doc (Y4, fixtures). §6.9 shape: `scan`, `Node`, `ScalarRef` (+3 fields, Y-D3), `ScanOpts`. Parse error = per-file error, no fallback (Y10). §9 M5c-y: path matching (Y8, Y9), primitives deleted (Y11), R7/R8 diffs (Y10), parity (Y1, Y8, Y9). §11: parity incl. token property, fixture list (Y1; masked `{{ }}` at unit level), planted `index()`/re-scan (Y8). §6.1: feature `yaml`, +2 crates (Y4). §14.1: L1 (Y3), L7 (Y2, Y3), L3 deferred (Y-D14).
- **Spec vs plan differences (Open for main):** R9 is new; R7 widened to tags/`!reference`; `ScanError` instead of `YamlError`; `ScalarRef` extra fields; mask keeps newlines; exit code 1 for parse errors until M5d.
- **Type consistency:** `scan(text, ScanOpts) -> Result<Vec<Node>, ScanError>` in Y4–Y10; `ScalarRef.{span, line, parent, in_flow, style}` used by Y8/Y9 exactly as defined; `PinTarget::Unsupported { kind, line, start, end, value, reason }` in Y7/Y8/Y9; `discover_yaml` (Y8–Y9) = `discover` (Y10+), `legacy_discover` only in Y10; `plan_once`/`check_spans` signatures in Y2/Y10; `LineIndex::line_of` in Y4/Y2.
- **Planted violations:** every new check has a box; Y11 is a refactor with before/after evidence.
- **Proportion:** bodies only for the walker table, `token_span`, `mask_templates`, the matcher rules and the exception table — the decisions the signatures and tests do not fix.

## Open for main (recommended default in bold)

1. **R9 (new, found while planning):** today's `discover()` misses every child of a BOM-prefixed first key (`\uFEFF default:` → `default.image` not found; `top_key` rejects the BOM). **Keep `ci-bom.yml` with the key on line 1 and list R9 as a third documented parity exception; add a spec §9 row at close-out.** Alternative: first line a comment (no exception, BOM covered for offsets only).
2. **R7 scope:** `ci-tags.yml` shows the same bug class for `!!str` and `!reference` (old: unparseable targets; new: tag skipped / not a target). **Treat as R7** (spec §6.9 already says "skip the properties", "`!reference` not a target").
3. **`ScanError` instead of `YamlError`** from `core::yaml::scan` (identical fields). **Accept.**
4. **`ScalarRef` gains `line`, `parent`, `in_flow`; non-core-tagged nodes are opaque `Tagged`.** **Accept**; spec §6.9 updated at close-out.
5. **Parse errors keep exit 1 in `pin-ci`** and the driver stops at the first file error until M5d. **Accept** (M5d owns exit 2 and per-file continuation).
6. **Keep the parity test and the frozen record after M5c-y** (G1 adds `include: - component:` targets → G1 adds them to `ADDED` or retires the test). **Keep until G1.**
7. **Review L3 deferred to M5d** (Y-D14). **Accept.**
