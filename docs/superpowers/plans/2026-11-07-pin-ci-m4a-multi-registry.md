# M4a plan — multi-registry support (docker v2, Bearer + Basic)

- Date: 2026-11-07
- Source of truth: spec M4a (docs/superpowers/specs/2026-11-01-pin-ci-design.md, commit pending) + design memo `.superpowers/sdd/2026-11-07-pin-ci-m4a-multi-registry/design.md` (owner rulings R1/R2, live probe evidence, full design)
- Base: `main` @ ea43e2e (post M3 merge). Branch: `m4a-multi-registry`.

## Goal

Every docker-v2 registry host resolves by default: GHCR, GCP Artifact Registry, ECR public (anonymous) / private (via `docker login` config), JFrog Artifactory, any self-hosted registry. The client handles both `Bearer` and `Basic` `WWW-Authenticate` challenges. Per-host credentials from env (`DOCKERHUB_USER`/`DOCKERHUB_TOKEN`, `GITHUB_TOKEN`) and `~/.docker/config.json` (username/password → Basic; identitytoken → Bearer). No credentials + auth demanded → `unresolved` with a login hint (fail loud, exit 1) — ruling R1.

## Architecture

| Module | Change | Task |
|---|---|---|
| `src/registry.rs` | `HostCreds` enum + `Creds` per-host map with `load_from`/`for_host`; token request auth generalizes to Basic **and** Bearer per host | T1 |
| `src/registry.rs` | `base_for` default `https://{host}` for any host (docker.io special-case kept); `base_urls` test-override wins | T2 |
| `src/lib.rs` | `resolve_image`: remove the `is_docker_hub()` skip — every host resolves; `client_from_env` uses new `Creds::load_from` | T1/T2 |
| `src/registry.rs` | 401 challenge dispatch: Bearer (existing path, per-host auth) / **Basic (new)** manifest retry / no-challenge; token-denied + basic-no-creds errors with per-host hints | T3 |
| `tests/e2e.rs` | M1 "non-hub skipped" expectation flips: `ghcr.io` line in `ci-e2e.yml` now maps to wiremock via `base_urls` and is `pinned` | T2 |
| `tests/registry_hosts.rs` (new) | wiremock suite: GHCR bearer-PAT, ECR-public anon, GCP identitytoken, JFrog Basic, no-creds paths | T3 |
| `tests/live.rs` | `live_ecr_public_anonymous` (always under PIN_CI_LIVE), `live_ghcr` (gated on GITHUB_TOKEN presence, skip-with-hint otherwise) | T4 |
| `README.md` | M4 section: hosts, env table, config.json, ECR-private-via-docker-login, SigV4 deferred, R1 semantics | T4 |

No new dependencies.

## Global constraints

1. **R1 fail loud.** Registry demands auth (401 anonymous; token request denied; or Basic challenge) and `for_host(host)` is `None` → `unresolved` with hint; exit 1. ghcr.io hint: `no credentials — set GITHUB_TOKEN (PAT with read:packages) or docker login ghcr.io`; generic: `no credentials — docker login {host}`. No new `skipped` reasons for images (existing ones: unparseable reference, templated — unchanged).
2. **R2 env.** `GITHUB_TOKEN` → ghcr.io Bearer (sent as `Authorization: Bearer {token}` on the TOKEN request, not Basic). `DOCKERHUB_USER`+`DOCKERHUB_TOKEN` → docker.io Basic (M1 behavior unchanged). No other env vars.
3. **docker.io behavior byte-identical.** All M1/M2 lib/mock/e2e/live/oracle tests pass UNCHANGED except the one deliberate e2e flip (constraint 6). The M1/M2 oracles still pass (they exercise docker.io only).
4. **Precedence** (per host, env before config.json): docker.io env → config keys `https://index.docker.io/v1/`, `https://docker.io`, `index.docker.io/v1/`, `docker.io` (M1 order); ghcr.io `GITHUB_TOKEN` → config keys `ghcr.io`, `https://ghcr.io`; other hosts config keys `{host}`, `https://{host}`. config.json entry: `username`+`password` → Basic; else `identitytoken` → Bearer; else none. Malformed config / missing home → no creds (never fatal).
5. **Token cache** stays keyed by scope string (M1 keying unchanged).
6. **The one deliberate flip:** the M1 e2e test asserting the `ghcr.io` line is `skipped (registry not enabled)` is rewritten — the fixture line stays; `base_urls` maps `ghcr.io` → wiremock; expectation becomes `pinned` with the mock digest. The `ci-e2e.yml` fixture otherwise unchanged.
7. **No SigV4.** ECR private works via the documented `docker login` config path; README says so.
8. **Pin semantics unchanged:** formats, statuses, comment rules, idempotency, exit codes (1 iff unresolved), report line/JSON schema, `--update` paths (they share auth, so they work for all hosts automatically — covered by existing M2 tests + one new T2 e2e asserting list_tags works on a non-hub mock host).
9. Env/commits: `CARGO_HOME=$PWD/.cargo-home`, rust at `/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin`, unsigned commits, `git -c core.excludesFile=/dev/null`, qwen3.8 implementers + inline controller reviews, scratch `.tmp/`.

## Files

| File | T1 | T2 | T3 | T4 |
|---|---|---|---|---|
| `src/registry.rs` | R | R | R | |
| `src/lib.rs` | R (client_from_env) | R (gate) | | |
| `src/image_ref.rs` | | R (is_docker_hub possibly removed) | | |
| `src/main.rs` | R (compile) | | | |
| `tests/mock_registry.rs` | R (Creds literals) | | | |
| `tests/e2e.rs` | | R (flip + list_tags non-hub) | | |
| `tests/registry_hosts.rs` | | | A | |
| `tests/live.rs` | | | | R |
| `README.md` | | | | R |

## Interfaces (cross-task contract)

```rust
// src/registry.rs (T1)
#[derive(Debug, Clone, PartialEq)]
pub enum HostCreds {
    Basic { user: String, password: String },
    Bearer { token: String },
}

#[derive(Debug, Default)]
pub struct Creds { /* hosts: HashMap<String, HostCreds> */ }
impl Creds {
    pub fn none() -> Self;
    /// Env (R2) + ~/.docker/config.json per the constraint-4 precedence table.
    pub fn load_from(home: Option<&Path>) -> Self;
    pub fn for_host(&self, host: &str) -> Option<&HostCreds>;
    /// Test hook.
    pub fn with_host(mut self, host: &str, creds: HostCreds) -> Self;
}

// T1: fetch_token gains the host and sends per-host auth:
//   Basic → req.basic_auth(user, Some(pass));  Bearer → Authorization: Bearer {token};
//   None  → anonymous. M1 docker.io behavior is the host="docker.io" case.

// T2: base_for — base_urls override, else docker.io → DEFAULT_DOCKER_HUB_BASE,
//     else https://{host}. (The "no base URL" error becomes unreachable and is removed.)

// T3: 401 dispatch in digest()/list_tags():
//   WWW-Authenticate Bearer → existing token flow (now per-host auth)
//   WWW-Authenticate Basic  → manifest HEAD retry with Basic from for_host(host)
//                             (None → Unresolved hint, constraint 1)
//   401 w/o challenge       → Unresolved (unchanged)
//   token request 401/403   → Unresolved("{host}: token denied — {hint}")
```

## Task 1 — per-host credentials

Files: `src/registry.rs` (primary), `src/lib.rs` (`client_from_env`), `src/main.rs` (compile ripple), `tests/mock_registry.rs` (Creds literal ripples).

**RED** — unit tests (registry.rs inline or a tests file, implementer's choice):

```
creds_env_dockerhub_basic      DOCKERHUB_USER+TOKEN → for_host("docker.io") = Some(Basic)
creds_env_ghcr_bearer          GITHUB_TOKEN → for_host("ghcr.io") = Some(Bearer)
creds_env_beats_config         both set → env wins (per host)
creds_config_dockerhub_keys    config.json with each of the 4 M1 keys (4 sub-cases) → Basic; order = M1 list
creds_config_identitytoken     entry {"identitytoken": "ya29…"} under "us-docker.pkg.dev" → Bearer
creds_config_arbitrary_host    entry under "artifactory.example.com" (+ https:// variant) → Basic
creds_missing_home_malformed   home=None / bad JSON / no auths → Creds::none(), no panic
fetch_token_sends_bearer       wiremock token endpoint: Bearer creds → Authorization: Bearer header
fetch_token_sends_basic        wiremock: Basic creds → Authorization: Basic base64 (M1 behavior)
fetch_token_anonymous          no creds → NO Authorization header (assert absence)
```

**GREEN** — implement per Interfaces + constraint 4. `fetch_token` gains a `host` param; existing callers (digest 401 path, list_tags 401 path) pass the ref's host. `client_from_env` in lib.rs becomes `Creds::load_from(home)` (env read moves into `load_from`).

**Check:** `export CARGO_HOME="$PWD/.cargo-home" PATH="/nix/store/3f0kf75lwkxwi4ikm8fn95aqjmjb91vg-rust-1.95.0/bin:$PATH"; cargo test && cargo fmt --check`

**Commit:** `feat: per-host registry credentials (env + config.json, Basic and Bearer)`

## Task 2 — every host resolves

Files: `src/registry.rs` (base_for), `src/lib.rs` (resolve_image gate), `src/image_ref.rs` (is_docker_hub if now unused), `tests/e2e.rs`.

**RED** — first the flip + new e2e:

```
e2e ghcr_line_now_pinned       ci-e2e.yml's ghcr.io line: base_urls maps ghcr.io → wiremock
                               (anonymous 200 digest D_GHCR) → status pinned, value
                               image:ghcr.io/org/img:1.0@sha256:<D_GHCR>; exit 0
e2e_update_works_on_nonhub     --update on a non-hub host: wiremock tag list (mock host) →
                               same-major bump + digest pin (proves list_tags shares auth path)
lib unit: base_for_default     new RegistryClient (empty base_urls): base_for(ghcr image) ==
                               "https://ghcr.io/…"? base_for is private — test via a new
                               pub(crate) fn or keep it covered by the e2e (implementer's
                               choice, e2e coverage is sufficient)
```

**GREEN** — base_for default `https://{host}`; remove the `is_docker_hub()` skip in `resolve_image` (and the now-dead `no base URL` error); drop `is_docker_hub` if unused (update M1 docs comment).

HARD REQUIREMENT: every other test stays green byte-identical (constraint 3).

**Check:** same cargo test + fmt.

**Commit:** `feat: resolve images on every registry host (drop the Docker-Hub-only gate)`

## Task 3 — challenge-driven auth (Bearer per-host + Basic)

Files: `src/registry.rs`, new `tests/registry_hosts.rs`.

**RED** — wiremock suite (follow `tests/gitlab_client.rs` patterns; per-host mocks via `base_urls`):

```
ghcr_bearer_pat_flow           401 challenge (ghcr realm/service/scope) → token request carries
                               Authorization: Bearer <GITHUB_TOKEN> → 200 token → manifest 200
ecr_public_anon_flow           challenge realm "…/token/" (trailing slash) service public.ecr.aws
                               scope "aws" → token request has NO Authorization header → 200
gcp_identitytoken_flow         config.json identitytoken under host key → Bearer on token request
jfrog_basic_challenge          401 with `Basic realm="Artifactory Realm"` → manifest retry with
                               Basic creds from config → 200 digest
basic_challenge_no_creds       Basic challenge, for_host None → Unresolved("{host}: registry
                               requires basic auth — docker login {host}")
token_denied_no_creds          ghcr: token endpoint 401 → Unresolved containing "GITHUB_TOKEN"
                               generic host: message contains "docker login {host}"
no_challenge_401_unchanged     401 w/o WWW-Authenticate → existing error text (M1 test stays)
```

**GREEN** — 401 dispatch per Interfaces (both `digest` and `list_tags` share a helper); token-request 401/403 → `Unresolved("{host}: token denied — {hint}")` (ghcr hint per constraint 1, generic otherwise); Basic challenge → manifest retry with Basic from `for_host(host)`.

**Check:** same cargo test + fmt + `cargo clippy --all-targets -- -D warnings`.

**Commit:** `feat: challenge-driven registry auth — Basic challenge and per-host token errors`

## Task 4 — live tests + README + final gate

Files: `tests/live.rs`, `README.md`.

- `live_ecr_public_anonymous` (PIN_CI_LIVE gate): `public.ecr.aws/nginx/nginx:latest` → digest `sha256:<64 hex>` (re-run `crane digest` in the devenv to verify the constant, with a date comment — digests of `:latest` move; keep the assertion shape-+`crane`-checked, not a frozen value, UNLESS crane disagrees loudly).
- `live_ghcr` (PIN_CI_LIVE + GITHUB_TOKEN present; SKIP with hint otherwise): a real public package (implementer verifies with `curl` first, records the probe in a comment).
- README M4 section per design memo (hosts, env table, config.json, ECR private via docker login, SigV4 deferred, R1 semantics, self-hosted `--gitlab-host` unaffected).
- **Final gate (controller, before merge):** full suite with `PIN_CI_LIVE=1`, fmt, clippy -D warnings, release build, live run on the real `ci/.gitlab-ci.yml` copy in `.tmp/` (docker hub lines pin as before; ghcr line unresolved-with-hint is the EXPECTED new behavior — verify it exits 1 with the hint and pins everything else, i.e. partial-write semantics), merge to main (unsigned), ledger close-out.

**Commit:** `test: live ECR public + GHCR tests; docs: README M4 multi-registry section`

## Review Focus (inline reviews)

1. **docker.io untouched** — M1/M2 mock + e2e + live + oracle tests pass unchanged (constraint 3); the ONLY e2e diff is the ghcr flip.
2. **Precedence table** — env before config per host; M1 docker hub key order preserved; malformed config never fatal.
3. **Token request auth** — Bearer variant sends `Authorization: Bearer` (not basic) — the GHCR/GCP path; anonymous case sends NO header (ECR path); Basic case = M1 behavior.
4. **Basic challenge** — manifest retry, no token round-trip; no-creds → hint error (constraint 1).
5. **base_for** — override map wins; docker.io special-case; `https://{host}` default; dead error removed.
6. **Live test hygiene** — ECR `:latest` digest not blindly frozen (crane cross-check); GHCR gate skips cleanly without a token.

## Self-review (controller, post-write)

- Task boundaries: T1 = creds data + token-request auth (unit + 3 wiremock); T2 = gate/base (small, 2 e2e); T3 = challenge dispatch (7 wiremock); T4 = live + docs. Each one commit, one check. ✓
- Interface consistency: `fetch_token(realm, service, scope, host)` threading matches T1→T3 flow; `for_host` is the single creds lookup used by both T1 (token request) and T3 (Basic retry). ✓
- Spec M4a coverage: GHCR ✓ (T1+T3+T4), GCP ✓ (T1 identitytoken + T3), ECR public ✓ (T3+T4), ECR private via config ✓ (constraint 4 + README), JFrog Basic ✓ (T3), self-hosted ✓ (generic), Bearer+Basic ✓, R1 ✓ (T3), `--update` all hosts ✓ (T2 e2e). ✓
- Risk: T2's e2e flip touches a test from M1's Review-Focus list — deliberate, documented (constraint 6), reviewed in Review-Focus 1.
