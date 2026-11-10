# M4b-1 plan — minimum release age guard

- Date: 2026-11-09
- Source of truth: spec M4b-1 (docs/superpowers/specs/2026-11-01-pin-ci-design.md, commit pending) + design memo `.superpowers/sdd/2026-11-09-pin-ci-m4b1-release-age/design.md` (probe evidence, rulings R3-R6, implementation shape)
- Base: `main` @ 0b4d187 (post M4a merge). Branch: `m4b1-release-age`.

## Goal

`--min-age-days <N>` (default 0 = off): with `--update`, a docker.io bump
candidate must be at least N days old (timestamps from the Docker Hub API,
`tag_last_pushed`/`last_updated`). No eligible candidate → ref unchanged,
`skipped (release age: no candidate ≥ {N}d)`, exit 0. Non-docker.io hosts:
documented no-op (R3). Same-day oracle vs the old pipeline (14d
`timestamp-required`) must now match, or the divergence must be
explainable by the age boundary.

## Architecture

| Module | Change | Task |
|---|---|---|
| `src/registry.rs` | `tag_timestamps(&ImageRef) -> Result<BTreeMap<String, i64>, RegError>` (Hub API, `next` pagination, page_size=100, `tag_last_pushed` → `last_updated` fallback, values = unix seconds); private RFC3339→unix helper + `is_old_enough(ts, now_unix, days)`; `hub_base_url` test-override field (same pattern as `base_urls`) | T1 |
| `src/lib.rs` | `PinOptions.min_age_days: u64`; update path: double-pick guard (unfiltered pick vs age-filtered pick) → update / skip / fall-through per R5-R6; private `now_unix()` | T2 |
| `src/main.rs` | `--min-age-days <N>` flag (default 0, help: "with --update: skip bump candidates younger than N days (Docker Hub refs only; 0 = off; recommended 14)") | T2 |
| `tests/registry_hosts.rs` (or new `tests/hub_api.rs`) | wiremock Hub API suite | T1 |
| `tests/e2e.rs` | guard e2e suite | T2 |
| `tests/oracle.rs` | M2 divergence assertion → age-explained relaxation | T3 |
| `README.md` | flag docs + 14d recommendation + docker.io-only note | T3 |

No new dependencies (RFC3339 parse hand-rolled — no time/chrono in the
crate, M1-M4a zero-dep history).

## Global constraints

1. **R3: docker.io only.** Guard active iff `opts.min_age_days > 0` AND ref host is docker.io (the no-slash default). Other hosts: M2 behavior, unchanged, no status change.
2. **R4: default off.** `--min-age-days 0` (the default) = exact M2 behavior — all M2 e2e/live/oracle tests pass unchanged when the flag is absent.
3. **R5: skip semantics.** Unfiltered pick = Some(c) with c != current, filtered pick = None or == current → status `skipped`, reason `release age: no candidate ≥ {N}d`, no replacement (ref byte-identical), exit 0.
4. **R6: filter point.** Age filter applied to the full tag list BEFORE pick; a tag missing from the timestamp map = NOT old enough (conservative, matches old config `timestamp-required`). Current tag is never a candidate (M2 rule untouched).
5. **Timestamps.** `tag_last_pushed` preferred, `last_updated` fallback, both absent for a tag = that tag has no entry. Hub API 404 repo → Ok(empty map) (→ everything fails the age filter → R5 skip when unfiltered candidates existed). Other Hub errors (network, 429, malformed page) → `RegError` → the existing update-path mapping to `unresolved (tag list: …)`-style reason (use reason prefix `hub api: {msg}` for the timestamps call).
6. **No 429 retry** (M4a precedent). Page cap 100; follow absolute `next` URLs until absent.
7. **Report format unchanged** (line + JSON schema); the skip reuses the existing `skipped (reason)` shape.
8. **Test clock.** Wiremock fixtures use far-past timestamps (e.g. `2020-01-01T00:00:00Z`) and far-future ones (`2999-01-01T00:00:00Z`) so wall-clock can't flip outcomes; the only wall-clock-dependent test is the live/oracle one (T3), which asserts relative properties, not fixed picks.
9. Env/commits: `CARGO_HOME=$PWD/.cargo-home`, rust at `/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin`, unsigned commits, `git -c core.excludesFile=/dev/null`, qwen3.8 implementers + inline controller reviews, scratch `.tmp/`.

## Files

| File | T1 | T2 | T3 |
|---|---|---|---|
| `src/registry.rs` | R | | |
| `src/lib.rs` | | R | |
| `src/main.rs` | | R | |
| `tests/hub_api.rs` (new) | A | | |
| `tests/e2e.rs` | | R | |
| `tests/oracle.rs` | | | R |
| `tests/live.rs` | | | R (optional live guarded update) |
| `README.md` | | | R |

## Interfaces (cross-task contract)

```rust
// src/registry.rs (T1)
impl RegistryClient {
    /// Push timestamps (unix seconds) for every tag of a docker.io ref,
    /// from the Docker Hub API. Non-docker.io: Ok(empty).
    /// `library/` mapping: no-slash ref → library/<repo> (M1 api_repo rule).
    pub async fn tag_timestamps(&self, r: &ImageRef) -> Result<BTreeMap<String, i64>, RegError>;
    // constructor gains hub_base_url override (test hook), default
    // "https://hub.docker.com/v2" — same HashMap pattern as base_urls.
}
// private, unit-tested:
fn parse_rfc3339_unix(s: &str) -> Option<i64>;   // Z or ±HH:MM offset, optional .frac
fn is_old_enough(ts_unix: i64, now_unix: i64, min_days: u64) -> bool;  // ts <= now - days*86400

// src/lib.rs (T2)
pub struct PinOptions { /* existing + */ pub min_age_days: u64 }
impl Default for PinOptions { /* every field at its off/default value (pin-toolkit spec §9) */ }
// update path, after list_tags:
//   let candidate = versioning::pick_latest_stable(tag, &tags, opts.update_major);
//   if guard active (min_age_days>0 && host docker.io):
//       stamps = client.tag_timestamps(r).await (Err → unresolved, reason "hub api: …")
//       tags_age = tags filtered to stamps.get(t).is_some_and(old enough)
//       let age_candidate = pick_latest_stable(tag, &tags_age, opts.update_major);
//       match (candidate, age_candidate):
//         (Some(c) if c != tag, None | Some(s) if s == tag) => skipped (R5 reason)
//         (Some(c), Some(a) if a != tag) => update to `a` (digest + splice, M2 shape)
//         _ => fall through (pin path, M2 behavior)
//   else: M2 behavior verbatim.
```

## Task 1 — Hub API timestamps client

Files: `src/registry.rs` (primary), new `tests/hub_api.rs`.

**RED** — unit (inline `#[cfg(test)]` in registry.rs):

```
rfc3339_parse_table            verified vectors (controller, `date -ud`, 2026-11-09):
                                "1970-01-01T00:00:00Z" → 0;
                                "2026-11-08T14:24:40.263592Z" → 1794147880 (frac dropped);
                                "2024-02-29T23:59:59+02:00" → 1709243999 (leap day + offset);
                                "2999-01-01T00:00:00Z" → 32472144000;
                                "2020-01-01T00:00:00+00:00" → 1577836800 == "…Z" form;
                                garbage ("2026-13-01T00:00:00Z", "not-a-date") → None
is_old_enough_table            boundary: ts == now - N*86400 → true (≥); 1s newer → false;
                                N=0 → any ts ≤ now true
```

wiremock (`tests/hub_api.rs`, client with `hub_base_url` → mock):

```
hub_single_page                /v2/repositories/library/alpine/tags?page_size=100 → 1 page,
                               2 tags (one with tag_last_pushed only, one with last_updated only)
                               → map has both; request assertion: page_size=100 param
hub_pagination                 page 1 with next → page 2 (absolute URL) → merged map; both
                               requests observed (expect(1) each)
hub_tag_last_pushed_wins       entry with both fields → tag_last_pushed value
hub_404_repo_empty             404 → Ok(empty)
hub_429_error                  429 → Err(RegError) (no retry)
hub_malformed_page_error       200 without "results" array → Err(RegError)
hub_nonhub_ref_empty           tag_timestamps(ghcr.io/…) → Ok(empty), ZERO mock requests
```

**GREEN** — implement per Interfaces + constraints 5-6.

**Check:** `export CARGO_HOME="$PWD/.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH"; cargo test && cargo fmt --check`

**Commit:** `feat: Docker Hub tag push-timestamps (paginated, RFC3339 parse)`

## Task 2 — guard in the update path + flag

Files: `src/lib.rs`, `src/main.rs`, `tests/e2e.rs`.

**RED** — e2e (wiremock registry + hub, temp file in `.tmp/`; follow the existing M2 update e2e setup):

```
guard_young_top_dropped        tags [1.4(future ts), 1.3(old ts), 1.2(old ts)] current 1.2,
                               --update --min-age-days 14 → updated to 1.3 (NOT 1.4), digest
                               of 1.3 pinned; exit 0
guard_no_eligible_skip          tags [1.3(future), 1.2(future)] current 1.1 → skipped
                               (release age: no candidate ≥ 14d); file byte-identical; exit 0
guard_missing_timestamp_excluded tags [1.3(no hub entry), 1.2(old)] current 1.1 → updated to 1.2
guard_off_default_m2_unchanged  same fixture, NO flag → M2 behavior (updates to 1.4)
guard_nonhub_noop               ghcr host mock, flag set → M2 behavior (no hub calls: mock
                               expect(0) on the hub base)
flag_default_zero               build_opts default min_age_days == 0; "--min-age-days 7" → 7
```

**GREEN** — per Interfaces (double-pick) + constraint 3; main.rs flag. Also (pin-toolkit spec §9, 2026-11-09): add `impl Default for PinOptions` (every field at its off/default value, `min_age_days = 0`) and convert ALL existing `PinOptions { … }` literals (build_opts + test files) to `..Default::default()` style, keeping currently-explicit fields explicit — this shrinks the M5 workspace-extraction churn. And remove the `#[allow(dead_code)]` on `registry::is_old_enough` (the guard now uses it).

HARD REQUIREMENT: all pre-existing tests green unchanged (R4; the literal conversion is compile-checked).

**Check:** same cargo test + fmt + clippy.

**Commit:** `feat: --min-age-days release age guard for --update (docker.io)`

## Task 3 — oracle relaxation + live + README + final gate

Files: `tests/oracle.rs`, optionally `tests/live.rs`, `README.md`.

- **Oracle (the payoff):** the M2 test `m2_oracle_update_matches_old_pipeline` currently allows one documented divergence (old 14d lag). New logic: for any old≠new tag pair, fetch both tags' Hub `tag_last_pushed` (real network, PIN_CI_ORACLE-gated) and assert the divergence is age-explained: `old_pick_age_days ≥ 14` AND (`new_pick_age_days < 14` OR new pick does not exist as an old pick). No divergence → pass as before. If neither tag has a Hub timestamp → FAIL loudly (data bug, not a divergence).
- **Live (optional, PIN_CI_LIVE):** a real `--update --min-age-days 14` run on a scratch file with `alpine:3.19` (known multi-year-old) → must update to the current 3.x latest (which is >14d old) and pin its digest.
- **README:** flag row in the CLI table + "Release age guard (M4b-1)" section: semantics, docker.io-only (R3), recommended 14 (old pipeline parity), no-timestamp = excluded (R6), no eligible → skipped.
- **Final gate (controller, before merge):** full suite; `PIN_CI_LIVE=1 cargo test`; `PIN_CI_ORACLE=1` oracle run (devenv: renovate+crane on PATH); release build; merge to main (unsigned), ledger close-out.

**Commit:** `test: release-age oracle relaxation + live; docs: README M4b-1 section`

## Review Focus (inline reviews)

1. **M2 behavior byte-identical with flag off** — all M2 e2e/live/oracle pass untouched (R4 is the contract).
2. **RFC3339 math** — leap day, offsets, fractional seconds; boundary `ts == cutoff` → old enough.
3. **Double-pick matrix** — the 4 (candidate, age_candidate) combinations map exactly to R5/R6/fall-through; the `skipped` path writes nothing.
4. **Hub error mapping** — 404 repo → empty map (→ skip via R5, not error); 429/network/malformed → `unresolved (hub api: …)`; no retry.
5. **Oracle relaxation soundness** — divergence accepted ONLY with fetched timestamps proving the 14d boundary; missing timestamps = loud fail (never a silent pass).
6. **No new deps** — Cargo.toml untouched.

## Self-review (controller, post-write)

- Task boundaries: T1 = data (client + parse, wiremock); T2 = policy + flag (e2e); T3 = oracle payoff + docs. Each one commit, one check. ✓
- Interface consistency: `tag_timestamps` returns `BTreeMap<String, i64>` (deterministic iteration for tests); the double-pick uses the existing `pick_latest_stable` signature twice — no versioning changes. ✓
- Spec M4b-1 coverage: flag + default ✓ T2, Hub API source ✓ T1, docker.io-only ✓ R3, no-eligible skip ✓ R5, 14d recommended + oracle ✓ T3. ✓
- Risk: T3's oracle fetches real Hub timestamps inside a PIN_CI_ORACLE-gated test — new network surface in the oracle; mitigated by the loud-fail-on-missing-timestamp rule and the fact the oracle already shells out to renovate+crane+git.
