# pin-toolkit — pin CI and Helm references to immutable ids

Status: approved by owner 2026-11-10; revised 2026-11-10 with four owner changes (O5–O8 in [§13](#13-owner-answers)):
no compatibility layer, parser-based YAML discovery (spike S2 done: adopt with caveats, §6.9), real use
(DF1) before Helm with a smaller config first, lazy feature-gated resolvers. M4b-1 merged (0e2047b); next: M5a–c plan.
Date: 2026-11-09. Supersedes the "Helm chart pinning — separate tool" note in
`2026-11-01-pin-ci-design.md`. That spec describes the `pin-ci` behaviour the restructure starts
from; pin-toolkit promises no compatibility with it (D33).
This document is self-contained. The research it is based on is not in git, so every fact that
matters is restated here, with external sources cited.

## 1. Intent and why

`pin-ci` (this repo) already pins GitLab CI references in place, byte-exact and idempotent:

- images and services: `repo:tag` → `repo:tag@sha256:<digest>`, on every docker-v2 registry
- components: `…@1.0.0` → `…@<sha40>  # 1.0.0`
- classic includes: `{project, ref}` → `ref: <sha40>  # <ref>`

Teams that pin CI also deploy with Helm. Helm has the same problem: chart versions are often
ranges, `values.yaml` images use mutable tags, and `helm upgrade oci://…` lines run inside CI
scripts. No existing tool covers both GitLab CI and Helm while also keeping the
human-readable version and editing bytes exactly. Prior art as of 2026-11-09:

| Tool | Gap |
|---|---|
| Renovate | A PR bot. It does not pin GitLab components or includes to a commit SHA. `helm-values` has `pinDigests: false` by default. Chart deps get versions only |
| ratchet | Its GitLab parser handles only job `image:`, and it re-marshals YAML (not byte-exact) |
| frizbee | Images and GitHub Actions only. It drops the tag from the ref (`repo@sha256 # tag`) |
| pinact | GitHub Actions only |

Sources: https://github.com/renovatebot/renovate, https://github.com/sethvargo/ratchet,
https://github.com/stacklok/frizbee, https://github.com/suzuki-shunsuke/pinact.
GitLab itself recommends pinning components to a commit SHA: https://docs.gitlab.com/ci/components/.

**pin-toolkit** turns this repo into one Rust workspace:

- a shared **core library** with resolvers, versioning, release age, byte-exact edits, the report and exit codes;
- **plugins split by file format**: `gitlab-ci` and `helm` now, `github-actions` later;
- one binary that runs every enabled plugin in **one pass**, with **one report** and **one exit code**.

`pin-ci` has never been used in the real world, so pin-toolkit replaces it outright: no legacy mode,
no frozen output (D33). What carries over is the product property: edits touch only the reference
bytes, and a second run makes no edits.

Each plugin also works as a standalone tool. The toolkit is open source on GitHub, its CI pushes
builds to Cachix, and a Home Manager module installs and configures it.

## 2. Goals, non-goals, anti-goals

**Goals**
- G1. Apply the pin rule (§3) to every target in §4 and report each reference: `pinned`, `ok`, `updated`, `skipped (reason)` or `unresolved (reason)`.
- G2. Keep pin-ci's capabilities (every kind, the pin rule, byte-exact idempotent edits) through the restructure. Output and CLI may change; every change shows up in a reviewed snapshot diff (§11). No compatibility layer (D33).
- G3. Run in one pass: one file read, one write per file, a resolver cache shared across plugins, one merged report, one exit code.
- G4. Each plugin works standalone (`pin-toolkit <plugin>`, `pin-<plugin>`) and as a Rust library.
- G5. Make extension easy: a new file format means a new plugin crate plus one registration line. A new artifact source means a new core resolver.
- G6. Distribute through a Nix flake (crane), Cachix, a Home Manager module, static release binaries and a GitLab-usable container image.

**Non-goals**
- PR or MR bots, scheduling, opening merge requests. This is a CLI that pins in place now.
- Dockerfile `FROM`, docker-compose, Kubernetes manifests, kustomize, Flux and Argo CD. Each is a possible future plugin.
- Writing `Chart.lock` or `helmfile.lock` (§4, H1/H5).
- Per-platform digests. We pin the manifest-list digest, as today.
- Runtime-loaded plugins (dylib/WASM) and third-party out-of-process plugins (§5.2).
- `unpin`, Windows binaries, publishing to crates.io (revisit after the API settles).
- Renovate interop beyond images. Renovate understands `repo:tag@sha256` images, but it does not extract `@<sha>  # ver` components or includes. For those, pin-toolkit `--update` (M4b-2) is the updater. Document this.

**Anti-goals** (each one is a failure)
- Re-serialising YAML. Edits are byte splices on the original text. Untouched bytes stay identical, and comments, anchors, quoting and indentation survive.
- Silently skipping anything, auth failures included. Every reference ends in a reported state. A host that demands credentials we lack is `unresolved` with a login hint and exit 1 (pin-ci M4a rule).
- Pinning to a mutable ref when an immutable one exists and the consumer accepts it. For example: never a branch name instead of a SHA, never a bare version where a digest is accepted.
- Writing secrets to files, logs or the Nix store. Sending a token to a host that only a repo config named (§7.3).
- Implicit version bumps. Only `--update` changes versions.
- Reordering, reformatting, or adding a second trailing comment to a line that already has one.

**Constraints**

| Constraint | Source | Consequence |
|---|---|---|
| Rust for anything beyond thin glue. Edition 2024, one cargo workspace, tokio without `spawn` | owner | only Nix/CI glue outside Rust; no scripts in the run path |
| Byte-exact, idempotent edits: byte splices on the original text, never parse-and-rewrite. Discovery may use a span-preserving parser (§6.9) | product property (hard invariant) | core `Splice`/`LineIndex`; a second run makes 0 edits; property tests (§11) |
| No compatibility layer: the tool has no real-world users. Kept as product semantics: partial writes, fail-loud auth, min-age default off, pin-only default | owner (2026-11-10) | no legacy mode; R1/R2 fixed in M5c; snapshot tests instead of byte-identical goldens |
| Resolvers are pin-core cargo features, each built lazily on first use; no extra crates | owner (2026-11-10) | `Ctx` is not a god object (§6.2) |
| GitLab-centric first, easy to extend later | owner | gitlab-ci milestones come before helm; github-actions is designed for, built last |
| Open source on GitHub; GitHub Actions builds releases and pushes to Cachix | owner | §8.3 |
| Installable and customisable via a Home Manager module: plugin enable = in the build, settings = runtime | owner | §8.2, O1 |
| Credentials only from env, docker/helm config files or a global host binding; never in repo files or the Nix store | pin-ci contract + §7.3 | `[hosts]` is global-only; holds from the first config layer (M5d) |
| Light dependencies: no Node, no runtime tools; static Linux binary | pin-ci contract | musl static build, container without extra tools |

## 3. The pin rule

Owner's words: *"always an immutable id (sha256 / commit sha) where possible; keep human-readable
tag/version inline (`foo:<tag>@sha256:...`) or as trailing comment; where only a version is
possible (e.g. Chart.yaml deps) pin exact version."*

Operational form, for a reference r in a file whose consumer is C (GitLab, Helm, helmfile, Actions):

1. If C accepts an immutable form of r, write it. Keep the human-readable version **inline** when the syntax allows it (`tag@sha256:…`, `oci://c:1.2.3@sha256:…`, `--version 1.2.3`). Otherwise put it in a **trailing comment** `␣␣# <original>`. Never put the comment inside a flow mapping or sequence: it goes after the closing bracket. Never put it after a trailing `\` line continuation.
2. Otherwise, if C only accepts a version, write the **exact version**. When the original was a range, keep it as a trailing comment (`version: 1.2.7  # ~1.2`).
3. Otherwise report `skipped (<reason>)`, or `unresolved (<reason>)` when resolution failed.

## 4. Targets

Resolvers: **OCI** = docker-v2/OCI registry (manifest digest, tag list, timestamps). **GL** =
GitLab API ref → commit. **IDX** = classic Helm repo `index.yaml`. **GH** = GitHub API ref → commit.

### gitlab-ci plugin (`.gitlab-ci.yml`, `**/*.gitlab-ci.yml`, `.gitlab/ci/**/*.yml`)

| # | Kind | Location | Resolver | Output (human-readable kept) | Impossible / skipped | Milestone |
|---|---|---|---|---|---|---|
| C1 | `image` | top-level, `default:` and job `image:` (string or `name:`) | OCI | `alpine:3.20@sha256:<d>` (inline tag). Tagless `alpine` → `alpine@sha256:<d>` | value that does not parse as an image ref (e.g. `$VAR`) → `skipped (unparseable reference)` | exists (M1–M4b-1) |
| C2 | `service` | `services:` items (string or `name:`) | OCI | same as C1 | same | exists. Reported as kind `service` since M5d (before: `image`) |
| C3 | `component` | top-level `component:` line | GL | `gitlab.com/g/p/name@<sha40>  # 1.0.0` | catalog-managed (`~latest`, `1.0.x`, partial `1`/`1.0`) → skipped. Host ≠ configured host → skipped (as built, M5d: host ∉ {primary} ∪ `gitlab-hosts`) | exists |
| C3b | `component` | `include:` items `- component: <host>/<path>@<ver>` (**GitLab's documented form**; pin-ci does **not** discover it today) | GL | same as C3. `$CI_SERVER_FQDN`/`$CI_SERVER_HOST` host maps to the configured GitLab host, and the variable text is kept | as C3 | G1 |
| C4 | `include` | `include:` `{project, ref[, file]}`, block or flow, any key order | GL | `ref: <sha40>  # v1.2.3`. Flow: `{ …, ref: <sha40> }  # main` | bare `{project}` (default branch): no target. `$` ref → skipped. `local:`/`template:` have no external id | exists |
| C5 | `helm-oci` | `script`/`before_script`/`after_script` items: `helm install\|upgrade\|template\|pull\|show … oci://h/p/c` | OCI (chart manifest) | `oci://h/p/c@sha256:<d> --version 1.2.3` or `oci://h/p/c:1.2.3@sha256:<d>`. A range `--version ~1.2` is resolved to an exact version and rewritten. No comment needed: the version stays in `--version`/`:tag` | needs Helm ≥ 3.17 to read the digest. `$VAR` → skipped (dynamic). `oci://c` with no version → `oci://c:<highest stable>@sha256:<d>`, the same way tagless images are handled (my call). `helm push`, `docker run … helm` → no target | M7 |
| C6 | `helm-repo` | `helm … repo/chart --version X`, `chart --repo URL --version X` | IDX | exact `--version 1.2.7` | no digest form exists for classic repos | later (M7+) |
| — | — | `include: remote:` | — | — | candidate: GitLab `include:integrity` (SRI sha256). **Verify** before adding a kind | not planned |

Facts: Helm accepts digest refs `oci://…@sha256:`, `…:tag@sha256:` and `@sha256: --version v`
(cross-validated; mismatch errors) since 3.17. Sources: helm/helm PR #12690 and
https://helm.sh/docs/topics/registries/#installing-charts-with-digest. OCI chart tags map `+` → `_`
(helm/helm#10166). Chart manifests are served only for an OCI `Accept`. A docker-v2-only `Accept`
gets 404 (verified live on ghcr.io argo-cd 5.43.0). The digest to pin is the manifest digest, the
same value `helm pull` prints.

### helm plugin (`**/Chart.yaml`, `**/requirements.yaml`, `**/values.yaml`, `**/values[-.]*.yaml`, `**/helmfile.yaml{,.gotmpl}`, `**/helmfile.d/**/*.yaml{,.gotmpl}`; `.yml` variants)

| # | Kind | Location | Resolver | Output | Impossible / skipped | Milestone |
|---|---|---|---|---|---|---|
| H1 | `chart-dependency` | `Chart.yaml` (apiVersion v2) `dependencies[].version`, `requirements.yaml` (v1) | IDX for `https://` repos, OCI tag list for `oci://` (chart = `<repo>/<name>`) | `version: 1.2.7  # ~1.2`. An exact version that was written exactly stays unchanged (`ok`) | **no digest field** in Chart.yaml deps (feature request helm/helm#32133; verify its state). Editing makes `Chart.lock` stale: its `digest` is an aggregate hash over requirements, not per chart. Report follow-up `helm dependency update <dir>`. `file://`, empty repository, `git+` → skipped. `@alias`/`alias:` without a mapping → `skipped (unknown alias)`. Unparseable or empty version → skipped | M8b |
| H2 | `values-image` | key matching `/image$/i` with a string value `repo:tag[@sha256:…]` (any depth, any document) | OCI | `image: "repo:1.2@sha256:<d>"`, same as C1 | value overlaps a masked `{{ }}` (§6.9) → `skipped (templated)` | M8a |
| H3 | `values-image` | map `image: {registry?, repository, tag\|version, digest?}` | OCI | an existing `digest:` key is filled (`digest: sha256:<d>`) and the tag kept. **No `digest:` key → `tag: "1.2.3@sha256:<d>"` by default** (O2, owner); opt-out `plugins.helm.values.tag-digest = false` → `skipped (no digest key)` | empty `tag` (chart uses `appVersion`) → skipped. `sha:`/`imageDigest` variants → skipped (later). Whether the chart's templates honour `digest` cannot be known from values alone | M8a |
| H4 | `helmfile-release` | `releases[]`/`templates{}` `{chart: repo/name\|oci://…, version}` + `repositories[]` (`oci: true` ⇒ OCI) | IDX / OCI | `version: 1.2.7  # ~1.2`. OCI opt-in (`helmfile.oci-digest`): `version: 1.2.7@sha256:<d>` | local charts, `{{ }}` values → skipped. Writing `helmfile.lock` → no, report follow-up `helmfile deps`. OCI digest needs a helmfile release that contains PR #2398 (merged 2026-02-12) | M8c |
| H5 | — | `Chart.lock`, `helmfile.lock`, index `digest` of a `.tgz` | — | never written | Helm reads no chart-content digest from Chart.yaml or helmfile | — |

Helm facts: dependency `version` is a Masterminds semver constraint (`~ ^ x * >= < - ||`). The
highest stable version that matches wins, and prereleases are excluded unless the constraint names
one. A YAML-unquoted `1.2` must be read as raw text, never as a float. `index.yaml` entries carry
`created` (RFC3339), which serves as the release time. It is ignored when missing or zero, and it is
unreliable when the index was regenerated without `--merge`. Helm-pushed OCI charts carry the
`org.opencontainers.image.created` manifest annotation. Sources: https://helm.sh/docs/topics/charts/,
helm/helm `internal/resolver/resolver.go`, `pkg/repo/v1/index.go`; Renovate `helm-values` and `helmfile` managers.

### github-actions plugin (future; `.github/workflows/*.y{a,}ml`, `action.y{a,}ml`)

| # | Kind | Location | Resolver | Output | Skipped | Milestone |
|---|---|---|---|---|---|---|
| A1 | `action` | `uses: o/r[/path]@v4` (steps, reusable workflows) | GH | `uses: o/r@<sha40>  # v4` | `./local`, `$`-expressions | M9 |
| A2 | `docker-action` | `uses: docker://alpine:3.20` | OCI | `docker://alpine:3.20@sha256:<d>` | — | M9 |
| A3 | `container` | `container:`/`services.*.image` | OCI | same as C1 | `${{ }}` | M9 |

## 5. Approaches

### 5.1 Plugin split

| | A. **by file format** (chosen by owner) | B. by artifact kind | C. one tool per file × artifact |
|---|---|---|---|
| Plugins | gitlab-ci, helm, github-actions | images, charts, git-refs | gitlab-images, gitlab-components, helm-values, … |
| Scanners | one per grammar (N) | every plugin parses every format (N×M) | N×M |
| Same file edited by | exactly one plugin | several, so splice conflicts | several processes, so racing writes |
| User toggle maps to | ecosystem ("no helm here") | artifact type | both, fine-grained |
| Shared resolver cache | in-process | in-process | none |
| Prior art | ratchet parsers, Renovate managers | — | Renovate #21446 (two managers for one file confuse users) |

**Why A:** the owner chose it. It is also the only option where a file has exactly one owner, so
byte-exact splices never conflict. Any artifact kind that shows up in several formats (OCI images
are in all three) is shared through core helpers (`oci::pin_inline_image`), not through plugins.

### 5.2 Plugin mechanism

| Option | Standalone | HM toggle | One pass + shared cache | Cost | Verdict |
|---|---|---|---|---|---|
| (1) one binary, all plugins compiled in, toggled by config | subcommand + argv[0] alias | config (Cachix hit) | ✓ | lowest | **base** |
| (2) (1) + one cargo feature per plugin (default = all) | + trimmed builds | config, or "not installed" via `.override` | ✓ | per-feature CI build | **add on top of (1)** (my call) |
| (3) separate `pin-toolkit-<p>` binaries + dispatcher (cargo/git style, JSON protocol) | ✓✓ | `home.packages` | ✗: subprocesses, no shared cache, one tag can yield two digests, versioned protocol | high | reject. The JSON report stays a possible external-plugin contract later |
| (4) dylib / WASM components | ✗ | plugin files | ◐ | no stable Rust ABI; WASI networking/credentials; wasmtime adds MBs | reject. `scan` is pure text → targets, so a WASM adapter remains possible later |

Size note: almost all binary bytes come from tokio, reqwest, rustls and serde, which every plugin
shares. Trimming saves only KBs.

## 6. Architecture

### 6.1 Workspace

```
Cargo.toml                 [workspace] members = ["crates/*"], resolver = "3"; [workspace.package] version, edition = "2024"
crates/pin-core/           lib: oci/{image_ref, registry [oci]}, gitlab [gitlab], helmrepo [helm-repo] (M8b), github [github] (M9),
                           ver/{docker,semver,age}, git (is_commit_sha), text (LineIndex), edit, yaml (§6.9), report, plugin, run, config,
                           cli (clap Args, feature "cli").  [x] = resolver cargo feature (D36); default features = none
crates/pin-gitlab-ci/      lib: discover (on core::yaml from M5c-y), policy, `GitlabCi: Plugin`; pin-core features oci + gitlab
crates/pin-helm/           lib (M8): oci + helm-repo        crates/pin-github-actions/   lib (M9): oci + github
crates/pin-toolkit/        bin `pin-toolkit`; features gitlab-ci, helm, github-actions (default = all); argv[0] dispatch
pin-ci-images.sh extract-docker-pins.py config.js   repo root, unmodified (oracle, kept while useful)
flake.nix nix/             M6
```

**As built (M5d):** `config` and `cli` live in `pin-toolkit`, not in pin-core (T-D2): the repo schema
validates `[plugins.<id>]` for known but unbuilt plugins from pin-toolkit's static `KNOWN` list
(`known.rs`), and the core must not know plugin ids (D10). pin-toolkit = lib (`execute(Process, tune)
-> Output`, `main_with`; `cli`, `config/{mod,repo,env}`, `known`, `enable`, `walk`) plus a thin
`[[bin]] pin-toolkit`; the process env is read only in `Process::from_os()`. Features:
`default = ["gitlab-ci"]` (the only plugin crate so far). pin-core has no `config`/`cli` modules; it
gained the feature `test-support` (dev-dependencies only, D38).

### 6.2 Core interface (one design)

The plugin owns *where* targets are (a pure scan) and *policy*: how a resolver decision becomes
splices, catalog-managed skips, host gates, the values digest-key heuristic. The core owns
resolvers, the cache, edits, file I/O, the report and exit codes. There is no core `Reference`
or `PinStyle` enum. Format-specific policy would turn it into a grab-bag, and the resolvers are
shared through `Ctx` anyway.

```rust
// pin-core::text / edit — byte-exact edits
pub struct LineIndex { /* line starts via split_inclusive('\n'); line_end() excludes "\r\n" (fixes R2) */ }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice { pub start: usize, pub end: usize, pub text: String }   // start == end ⇒ insertion
pub fn apply(text: &str, s: &[Splice]) -> String;  // end-first; equal-start insertions keep source order
pub fn coalesce_comments(s: Vec<Splice>) -> Vec<Splice>; // same-offset "  # a" + "  # b" → "  # a, b" (fixes R1)
pub fn requote(old_token: &str, new_value: &str) -> String;                 // today's splice_replace
pub fn trailing_comment(text: &str, line: usize, token_end: usize, orig: &str) -> Option<Splice>; // None if '#' already after token
// later: existing_comment(..) (M4b-2/M9), insert_line_after(..) (only if a digest-key insertion mode is ever added)

// pin-core::plugin
pub struct KindSpec { pub id: &'static str, pub default_on: bool, pub help: &'static str }
pub struct Finding {                       // one reference, resolved
    pub line: usize,                       // 0-based; sort key
    pub kind: &'static str,
    pub reference: String, pub old: String, pub new: Option<String>,
    pub status: Status, pub reason: Option<String>,
    pub edits: Vec<Splice>,                // empty unless pinned/updated
    pub follow_up: Option<String>,         // e.g. "helm dependency update charts/app"
}
pub struct ScanError { pub line: Option<usize> /* 1-based */, pub msg: String }  // the file does not parse (§6.9)
pub trait PluginMeta {                     // shared by Plugin and DynPlugin: one method set, no ambiguity
    fn id(&self) -> &'static str;                         // "gitlab-ci": config key, subcommand, report field
    fn kinds(&self) -> &'static [KindSpec];
    fn default_files(&self) -> &'static [&'static str];   // globs
}
pub trait Plugin: PluginMeta {             // no Send bound: futures are joined, never spawned (as today)
    type Target;
    fn scan(&self, path: &Path, text: &str, on: &KindSet) -> Result<Vec<Self::Target>, ScanError>; // PURE; sorted (line, start); Err = no partial targets
    async fn resolve(&self, ctx: &Ctx, text: &str, t: &Self::Target) -> Finding;
}
pub trait DynPlugin: PluginMeta { /* plan(ctx, path, text, on, concurrency) -> LocalBoxFuture<Result<Vec<Finding>, ScanError>> */ }
impl<P: Plugin> DynPlugin for P { /* scan (Err → returned, nothing resolved) → bounded join of resolve, as today's join_all + semaphore */ }

// pin-core::run
pub struct UpdatePolicy { pub enabled: bool, pub allow_major: bool, pub min_age_days: u32, pub unknown_age: AgePolicy }
pub struct Ctx {                           // small: policy + memo + lazily built resolvers (D36)
    pub update: UpdatePolicy, pub now_unix: i64,
    env: ResolverEnv,                      // tokens, host bindings, CA files: read once from env (+ global [hosts] from M5e')
    memo: Memo,
    #[cfg(feature = "oci")]       oci: OnceCell<Result<RegistryClient, String>>,
    #[cfg(feature = "gitlab")]    gitlab: OnceCell<Result<GitLabHosts, String>>,
    #[cfg(feature = "helm-repo")] helm_repos: OnceCell<Result<HelmRepoClient, String>>,   // M8b
    #[cfg(feature = "github")]    github: OnceCell<Result<GitHubClient, String>>,        // M9
}
impl Ctx {                                 // resolve entry points (D38): the only resolver access for plugins and core policy
    #[cfg(feature = "oci")] pub async fn oci_digest(&self, r: &ImageRef) -> Result<String, RegError>;
    #[cfg(feature = "oci")] pub async fn oci_tags(&self, r: &ImageRef) -> Result<Vec<String>, RegError>;
    #[cfg(feature = "oci")] pub async fn oci_tag_times(&self, r: &ImageRef) -> Result<BTreeMap<String, i64>, RegError>;
    #[cfg(feature = "gitlab")] pub async fn gitlab_resolve(&self, host: Option<&str>, project: &str, git_ref: &str) -> Result<String, String>; // None = primary
    /* helm-repo (M8b) and github (M9) entry points alike. The lazily built clients (oci(), gitlab(), …) and GitLabHosts are crate-private */
}
pub struct RunOptions { pub dry_run: bool, pub check: bool, pub concurrency: usize }
pub enum FileError { Read { path: String, msg: String }, Parse { path: String, line: Option<usize>, msg: String }, Write { path: String, msg: String } }
pub async fn pin_file(p: &dyn DynPlugin, ctx: &Ctx, path: &Path, on: &KindSet, o: &RunOptions) -> Result<Vec<Row>, FileError>;
pub async fn run(routes: &[Route], ctx: &Ctx, o: &RunOptions) -> Report;

// pin-core::oci [feature oci] — today's resolve_image policy, minus the tokens
pub enum ImageDecision { Current, Pin { digest: String }, Update { tag: String, digest: String }, Skip(String) }
pub async fn decide_image(ctx: &Ctx, r: &ImageRef) -> Result<ImageDecision, String>; // Err = today's exact unresolved text
pub async fn pin_inline_image(ctx: &Ctx, text: &str, line: usize, start: usize, end: usize, value: &str, kind: &'static str) -> Finding;
// pin-core::gitlab: GitLabClient::resolve(project, ref) -> Result<Sha, GitLabError>; tags() (M4b-2)
// pin-core::git (no feature): is_commit_sha() — shared by gitlab (C3/C4) and github (M9)
// pin-core::ver: docker::{parse, compare, pick_latest_stable}; age::{parse_rfc3339_unix, is_old_enough}; semver::{Version, Constraint} (M8b)
// pin-core::report
#[derive(Serialize)] #[serde(rename_all = "lowercase")] pub enum Status { Pinned, Ok, Updated, Skipped, Unresolved }
pub struct Row { plugin, kind, file, line /*1-based*/, reference, old, new, status, reason }
```

Rules:
- **One file = one plugin.** An explicit subcommand routes the file. Otherwise the file must match exactly one enabled plugin's globs. Zero or several matches on an explicit path is exit 2 with a hint. On a directory walk, files nobody claims are ignored.
- Within one file, the core rejects overlapping splices: both findings become `unresolved (internal: overlapping edits)`. Same-offset comment insertions coalesce into one comment (R1 fix).
- Partial writes stay as today: an `unresolved` row does not block the other splices in the same file.
- A `ScanError` makes `pin_file` return `FileError::Parse`: no rows, nothing resolved or written. The run records it under `plugins.<id>.errors`, exit 2 (§6.5, §6.9; from M5d).
- **Plugins resolve only through the `Ctx` entry points (D38)**, and so does core policy such as `decide_image`. The memo (§6.7) and the host/token binding (D31, D37) live behind them, in one place. The plugin still checks its host gate before calling.
- Error texts carry over as the starting point: digest errors report the inner message, tag-list errors read `tag list: {e}`, GitLab errors use `e.to_string()`. Any change shows in a snapshot diff.
- **Lazy, feature-gated resolvers (D36).** Each resolver is a pin-core cargo feature; its HTTP deps are optional behind it. A client is built on first use (`std::cell::OnceCell` suffices: futures are joined, never spawned), so a run without helm targets never builds `HelmRepoClient`. A construction error (e.g. unreadable CA file) is cached and makes every reference that needs it `unresolved (<error>)`, exit 1. Config errors stay eager (exit 2). Plugin crates enable only the features they need; no extra crates.

**As built (M5d):**
- `Plugin::resolve(&self, ctx, src: &Source<'_>, t)`. `Source { path, text, lines: LineIndex }` is built
  once per file in `DynPlugin::plan` (review L3); `trailing_comment(text, lines, line, token_end, orig)`.
- `Selection { kinds: KindSet, ignore: RefPatterns }` replaces `on: &KindSet` in `plan` and `pin_file`.
  `Plugin::ignore_key` (default `None`) gives a target's key: gitlab-ci uses the value with quotes
  stripped, and `{project}@{ref}` for includes (T-D17). `*` (any run, incl. `/` and empty) and `?`
  match the whole key. Ignored targets have no row and are never resolved.
- `run::{Route { plugin, sel, path, display }, run(routes, ctx, RunOptions) -> Report}`: sequential over
  routes; a `FileError` becomes `ErrorEntry { file, line, message }` under its plugin; never stops early.
  `pin_file` writes only the edits of `pinned`/`updated` findings (review L2).
- One file = one plugin, as built (T-D10): an explicit file with a subcommand or alias is claimed
  regardless of globs; in all-plugins mode an explicit path claimed by 0 or 2+ enabled plugins is fatal
  before the run (exit 2). A **walked** file claimed by 2+ plugins is a per-file `ErrorEntry` under the
  first claimant (compiled order): not routed, the other files run, exit 2 (PR-L11).
- D38: the raw `RegistryClient`, `Creds`, `GitLabClient`, `GitLabHosts`, `Ctx::with_oci/with_gitlab` and
  `ResolverEnv::from_process` are `#[doc(hidden)] pub` only under the pin-core feature `test-support`,
  which only dev-dependencies enable; `scripts/gate.sh` checks it (`cargo check -p pin-gitlab-ci -p
  pin-toolkit`; `cargo tree -e normal,build` must not show `test-support`).
- Kind ids are constants (`pin_gitlab_ci::{IMAGE, SERVICE, COMPONENT, INCLUDE}`, review L10);
  `PinTarget::Image` carries its `kind` (`service` under `services:`, else `image`).

### 6.3 One-pass run

1. Load the config (§7) and build `Ctx`: env and host bindings, an empty memo, `now`. No resolver client is built yet (§6.2).
2. Inputs: explicit files and directories, or the git root (falling back to cwd). Walk with `.gitignore` respected (the `ignore` crate), match each enabled plugin's `files` globs, sort paths.
3. Per file, sequentially as today: read once. Run `plan` (scan → resolve, with at most `--concurrency` in flight). Stable-sort by line. Check for overlaps. Unless `--dry-run`/`--check`, apply the edits and write only if there is at least one splice.
4. Merge rows, per-plugin summaries, follow-ups and per-file errors into one `Report`. Render it and exit (§6.5).

A file that fails to read, parse (§6.9) or write is recorded under `plugins.<id>.errors`. The other files and plugins continue, and the exit code is 2.

**As built (M5d, T-D23):** the git root is the first ancestor of the cwd (incl.) with a `.git` entry,
else the cwd. `ignore::WalkBuilder` with `hidden(false)` (keeps `.gitlab-ci.yml`, `.gitlab/`) and `.git`
filtered; ignore files at the crate defaults (`.ignore`; inside a git repository `.gitignore`,
`.git/info/exclude`, the global excludes), so results can differ per machine. Only regular files;
symlinks are not followed in a walk, an explicit symlink is used as given. Globs (globset,
`literal_separator(true)`) match the path relative to the git root; for an explicit directory outside
the git root, relative to that directory's own git root, else to the directory itself (review-2 M1,
main). Paths are normalised lexically (`.`/`..`, no file-system access) and deduplicated by the
normalised absolute path: the first spelling is shown, and the input stays explicit if any spelling
was (review-2 M2). Display: relative to the cwd for walked files (`../.gitlab-ci.yml`), as passed for
explicit ones; routes are sorted by display. A walk error (e.g. an unreadable directory) is a per-file
`ErrorEntry` under the first enabled plugin (exit 2); a bad line in an ignore file is a stderr warning
`pin-toolkit: warning: {file}:{line}: {message}`, exit unaffected (review-2 L3, main). Files run
sequentially (T-D20); the memo (§6.7) makes repeated refs cheap.

### 6.4 Merged report

Toolkit JSON keeps pin-ci's `{results[], summary}` shape and status strings and adds fields. That is
continuity of design, not a compatibility promise (D33); the schema is stable from the first release:

```json
{ "schema_version": 1,
  "results": [ { "plugin": "helm", "kind": "chart-dependency", "file": "charts/app/Chart.yaml", "line": 12,
                 "reference": "postgresql ~12.1.0", "old": "~12.1.0", "new": "12.1.9", "status": "pinned" } ],
  "summary": { "pinned": 1, "ok": 0, "updated": 0, "skipped": 0, "unresolved": 0 },
  "plugins": { "helm": { "summary": { "pinned": 1, "ok": 0, "updated": 0, "skipped": 0, "unresolved": 0 }, "errors": [] } },
  "follow_ups": [ "helm dependency update charts/app" ] }
```

Human output: the pin-ci line format (`file:line  status  old -> new  # reference`), with
`plugin:kind` added after `file:line`, then the pin-ci summary line. When more than one plugin ran,
one summary line per plugin follows, then `follow-up: …` lines. A disabled kind produces **no row**:
disabling it was an explicit choice, and `config show` (M5e') reports it.

**As built (M5d, T-D21, T-D22):**
- Human: `{file}:{line}  {plugin}:{kind}  {status}[ ({reason})]  {old} -> {new|-}  # {reference}`, then the
  unchanged total line `pinned: …, ok: …, updated: …, skipped: …, unresolved: …`, then one `{plugin}: pinned: …`
  line per plugin when more than one plugin **ran** (`Report.ran`: plugins with ≥ 1 routed file or row;
  a plugin that only carries conflict or walk errors does not count, review-2 L7), then
  `follow-up: {text}` lines (dedup, first seen). Per-file errors go to **stderr** in both modes as
  `pin-toolkit: {file}[:{line}]: {message}`.
- JSON: key order `schema_version, results, summary, plugins, follow_ups`; row keys
  `plugin, kind, file, line, reference, old, new, status[, reason]`; `plugins.<id> = {summary, errors}`;
  `errors[i] = {file, line?, message}`. The `plugins` keyset equals the **enabled** plugins: the toolkit
  seeds it from the enablement plan, so an empty repo gives each enabled plugin a zero summary and
  `errors: []` (main, review-2 L7). Each plugin's errors are sorted by `(file, line)` regardless of
  origin (read/parse/write, walk, conflict; main); stderr uses the same order.
- pin-core: `Report::assemble(rows, ran, errors)`, `Report::exit_code(check)`, `render_human`, `to_json`.
- Fatal errors (exit 2 before the run): stderr `pin-toolkit: {msg}`, **stdout empty, also with `--json`**.

### 6.5 Exit codes

| Code | `pin-toolkit` (and every alias) |
|---|---|
| 0 | no `unresolved`, no error. With `--check`: also nothing would change |
| 1 | any `unresolved`. With `--check`: also any row that would be `pinned`/`updated` |
| 2 | fatal: usage or config error, a global-only key in the repo config, IO or YAML parse error in a file (§6.9), a plugin that is enabled but not compiled in, an explicit path claimed by 0 or 2+ plugins |

`--check` = `--dry-run` plus the exit-1 rule above. Its intended use is a CI gate. From M4b-2 it
also verifies that a `# <ref>` comment still resolves to the pinned SHA, like pinact's verify.

**As built (M5d):** exit 2 is either fatal before the run (usage, config, an explicit path claimed by
0 or 2+ plugins, a requested plugin that is not in the build; stdout empty) or per-file (read, parse
or write error, walk error, walked file claimed by 2+ plugins; the report is printed). `Report::exit_code`
implements the table; `crates/pin-toolkit/tests/exit_codes.rs` pins it and recomputes it from the JSON.

### 6.6 Front-ends

- `pin-toolkit [OPTIONS] [PATHS]…` runs all enabled plugins. `pin-toolkit <plugin> …` runs one plugin, as does the argv[0] alias `pin-<plugin>`. Also `pin-toolkit plugins` (lists compiled-in plugins, kinds, effective enable state) and, from M5e', `pin-toolkit config show [--origin]`.
- Options: `--dry-run --check --json --update --update-major --min-age-days N --concurrency N --only/--skip <p,…> --kind <p>:<k>=on|off --config FILE --no-config --gitlab-host H`.
- **No `pin-ci` legacy mode** (D33). M5d deletes the crate's `[[bin]] pin-ci`. `pin-ci` may remain only as a plain argv[0] alias of `pin-toolkit gitlab-ci` (toolkit flags, config, output and exit codes) if it costs no more than one dispatch entry and one symlink; otherwise it is dropped. The README says which.

**As built (M5d):**
- `pin-ci` is **kept**: one entry `ALIASES = [("pin-ci", "gitlab-ci")]`; `pin-<id>` for every id in `KNOWN`
  dispatches too (basename, `.exe` stripped). The symlinks ship with the nix package (M6). `[[bin]] pin-ci`
  is deleted (7ec517a); its driver lives on as the test helper `crates/pin-gitlab-ci/tests/common/driver.rs`
  for the e2e, live and oracle tests (T-D25).
- clap derive; every option is `global = true`. A subcommand is recognised only before the first path
  (`pin-toolkit a.yml gitlab-ci` treats `gitlab-ci` as a path). clap's implicit `help` subcommand is kept,
  so a directory named `help` must be passed as `./help` (main). Usage and help always say `pin-toolkit`,
  also after an alias (review-2 L4).
- CLI `--only`/`--skip` with a subcommand or alias → exit 2 (`--only/--skip cannot be combined with a plugin
  subcommand`, T-D28). An unknown id in `--only` or `--skip` → exit 2. CLI and env lists are trimmed,
  empty items are dropped, and an empty list counts as unset (review-2 L1, N3).
- `--kind <p>:<k>=on|off` is validated against `KNOWN`; a known kind that is not compiled (`helm-oci`) is
  accepted and ignored. Presence-only flags, no `--no-*` twins (T-D8).
- `plugins` prints `{id}: enabled|disabled|not in this build`, then per compiled kind
  `  {kind}: on|off — {help}` (T-D27). `config show` comes in M5e'.

### 6.7 Resolver cache

`Ctx.memo` is in-memory and lasts for one run. It is keyed by `(host, repo, tag)` for digests,
`(host, repo)` for tag lists and timestamps, `(host, project, ref)` for GitLab, the URL for helm
indexes, and `(owner/repo, ref)` for GitHub. Results **including errors** are cached, so one ref
cannot get two outcomes in a run. Concurrent identical lookups are coalesced. There is **no disk
cache**: freshness is the product. Revisit only if the Hub API limit (180 req/h per IP, used by the
M4b-1 age guard) bites. Existing per-client token caches stay.

**As built (M5d, T-D15):** `Memo<K, V> = RefCell<HashMap<K, Rc<tokio::sync::OnceCell<V>>>>` per entry
point, keyed by `(host, api_repo, effective_tag)` for digests, `(host, api_repo)` for tag lists and tag
times, `(host, project, ref)` for GitLab. Errors are cached (`RegError: Clone`, review L4); concurrent
identical lookups are coalesced by `OnceCell::get_or_init`. The registry client's repo-keyed tag-list
cache was removed (review-1 M1), so the memo is the one result cache; bearer tokens are cached per
`(host, realm, service, scope)` (review-1 H1).

### 6.8 Release age

`--min-age-days` (M4b-1, docker.io only via the Hub API) generalises to every resolver that has
timestamps: Hub API tags, `index.yaml` `created`, the OCI `org.opencontainers.image.created`
annotation, GitLab tag dates (M4b-2), GitHub (M9). When the age is unknown, `release_age.unknown =
"allow"` is the default. That matches today's documented no-op for non-docker.io hosts, and
`"block"` is the alternative. The default stays 0 (off) and 14 is documented as the recommended value.

**As built (M5d, T-D18):** `AgePolicy { Allow (default), Block }` in `UpdatePolicy::unknown_age`; the guard
is active iff `min_age_days > 0 ∧ (docker.io ∨ Block)`. Outside docker.io no timestamps are fetched, so
`Block` skips a reference that has an update candidate with `skipped (release age unknown ({host};
release_age.unknown = block))`; without a candidate it is pinned as usual. Set by `[release_age] unknown`
or `PIN_TOOLKIT_RELEASE_AGE_UNKNOWN` (`allow`/`block`, case-insensitive).

### 6.9 YAML discovery (D32)

Discovery (finding targets and their byte spans) uses the span-preserving event parser
`saphyr-parser` 0.3.x (MIT OR Apache-2.0, MSRV 1.85, +2 crates: saphyr-parser, arraydeque). Edits
stay byte splices on those spans (§6.2); the parser never writes. Spike S2 (2026-11-10) reproduced
all 37 `discover()` targets of `tests/fixtures/*.yml` and `ci/.gitlab-ci.yml` at identical bytes,
given the fix-ups below. Rejected: `yaml-rust2`, `marked-yaml` (start marker only, no end).

`core::yaml` = parser + a hand-rolled fix-up layer (built in M5c-y, pin-core feature `yaml`; `saphyr-parser`
is optional, so pin-core without the feature has no parser dependency):

| Parser fact (S2, saphyr-parser 0.3.3) | `core::yaml` handling |
|---|---|
| `Marker::index()` counts **chars**; a BOM counts as char 0; `col()` on line 1 excludes the BOM; `ScanError` says "byte N" but N is a char index | one char→byte table per file (`char_indices` + `len`); use `index()` only, never `col()` |
| quoted-scalar span end overshoots into trailing spaces and `# comment` (in flow also a space before `]`/`,`). Upstream bug in `scan_flow_scalar`; the start is correct | re-scan from the start to the closing quote (skip `''` and `\x` escapes) |
| scalar spans include node properties `&anchor `, `!tag ` | skip the properties, only when the event carries an anchor or a tag (block content may itself start with `!` or `&`). The skip also crosses line breaks and `#` comments between the property and the token, and every skip stops at the parser's span end |
| no comments in events; anchor names absent (numeric ids); merge keys `<<:` not expanded | trailing-comment detection stays hand-rolled (`edit::trailing_comment`). Aliases (`Event::Alias`), merge keys and `!reference […]` are **not targets**: the bytes live at the definition |
| after the first `ScanError` the iterator repeats it (sticky); unquoted `{{ … }}` mis-parses as flow mappings **without** an error | templated files (helm values, `.gotmpl`): with `ScanOpts { mask_templates: true }` every `{{…}}` (non-greedy, may span lines; an unclosed `{{` is left alone) becomes spaces, **keeping `\n` and `\r`**, before parsing: byte offsets and line numbers (also in error lines) stay exact. `ScalarRef.templated` is set when a non-empty span overlaps a masked range, **or when a plain scalar touches one across same-line blanks** (`image: nginx:1.25-{{ .v }}`, `{{ .r }}/nginx:1.25`, `a: x {{ y }}`; the masked bytes are blanks, so the parser's token ends or starts at the template, while Go renders one scalar; a flow separator, a comment or a line break ends the touch; review M1); the consumer decides (helm: `skipped (templated)`, M8a). An unquoted `key: {{ x }}` parses without error as an empty value that is **not** flagged: M8a must report it. gitlab-ci does not mask (GitLab CI is not Go-templated) |
| block scalars (`\|`, `>`) span the content only, header excluded; with properties the span starts at the properties instead. Either way it runs past the content to after the trailing break and the next line's indentation | the span is the content: with properties the header line and the blanks after it are skipped so it starts at the first content byte (without properties it is never skipped); the end is trimmed to the last non-blank byte, so a block span never reaches into the next line. Never spliced: `skipped (block scalar)` at a target path |
| a plain scalar with an empty value (`key:`, flow `{k}`): the parser's span is the **next** token (`}`, `, `) | an implicit null is an empty `Plain` scalar with `value == ""` and an **empty span** `s..s` at the start of the parser span: block → the key's `:` (on the key's line); flow → the start of the next token (`}` or `,`); with properties → the start of the properties. Consumers read an empty span as "no value". Without it `{project: p, ref}` would give `ref` the span `}` and a pin would replace the bracket |
| a node with a tag: `!reference [.base, image]`, `!custom x`, `!!str v` | non-core tag (`Tag::is_yaml_core_schema()` false; saphyr expands `!!` to the handle `tag:yaml.org,2002:`) → one `Node::Tagged`, its subtree is not emitted. Core tags (`!!str`, `!!seq`) are transparent: the node is a normal scalar/collection |
| `:` + TAB + a letter or `-` (`FOO:\tbar`, `image:\talpine:3.20`) is a `ScanError` ("':' must be followed by a valid YAML whitespace"), although YAML (a tab is `s-separate-in-line`) and GitLab (libyaml) accept it; the legacy scanner accepted it too. With `: \tx` the parser is fine (review H1; report upstream) | on exactly that error, the tab run just before the error marker is blanked (tab → space, 1 byte for 1 byte: every offset and char index is unchanged, no value changes) in a scan copy and the parse is retried; at most once per flagged tab, and within a budget of 64 MiB of re-parsed text (`RETRY_BUDGET`; each retry re-parses up to the next flagged tab), after which the parser's error is returned. Spans index the ORIGINAL text. Not forgiven, still `ScanError`: a tab as block indentation (`job:\n\timage: x`), shapes that stay invalid with a space (`a:\t- b`), and the other tab forms (`?\tx`, `-\t- x`). The error of the retried text is reported (its own line) |
| a lone `\r` is a line break for the parser but not for `LineIndex` (comment placement, multi-line checks, the property/comment skips): spans and comments go wrong | a file with a bare CR (`\r` not followed by `\n`) is `ScanError { line, msg: "bare CR line endings are not supported" }` (review L1); CRLF is fine |
| CRLF: `\r` is one char, excluded from spans; multi-document spans are file-global | none |

Shape as built (`pin_core::yaml`, feature `yaml`):
`scan(text, ScanOpts { mask_templates }) -> Result<Vec<Node>, ScanError>` with the existing
`plugin::ScanError { line: Option<usize> /* 1-based */, msg }` (no separate `YamlError`);
`Node::{Scalar(ScalarRef), Alias { path, id, span, parent }, Tagged { path, tag, parent }}`;
`ScalarRef { path, doc, line, span, style, value, anchor, parent, in_flow, templated }`:
`path: Vec<PathSeg>` is the value path from the document root (`PathSeg::{Key, Index, Complex}`; keys are
not nodes, a non-scalar or alias key is `Complex` and its subtree is skipped); `doc` the 0-based
document; `line` the 0-based line of `span.start` (`LineIndex::line_of`); `span` the **byte** range of
the token incl. quotes (properties excluded; block: content only, end trimmed; implicit null: empty
span, see the table); `style` `Plain|Single|Double|Block`; `value` decoded (from the masked text when
masking); `anchor` the id when the scalar defines one; `parent` the id of the enclosing collection
(ids from 1 in event order, 0 = the document is the scalar), which groups the keys of one mapping even
when a key such as `include:` repeats; `in_flow` whether the enclosing collection is (nested in) a flow
collection; `templated` as in the table. `Alias` and `Tagged` carry `parent` too. Plugins match path
patterns over `Vec<Node>` (`image`, `services[*].name`, `include[*].ref`, `dependencies[*].version`,
`**.image.{repository,tag,digest}`), so `scan` stays pure. Char→byte is one `Vec<usize>` per file, so
a span from the parser never needs `col()`; an impossible span is an internal `ScanError`, never a
panic (every marker lookup and scan is bounded; a test runs `scan` on every char-boundary prefix of the
sample texts, masked and unmasked).

A file that does not parse (after masking) is a per-file error: `ScanError { line, msg: "invalid YAML:
{parser info}" }`, which `run::pin_file` turns into `FileError::Parse` (`<path>:<line>: invalid YAML: …`)
with nothing written; no partial targets, no second hand-rolled discoverer (my call; revisit in DF1).
The `pin-ci` driver exited 1 on it and stopped at the first such file (printing the rows of the files before it, review L2) until M5d;
the toolkit records it under `plugins.<id>.errors`, exits 2 and continues with the other files (M5d, as built). Hand-rolled code stays only for the
fix-ups, template masking, trailing-comment detection, `requote` and the C5 shell tokenizer.

**Comment guard (review M2).** `edit::trailing_comment` knows lines, not YAML: in a multi-line flow
include `{project: g/p, ref: main, file: a\n  .yml}` the end of line 1 lies inside the plain scalar
`a .yml`, and the `  # main` comment would break the file. `yaml::inside_token(nodes, at)` tells whether a
byte lies strictly inside a scalar or alias token (a line break of a multi-line plain or quoted
scalar, also a quoted one's `\`-escaped break). The gitlab-ci discovery sets `PinTarget::comment_ok` on `Component` and
`IncludeRef` targets from it, and `policy::pinned` reports `skipped (no safe place for comment)` and
writes nothing for that target when a comment would be inserted and `comment_ok` is false (a `#`
already after the token means no comment is inserted, so no skip). The guard lives in the gitlab-ci
crate because `run`/`edit` never see node spans (`Plugin::Target` is opaque to the core); a plugin
that adds comments (helm?) calls `inside_token` itself. Not covered: mapping keys and the subtrees
of `Node::Tagged` (they are not nodes); a multi-line key or tagged scalar next to the target in a
flow mapping is not a case seen in CI files.

gitlab-ci matching (`discover.rs`, built in M5c-y): over the scalars with a non-empty span, paths
relative to the document root. `image` kind: `image` or `services` at the root or one key down (a job,
`default`, a hidden template), followed by nothing, `name`, an index, or an index and `name`.
`component` kind: exactly the root key `component`. `include` kind: a mapping at `include` or
`include[n]` (root only), grouped by `parent`, with a `project` and a `ref` scalar (first of each, any
order); the target is the `ref`, with `flow` taken from the scalar. A block scalar at a target path
is `skipped (block scalar)`. **A quoted or plain scalar whose text spans lines at a target path is
`skipped (multi-line scalar)`**: splicing it would drop a line break (and, for an include `ref`, lose
the `# ref` comment). Both are reported through `PinTarget::Unsupported`, whose range is cut at the
first line break so the row's `old` stays one line. Aliases, merge keys and `!reference` are not
targets. Behaviour change versus the old `discover.rs` (bugs it fixed, reviewed as snapshot diffs; R7,
R8, R9 in §9): `image: &img "ubuntu:24.04"` no longer yields a value containing `&img`, `image: *img`
and `!reference [...]` are no longer targets, `!!str node:20` is pinned in place (the tag stays), block children
under CRLF and under an anchored key are found, and the first key after a BOM is no longer lost. Also fixed (found by the review's differential against the old code; no fixture, no snapshot changed; R10 in §9): flow `image: {name: x}` is pinned (the old code made the whole flow text an "unparseable" target), `services: [a:1, {name: b:1, alias: x}]` is found, a value followed by `\t# comment` no longer includes the comment, and a multi-line flow include is found.

## 7. Configuration

### 7.1 Layers and precedence

`built-in defaults < global ($XDG_CONFIG_HOME or ~/.config)/pin-toolkit/config.toml, also on macOS < repo .pin-toolkit.toml (git root, or --config / PIN_TOOLKIT_CONFIG) < env PIN_TOOLKIT_* < CLI`

Format is TOML. A `version = 1` key is required. Unknown keys are errors, which catches typos.
Merge rules: tables deep-merge, scalars take the higher layer, `files` lists are **replaced** by the
higher layer, `ignore` lists **concatenate**. `--no-config` skips the global and repo files.

Smaller config first (D35): M5d implements `defaults < repo < env < CLI` with the full repo schema
below; M5e' inserts the global layer, `[hosts]` and `config show --origin`. The security rule (§7.3)
holds from M5d: a global-only key in a repo file is exit 2, and without a global layer tokens come
from env only, to the primary host (D31); `CI_JOB_TOKEN` only to `CI_SERVER_HOST` (D37).

```toml
version = 1
[update]       # pin is the default; these mirror --update/--update-major/--min-age-days
enabled = false
major = false
[release_age]
min_days = 0
unknown = "allow"
[plugins.gitlab-ci]
run = true          # runtime toggle (O1); a plugin not in the build is never run
ignore = ["registry.example.com/legacy/*"]   # refs never touched (concatenated across layers)
files = [".gitlab-ci.yml", "**/*.gitlab-ci.yml", ".gitlab/ci/**/*.yml"]
gitlab-hosts = ["gitlab.com", "gitlab.example.com"]  # hosts to resolve (not [hosts]); others → skipped (host not enabled). Tokens only if bound globally (§7.3)
kinds = { image = true, service = true, component = true, include = true, helm-oci = true }
[plugins.helm]
run = true
kinds = { chart-dependency = true, values-image = true, helmfile-release = true }
values = { tag-digest = true }           # O2: default on, opt-out per repo
helmfile = { oci-digest = false }
aliases = { stable = "https://charts.helm.sh/stable" }   # name → URL only, never credentials
```

**As built (M5d, T-D4–T-D7):**
- One precedence function, `Effective::fold(&[Layer])`; repo, env and CLI each produce a `Layer`. Scalars:
  last set wins; `files` and `gitlab-hosts` are replaced; `ignore` concatenates; `kinds` merge per key.
- Repo file = `--no-config` ? none : `--config` (must exist) : `PIN_TOOLKIT_CONFIG` (must exist) :
  `<git root>/.pin-toolkit.toml` if it exists; `config file not found: {path}`, an unreadable file
  `{path}: {io error}`. Messages name the file relative to the cwd (review-2 N1).
- Repo parsing: a `toml::Table` pre-pass (global-only keys, then `version`, then plugin ids) before the
  typed parse with `deny_unknown_fields`. Tables of known but unbuilt plugins are validated too. The
  `files` globs of **every** plugin table are validated (review-2 L2). Rejected (exit 2): the global-only
  keys `hosts` and `concurrency`, unknown keys, plugin ids or kinds, wrong types, `version` missing or
  ≠ 1, invalid globs, a `gitlab-hosts` entry that is not a plain host (§7.3), a helm `aliases` URL with
  userinfo (`plugins.helm.aliases.{name}: a repository URL must not contain credentials`, PR-L14). There
  is no `gitlab-host` key, so a repo cannot set the primary host.
- Env: `PIN_TOOLKIT_{UPDATE, UPDATE_MAJOR, MIN_AGE_DAYS, RELEASE_AGE_UNKNOWN, CONCURRENCY, JSON, ONLY, SKIP,
  GITLAB_HOST, CONFIG}`. Booleans `1/0, true/false, yes/no, on/off`, `allow`/`block`, both
  case-insensitive; lists comma-separated (trimmed, empty items dropped, an empty list is unset); an
  empty value is unset. A bad value of a known variable → exit 2 (`{VAR}: expected a boolean (…), got
  "{v}"`, `{VAR}: expected a number, got "{v}"`, `{VAR}: expected allow or block, got "{v}"`,
  ``PIN_TOOLKIT_GITLAB_HOST: invalid GitLab host `{h}` ``). Any other `PIN_TOOLKIT_*` variable → one
  stderr warning `pin-toolkit: ignoring unknown environment variable {VAR}` each (sorted), exit
  unaffected (main, PR-M3: GitLab exports every CI/CD variable to the job).
- `PIN_TOOLKIT_ONLY`/`PIN_TOOLKIT_SKIP` are ignored with a plugin subcommand or alias (main, PR-L6, T-D28);
  in all-plugins mode an unknown id → exit 2 and `PIN_TOOLKIT_ONLY=<unbuilt>` → `plugin <p> not in this
  build`, exit 2.

### 7.2 Where settings may live

| Setting | Global (HM) | Repo | Env / CLI |
|---|---|---|---|
| `plugins.<p>.run`, `kinds`, `files`, `ignore`, `update`, `release_age`, plugin options | default | ✓ overrides, so CI and local runs agree | ✓ |
| `concurrency`, output format | ✓ | — | ✓ |
| **`[hosts]`**: GitLab hosts that may receive tokens, which env var holds each token, helm repo URL → credential env names, CA file, registry aliases/mirrors | **✓ only** | ✗ (exit 2) | ✓ (`--gitlab-host`, `CI_SERVER_HOST`) |
| secrets | never | never | env, `~/.docker/config.json`/`$DOCKER_CONFIG`, `$HELM_REGISTRY_CONFIG`, `$HELM_REPOSITORY_CONFIG` |

**As built (M5d):** `concurrency` in a repo file is exit 2 with the global-only message, like `[hosts]`
(``{path}: `{key}` is a global-only key; it is not allowed in a repository config``). The output format,
`--dry-run`/`--check` and `only`/`skip` have no repo key.

### 7.3 Security rule: credentials bind to hosts only in global config or env

A token is sent only to a host that the user, global config, env or CLI bound it to. Today
`GITLAB_TOKEN` goes to `--gitlab-host`. If a cloned repo's `.pin-toolkit.toml` could set that host,
the repo could exfiltrate the token. So a repo file may *reference* a host (component hosts,
helm aliases): references to an unbound host resolve **anonymously**, and if that fails the
reference is `unresolved (<host>: not trusted for credentials — bind it in ~/.config/pin-toolkit/config.toml)`.
`CI_JOB_TOKEN` is bound to `CI_SERVER_HOST` only (M5d, pulled from F3). Today it would also be sent to the default
gitlab.com. The HM-generated config sits in the world-readable `/nix/store`, so it holds env var
*names* only, never secrets.

**Which host gets `GITLAB_TOKEN` (D31, my call, from builder check).** `GITLAB_TOKEN` is sent only
to the single *primary* GitLab host: `--gitlab-host`, else `CI_SERVER_HOST`, else `gitlab.com`.
Every other host in `plugins.gitlab-ci.gitlab-hosts` resolves anonymously unless a global `[hosts]`
entry binds a token env var to it. `gitlab-hosts` only decides *which* hosts are resolved; it never
grants credentials. `[hosts]` shape (global file only; the builder may extend it, not loosen it):

```toml
[hosts."gitlab.example.com"]
kind = "gitlab"
token-env = "GITLAB_EXAMPLE_TOKEN"     # env var NAME, sent as PRIVATE-TOKEN
[hosts."charts.example.com"]
kind = "helm-repo"
username-env = "CHARTS_USER"
password-env = "CHARTS_PASSWORD"
[hosts."registry.example.com"]
kind = "oci"
ca-file = "/etc/ssl/certs/corp-ca.pem"  # private CA (F3; §8.4)
```

**As built (M5d, D31/D37 implemented, 3607bc7):** pin-core `GitlabEnv { primary, hosts, gitlab_token,
ci_job_token, ci_server_host }`; primary = `--gitlab-host` > `PIN_TOOLKIT_GITLAB_HOST` > `CI_SERVER_HOST` >
`gitlab.com` (T-D12); resolve set = {primary} ∪ `plugins.gitlab-ci.gitlab-hosts` (`CI_SERVER_HOST` is not
added implicitly); `GitlabEnv::auth_for(host)`: `PRIVATE-TOKEN` (`GITLAB_TOKEN`) only to the primary,
else `JOB-TOKEN` (`CI_JOB_TOKEN`) only to `CI_SERVER_HOST`, else anonymous. Token-holding types have a
redacting `Debug` (T-D29). Host values (`--gitlab-host`, `PIN_TOOLKIT_GITLAB_HOST`, `gitlab-hosts`) must
match `^[A-Za-z0-9.-]+(:[0-9]+)?$`, else exit 2 (review-2 L6: `gitlab.com@evil.example` would send the
token to `evil.example`). **Deferred to M5e' (T-D30, main, PR-M6 option b):** the `unresolved (<host>:
not trusted for credentials — bind it in …)` message. In M5d a non-primary `gitlab-hosts` host that is
not `CI_SERVER_HOST` resolves anonymously, and a failure keeps the plain `unresolved (…)` reason. Wiremock
tests: pin-core `gitlab_hosts.rs`, pin-toolkit `credentials.rs`.

## 8. Packaging and distribution

### 8.1 Flake (crane)

- Outputs: `packages.<sys>.{default, pin-toolkit}`. On Linux also `pin-toolkit-static` (musl, `+crt-static`) and `container`. Plus `checks` (clippy `-D warnings`, nextest, fmt, cargo-deny, one build per single-plugin feature set, and pin-core with no and with each single resolver feature), `overlays.default`, `homeManagerModules.default` with the alias `homeModules.default`, and `devShells.default`.
- Systems: `x86_64-linux`, `aarch64-linux`, `aarch64-darwin`.
- Use crane, not `buildRustPackage`: `buildDepsOnly` caches dependencies across build, clippy and test, and crane documents a static-musl example. Sources: https://github.com/ipetkov/crane (v0.24.0) and its `examples/cross-musl`.
- `nix/package.nix` goes through `callPackage` with the argument `plugins ? allPlugins`. `allPlugins` is read from `crates/pin-toolkit/Cargo.toml` `[features]` via `importTOML`, so Cargo is the single source of truth. That gives `--no-default-features --features ${plugins}`, `postInstall` symlinks `pin-<p>` (plus `pin-ci` when `gitlab-ci` is included and the alias is kept, §6.6), `passthru.plugins`, and `meta.mainProgram = "pin-toolkit"`.
- Selecting plugins: `pin-toolkit.packages.${system}.default.override { plugins = [ "gitlab-ci" ]; }`. This is a different store path, so Cachix misses and it builds locally.
- Tests in the sandbox: no network, and live/oracle tests stay env-gated. wiremock binds to loopback, so darwin needs `__darwinAllowLocalNetworking = true`.
- Consumers must not set `inputs.pin-toolkit.inputs.nixpkgs.follows`, because that changes the store path and loses the Cachix hit. Document this.

### 8.2 Home Manager module

O1 (owner, 2026-11-09): **enabling is a build decision, configuring is a runtime decision.**
`plugins.<name>.enable` in Home Manager decides whether the plugin is *in the build*; everything
else about a plugin (whether it runs in a given repo, kinds, files, options) is configuration.

```nix
programs.pin-toolkit = {
  enable = true;
  package = <flake default>;                    # all plugins; the Cachix-cached store path
  plugins.<name>.enable = true;                 # generated from allPlugins; default true; = in the build
  plugins.<name>.settings = { };                # → [plugins.<name>] (run, kinds, files, …)
  settings = { };                               # → rest of config.toml (pkgs.formats.toml), no secrets
  # read-only: finalPackage
};
```

- `finalPackage` = `cfg.package` **unchanged** when every plugin is enabled (same store path as the
  flake default → Cachix hit; the owner's own setup). Otherwise
  `cfg.package.override { plugins = <enabled>; }` — a feature-trimmed build that is **not** in
  Cachix and compiles locally; the option description says so.
- The module writes `xdg.configFile."pin-toolkit/config.toml"` and adds `home.packages = [ cfg.finalPackage ]`.
- Assertion: an enabled plugin missing from `cfg.package.passthru.plugins` (custom package) fails evaluation.
- Runtime: `[plugins.<p>] run = false` (any config layer, incl. a repo's `.pin-toolkit.toml`) turns a
  built plugin off for that scope. A config that asks to run a plugin that is not in the build is
  reported once as `skipped (plugin <p> not in this build)` (exit unaffected); `pin-toolkit <p>`
  for a missing plugin exits 2 with the same message. CI uses the container image, which always
  has all plugins, so a repo's config behaves the same in CI regardless of a user's local build.
- As built (M5d, T-D11): the notice is the stderr line `pin-toolkit: skipped (plugin <p> not in this
  build)`, printed once per plugin and only in all-plugins mode, for a repo `[plugins.<p>]` table with
  `run ≠ false`. An explicit request (`pin-toolkit <p>`, `pin-<p>`, `--only <p>`, `PIN_TOOLKIT_ONLY` in
  all-plugins mode) exits 2 with `pin-toolkit: plugin <p> not in this build`.

Source for `homeModules`: https://github.com/nix-community/home-manager/blob/master/flake-module.nix.

### 8.3 GitHub Actions, Cachix, releases

One workflow `ci.yml` (on PRs, pushes to `main`, `v*` tags). Every `uses:` is pinned by SHA with a
`# vX` comment, which is dogfooding.

| Job | Runs on | Does |
|---|---|---|
| `nix` (matrix `ubuntu-latest`, `ubuntu-24.04-arm`, `macos-latest`) | always | `cachix/install-nix-action` (v31), `cachix/cachix-action` (v17, cache `pin-toolkit`; pushes only on main and tags, so PRs and forks pull only), `nix flake check -L`, `nix build .#pin-toolkit` |
| `create-release` | tag, needs `nix` | checks that the tag equals `workspace.package.version`, then `gh release create $TAG --draft --verify-tag --generate-notes` **once** |
| `binaries` (matrix x86_64/aarch64, each built natively) | needs `create-release` | `nix build .#pin-toolkit-static`, tar.gz + `.sha256`, `actions/attest-build-provenance`, `gh release upload --clobber` |
| `images` (matrix per arch) | needs `create-release` | `nix build .#container`, push `ghcr.io/<owner>/pin-toolkit:<ver>-<arch>` with skopeo |
| `manifest` | needs `images` | `docker buildx imagetools create` for `:<ver>` and `:latest`, then attest |
| `publish` | needs `binaries`, `manifest` | `gh release edit $TAG --draft=false` |

This avoids the race of the naive design, where every matrix job runs
`gh release create … || true`: the release is now created exactly once, as a draft, and published
only after all assets exist. Use install-nix-action rather than DeterminateSystems/nix-installer-action,
because the latter now installs Determinate Nix by default and calls upstream Nix unsupported.
macOS ships through Cachix and `cargo install --locked` only: Nix-built darwin binaries reference
`/nix/store` dylibs. cargo-dist only becomes worth it if non-Nix installers become a goal.

### 8.4 Container for GitLab CI

`dockerTools.buildLayeredImage`: the static binary, static busybox (GitLab needs `sh` and `grep`)
and `dockerTools.caCertificates`, plus `SSL_CERT_FILE`. There is **no Entrypoint**, so GitLab can
pipe the script into `sh` (https://docs.gitlab.com/ci/docker/using_docker_images/). Expect about
10–15 MB. It is published to GHCR only.

```yaml
pin-check:
  image: ghcr.io/<owner>/pin-toolkit:<ver>@sha256:<digest>   # kept fresh by pin-toolkit itself
  script: [ pin-toolkit --check --json > pin-report.json ]
  artifacts: { when: always, paths: [pin-report.json] }
  rules: [ { if: $CI_PIPELINE_SOURCE == "merge_request_event" } ]
```
Registry credentials come via `DOCKER_AUTH_CONFIG` written to `~/.docker/config.json` (or
`$DOCKER_CONFIG`), and GitLab credentials via `CI_JOB_TOKEN`/`GITLAB_TOKEN`. Private CAs need F3:
reqwest with both `rustls-tls-webpki-roots` **and** `rustls-tls-native-roots`. Today's `rustls-tls`
uses bundled roots only and ignores `SSL_CERT_FILE`.

## 9. Migration of this repo

M4b-1 is merged (0e2047b), including `impl Default for PinOptions`. There is no compatibility layer
(D33): the safety net is snapshot review plus the byte-exact and idempotence properties (§11), not
frozen output. Rows are in execution order.

| Step | What | Safety net |
|---|---|---|
| S2 | YAML discovery spike: **done** 2026-11-10, adopt with caveats (§6.9) | — |
| M5a | Snapshot tests: a lib-level test runs every e2e fixture and snapshots `render_human()`, `to_json()` and the edited file contents (the builder may pick `insta`); mock-port URLs in reasons are redacted. Add the byte-exact and idempotence property tests over all fixtures. Add fixtures for R1, R2 (CRLF) and anchors/aliases; their first snapshots record today's wrong output. Plain tag `pre-pin-toolkit` (a reference point, not a compatibility baseline) | baseline |
| M5b | `git mv src tests` → `crates/pin-gitlab-ci/`. The package is still named `pin-ci`. Root workspace manifest, root `Cargo.lock`. Oracle path: `oracle.rs:21` `REPO = env!("CARGO_MANIFEST_DIR")` → `concat!(env!("CARGO_MANIFEST_DIR"), "/../..")` | all tests green, snapshots unchanged |
| M5c | Split out `pin-core`. First `git mv` image_ref, registry, gitlab, versioning and report verbatim, in move-only commits. Then add `Status`, `Splice`/`Finding`, the lazy feature-gated `Ctx` (§6.2, D36), `decide_image`/`pin_inline_image`, `Plugin` + `run::pin_file`. lib.rs becomes `GitlabCi`; today's `pin_files(paths, &PinOptions, &reg, &gitlab)` may survive as a test helper until M5d, not as API. Rename the package to `pin-gitlab-ci`. `is_sha` dedups into the ungated `git::is_commit_sha` (github shares it in M9). `discover.rs` moves verbatim. Fix R1 (`coalesce_comments`) and R2 (one shared `LineIndex`), one commit each. `main.rs` stays as the crate's `[[bin]] pin-ci` until M5d. Tests: mock_registry, registry_hosts, hub_api, gitlab_client and live_gitlab move to `pin-core`; e2e, e2e_components, live, oracle and fixtures stay in `pin-gitlab-ci`. Relative `.tmp` paths resolve per package and are still covered by the root `.gitignore` | snapshots unchanged except the reviewed R1/R2 diffs; properties green |
| M5c-y | **Done** (branch `m5cy-yaml-discovery`, 11 commits, last 7546e0f). Own step right after M5c (kept M5c manageable): `core::yaml` = saphyr-parser + fix-ups (§6.9); gitlab-ci discovery is path matching over `Vec<Node>`; the `discover.rs` line-scanner primitives are deleted (≈ −800 lines). Fixes R7, R8, R9. Also built: `PinTarget::Unsupported` (`skipped (block scalar)`, `skipped (multi-line scalar)`, §6.9), invalid YAML as a file error, the review carry-overs L1 and L7 (§14.1). Review fixes after 8515e54 (`.superpowers/sdd/2026-11-10-m5cy/fix-review.md`): tab after `key:` retry (H1), plain scalar next to a mask is `templated` (M1), comment guard `skipped (no safe place for comment)` (M2), bare CR is a `ScanError` (L1), `pin-ci` prints the earlier rows before a file error (L2) | discovery-parity test against a frozen record of the old output plus an exact exception table (§11); exactly 12 snapshots changed (`ci-anchors`, `ci-r2-crlf`, `ci-tags`, `ci-bom` × human/json/file), all reviewed |
| M5d | **Done** 2026-11-11 (branch `m5d-toolkit-binary`, 8f053e2 … 3941898 plus the docs close-out; plan `2026-11-10-pin-toolkit-m5d-toolkit-binary.md`, two review checkpoints, reviews in `.superpowers/sdd/2026-11-10-m5d/`; as built: §6.1–§6.8, §7.1–§7.3, §8.2). Minimal `pin-toolkit` binary, replaces `pin-ci`: walk, routing, `pin-toolkit gitlab-ci` and `pin-<plugin>` aliases (optional `pin-ci` alias, §6.6), `service` kind tag, combined report, exit codes 0/1/2, `--check`, memo. Config: repo `.pin-toolkit.toml` + env + CLI only (§7.1, D35). `CI_JOB_TOKEN` bound to `CI_SERVER_HOST` only (D37). Delete the crate's `[[bin]] pin-ci` | toolkit snapshots (human + JSON) over the M5a fixtures, diffs reviewed; exit-code table tests; repo `[hosts]` → exit 2; wiremock: `CI_JOB_TOKEN` never sent to another host |
| M5f | Docs: README → toolkit README plus per-plugin sections (fix R5 drift; states the `pin-ci` alias decision), a pointer from the old spec, LICENSE (O4), push to GitHub with full history (O3) | — |
| M5e' | Global config layer (`~/.config/pin-toolkit/config.toml`, written by the HM module), `[hosts]` bindings (D31), `config show --origin`. Right before M6, whose HM module needs it | precedence table tests; token-binding tests (§7.3) |

**Oracle, kept until DF1:** the root files stay unmodified and `tests/oracle.rs` is adapted to the new
crate API. `PIN_CI_LIVE` and `PIN_CI_ORACLE` keep their names; the command becomes
`cargo test -p pin-gitlab-ci --test oracle`. The owner decides its retirement after dogfooding.

**Latent bugs: fixed during the restructure (M5c), not preserved.** Each fix is its own commit that
deliberately updates its snapshot and gets a README note.

| ID | Issue (found by reading the code, not executed) | Handling |
|---|---|---|
| R1 | `apply_splices` sorts by `Reverse(start)` stably, so two insertions at the same offset come out in **reverse** input order. `include: [{project: a, ref: main}, {project: b, ref: dev}]` becomes `…]  # dev  # main`, with two comments and in reverse order | M5c: one combined comment per line, refs in source order: `  # main, dev` |
| R2 | `comment_insert_offset` computes line starts with `text.lines()` and `len()+1`. `lines()` strips `\r\n`, so offsets drift by one per earlier CRLF line, while `discover` uses `split_inclusive('\n')` | M5c: one shared `LineIndex`, comment inserted before `\r\n` |
| R3 | (not a bug) partial-write semantics and one semaphore per file shared by registry and GitLab calls | keep in `run` |
| R6 | new kinds (`helm-oci`, C3b) change output for existing CI files | intended; README note per kind; per-kind toggles (`kinds`) in config. C3b counts as the `component` kind |
| R7 | `discover()` keeps `&img` in an anchored value, treats the alias `*img` as a target, and treats `!reference [...]` and the tag in `!!str node:20` as (unparseable) targets (found by S2; the `ci-tags` fixture added in M5c-y) | M5c-y: **fixed** by parser discovery (§6.9): the definition is pinned once (`&img alpine:3.20@sha256:…`, `!!str node:20@sha256:…`); aliases, merge keys and `!reference` are not targets and have no row |
| R8 | `discover()` misses block children in two cases (found by the M5a fixture run): under CRLF a `key:\r` line counts as a key with a value, so `include:`/`services:` blocks and every job-nested `image:` are not targets; children of an anchored key (`.base: &base`) are never targets | not fixed in M5c (the hand-rolled code was deleted by M5c-y); recorded in the M5a snapshots (`ci-r2-crlf`, `ci-anchors`). M5c-y: **fixed** by parser discovery (CRLF block `include:`/`services:`/job `image:` and children of `.base: &base` are targets); those snapshots now pin them; listed with R7 and R9 as the documented parity exceptions |
| R9 | `discover()` misses every child of a first key behind a BOM (`\u{FEFF}default:`): `top_key` rejects the BOM-prefixed key (found while planning M5c-y; the `ci-bom` fixture) | M5c-y: **fixed** by parser discovery (a BOM is char 0 of the char→byte table); third documented parity exception; `ci-bom` gained the row for line 2 |
| R10 | four more legacy `discover()` bugs, found by the M5c-y review's differential over generated CI files; no fixture covers them: flow `image: { name: x }` / `{name: x}` makes the whole flow text an "unparseable" target; `services: [a:1, {name: b:1, alias: x}]` (flow services with an alias) is missed; a value followed by `\t# comment` includes the comment in the value; a multi-line flow include is not discovered (`flow_close` needs the closing brace on the same line) | M5c-y: **fixed** by parser discovery (§6.9); not parity exceptions (no fixture, no snapshot row) |

## 10. Milestones

S ≈ 1–2 tasks, M ≈ 3–4, L ≈ 5+. Each milestone gets its own plan in `docs/superpowers/plans/`.

| # | Milestone | Size | Behaviour change |
|---|---|---|---|
| M4b-1 | done (0e2047b): release-age guard, `Default` for `PinOptions`, oracle + README | M | planned feature |
| S2 | YAML discovery spike (done, §6.9) | S | none |
| M5a–c | snapshots → workspace move → core split + lazy resolvers + R1/R2 fixes (§9) | S + S + L | R1/R2 fixed (reviewed snapshot diffs) |
| M5c-y | done (7546e0f): gitlab-ci discovery on `core::yaml` (saphyr-parser + fix-ups) | M | R7, R8, R9 fixed (reviewed snapshot diffs); invalid YAML is a file error (exit 1 until M5d) |
| M5d | done (2026-11-11, … 3941898 + docs): minimal `pin-toolkit` binary (replaces `pin-ci`); repo config + env + CLI; `CI_JOB_TOKEN` → `CI_SERVER_HOST` only | M (built as 31 tasks + review fixes) | new binary; `pin-ci` kept as a plain argv[0] alias |
| M5f | docs, license, GitHub publish | S | none |
| M5e' | global config (HM-written), `[hosts]`, `config show --origin` | M | new config layer |
| M6 | flake + crane + checks + HM module (M), GitHub Actions + Cachix (S), releases + container (M). Human prerequisites (O3): GitHub repo, Cachix cache + `CACHIX_AUTH_TOKEN`, GHCR push rights | L | none |
| DF1 | dogfood: the owner runs pin-toolkit on his real repos (locally via HM, in CI via the container). Each fix from real use is its own commit with reviewed snapshots | open (owner-driven) | yes, from real use |
| G1 | gitlab-ci C3b: `include: - component:` + `$CI_SERVER_FQDN` alias | M | yes (new targets) |
| F3 | native TLS roots (§8.4); its `CI_JOB_TOKEN` half moved to M5d (D37) | S | yes, documented |
| M4b-2 | component/include `--update` (GitLab tags list, `order_by=version`), comment-aware re-pin (`existing_comment`), verify in `--check`, GitLab tag dates for release age | M | yes |
| M7 | gitlab-ci `helm-oci` (C5): script-line shell tokenizer, OCI chart resolver (`+`→`_`, Accept, helm config mediaType check) | M | new kind |
| M8a | helm plugin skeleton + values images (H2, H3 digest-key form, O2) on `core::yaml` with template masking (§6.9) | M–L | new plugin |
| M8b | Chart.yaml/requirements deps (H1): `HelmRepoClient` (pin-core feature `helm-repo`; index.yaml, gzip, per-run cache, relative `urls`), OCI chart tags, `ver::semver` constraints, follow-ups | L | new kind |
| M8c | helmfile (H4), incl. `.gotmpl` via masking | M | new kind |
| M9 | github-actions plugin (A1–A3), `GitHubClient` (pin-core feature `github`) | M | new plugin |
| later | C6 classic `helm … repo/chart`, `sha:` value keys, template scan for `tag-digest`, `--refresh-lock` (shells out to `helm dependency update`), `include:integrity` | — | — |

Order (D34): M4b-1 ✓ → S2 ✓ → M5a → M5b → M5c → M5c-y → M5d → M5f → M5e' → M6 → **DF1** → G1 → F3 →
M4b-2 → M7 → M8a → M8b → M8c → M9. Real use comes before Helm (owner, 2026-11-10): what dogfooding finds
shapes the core before the helm plugin builds on it. GitLab first (owner); distribution right after
the minimal binary, so dogfooding uses the real install path. M5f precedes M5e' because M6 needs the
GitHub repo, and M5e' precedes M6 because the HM module writes the global config.

## 11. Verification (per milestone gate)

| Property | Check |
|---|---|
| Byte-exact (hard invariant) | ∀ fixture f: the bytes outside `⋃ edits` are identical before and after (a property test over all fixtures and plugins): edits touch only the reference bytes |
| Idempotent (hard invariant) | ∀ f: a second run has 0 edits and every row is `ok`/`skipped` (wiremock) |
| Snapshot review | Snapshots of the human report, JSON report and edited files (M5a; toolkit output from M5d) fail on any output change. A change is accepted only by deliberately updating the snapshot, and the commit names it as intended. Oracle (`PIN_CI_ORACLE=1`) green while it is kept |
| Discovery parity (M5c-y) | ∀ fixture: parser discovery yields the old `discover.rs` targets except the documented R7, R8 and R9 cases (exact exception table; the old output is a frozen record, `tests/parity/legacy-discover.txt`, because the code is deleted), and ∀ spliceable target `text[span]` equals the value as written (plain, `'…'` or `"…"`) on its line. Fixtures cover non-ASCII before a target, BOM, CRLF, quoted value + `# comment`, `&anchor`/`!tag`, aliases, `<<:`, multi-document, a realistic pipeline; masked `{{ }}` is covered at unit level (no consumer before M8a) |
| Lazy resolvers | a run whose files have no target of a kind constructs no client of that kind (construction counter in a test). pin-core builds with no resolver feature and with each one alone (flake checks) |
| Plugin isolation | `scan` is pure: unit tests with no network. Every single-plugin feature build compiles (flake checks) |
| Fail loud | Every auth, network, alias or resolver-construction failure yields `unresolved` and exit 1. A YAML parse error or a global-only key in the repo config yields exit 2 (from M5d) |
| Planted violations | Each new check is first seen red: flip a byte outside a span; revert the R1/R2 fix (its snapshot fails); use `Marker::index()` as a byte offset, or drop the quoted-span re-scan (parity fails); build a resolver eagerly (lazy test fails); put `[hosts]` in a repo file |

## 12. Decisions

| # | Decision | By | Reason |
|---|---|---|---|
| D1 | New project pin-toolkit (working name), open source on GitHub, its CI builds releases and pushes to Cachix | owner | — |
| D2 | Installable and customisable via a Home Manager module (plugins on/off) | owner | O1: HM enable = in the build (all enabled → cached default package); runtime `run`/kinds/settings = configuration |
| D3 | Rust for anything beyond thin glue | owner | — |
| D4 | Pin rule (§3) | owner | — |
| D5 | Approach A: shared core library and plugins by file format, each also standalone. Per-kind toggles. One pass, one report, one exit code | owner | — |
| D6 | gitlab-ci scope: images, services, components, includes, `helm … oci://` script lines. helm scope: Chart.yaml deps, values images, helmfile ("all helm files possible") | owner | — |
| D7 | GitLab-centric first, easy to extend (github-actions) | owner | — |
| D8 | Steer this repo into pin-toolkit | owner (stated as likely) | — |
| D9 | One multi-call binary plus cargo features per plugin (default all) | my call | shared cache and one artifact; features enable the "not installed" override cheaply |
| D10 | Plugin = pure `scan` + async `resolve` that owns policy. Core owns resolvers, cache, edits, run, report. No core `Reference` enum | my call | policy is format-specific; a pure scan keeps a WASM path open |
| D11 | One file = one plugin. Overlapping splices → unresolved | my call | no conflict resolution needed |
| D12 | Crates `pin-core`, `pin-gitlab-ci`, `pin-helm`, `pin-github-actions`, `pin-toolkit`. Aliases `pin-<plugin>`. **Changed:** no `pin-ci` legacy mode; `pin-ci` at most a plain alias of `pin-toolkit gitlab-ci` if cheap, else dropped; the README says which | my call; `pin-ci` part owner (2026-11-10) | short names; no users to stay compatible with |
| D13 | Exit codes 0/1/2 (§6.5). **Changed:** no legacy 0/1 | my call; owner (2026-11-10) | no compatibility layer |
| D14 | `--check` (dry-run + exit 1 on any change). Verify comment↔SHA from M4b-2 | my call | CI gate (pinact precedent) |
| D15 | JSON keeps pin-ci's shape and adds `schema_version`, `plugin`, `kind`, `plugins`, `follow_ups`. **Changed:** no byte-identity with pin-ci's JSON | my call; owner (2026-11-10) | continuity of design; stable from the first release |
| D16 | Disabled kind → no row | my call | explicit user choice, not a silent skip |
| D17 | TOML config, layering and merge rules as in §7.1. `version = 1`. Unknown keys are errors | my call | pinact/Renovate precedent |
| D18 | Host and credential bindings only in global config, env or CLI. Repo config may only reference | my call | prevents token exfiltration |
| D19 | Per-run memo cache, errors included, no disk cache | my call | consistency without staleness |
| D20 | Release age everywhere timestamps exist. Unknown age = allow (configurable). Default off | my call | matches today |
| D21 | Chart deps: pin = highest stable match of the constraint, constraint kept as comment. `--update` = highest within the recorded constraint; with no constraint, the image policy (same major, `--update-major` lifts it) | my call | respects author intent; consistent with images |
| D22 | Never write Chart.lock/helmfile.lock. Report follow-up commands | my call | aggregate digest is fragile to recompute (Go JSON details) |
| D23 | `helm-oci` default on in the toolkit (needs Helm ≥ 3.17, released 2025-01). helmfile OCI digest default off | my call | old helm is rare; helmfile support is too new to verify |
| D24 | Hand-rolled Masterminds constraint subset in `ver::semver` (unchanged). **Changed:** the "no YAML parser" clause is replaced by D32 | my call; YAML clause owner (2026-11-10) | Cargo `semver` semantics differ |
| D25 | **Changed:** R1/R2 fixed during the restructure (M5c), not preserved; F1/F2 are no longer milestones. R1 → one combined comment `  # main, dev` | owner (2026-11-10) | no users to stay compatible with; fixing while the code moves is cheaper |
| D26 | C3b (`include: - component:`) added in G1 (legacy clause dropped) | my call | GitLab's documented component form; today it is not a target |
| D27 | crane, systems as in §8.1, `homeModules` alias, install-nix-action + cachix-action, SHA-pinned actions, release created once | my call | §8 |
| D28 | Container on GHCR only, no Entrypoint, busybox + CA certs | my call | GitLab image requirements |
| D29 | `PIN_CI_LIVE`/`PIN_CI_ORACLE` names kept. No crates.io publish. No `unpin`. **Changed:** oracle kept until DF1, adapted to the new crate API; the owner decides retirement after dogfooding | my call; owner (2026-11-10) | scope control |
| D30 | A version **range** (H1 Chart.yaml/requirements, H4 helmfile) on a line that already has a trailing comment → `skipped (existing comment; pinning would lose constraint <c>)`. The constraint is never dropped. `--update` reads a recorded constraint only from a comment that is entirely `# <constraint>` and parses as one; any other comment counts as "no recorded constraint" (→ the image policy of D21). Exact versions and C3/C4 keep today's rule: pin, no second comment | my call (builder check) | never lose author intent; never misread an unrelated comment |
| D31 | `GITLAB_TOKEN` only to the primary host (`--gitlab-host` > `CI_SERVER_HOST` > gitlab.com); other `gitlab-hosts` anonymous unless bound in global `[hosts]` (§7.3) | my call (builder check) | least privilege; repo config cannot widen it |
| D32 | YAML discovery via the span-preserving `saphyr-parser` (S2: adopt with caveats); char→byte table, quoted-span re-scan, property skip, `{{ }}` masking (§6.9). Edits stay byte splices. Aliases/merge keys/`!reference` are not targets. Parse error → file error (as built: `ScanError` → `FileError::Parse`; exit 1 until M5d, then exit 2), no `discover.rs` fallback | owner (2026-11-10); parse-error rule my call | robust structure handling (flow, anchors, multi-doc) without giving up byte-exact edits; fixes R7–R9 |
| D33 | No compatibility layer: no `pin-ci` legacy mode, legacy exit codes or JSON, legacy goldens, A/B script or compat tag. Snapshot tests replace goldens. Byte-exact, idempotent edits stay hard invariants with property tests | owner (2026-11-10) | never used in the real world; no users |
| D34 | Order: M5a–c → M5c-y → M5d → M5f → M5e' → M6 → DF1 → G1 → F3 → M4b-2 → M7 → M8a–c → M9 (§10) | owner (2026-11-10); M5c-y, M5f/M5e' placement my call | real use before Helm; M6's HM module needs the global layer |
| D35 | Smaller config first: M5d reads repo `.pin-toolkit.toml` + env + CLI; the global layer, `[hosts]` and `config show --origin` come in M5e'. D18 holds from M5d: a repo file never binds credentials (global-only keys → exit 2) | owner (2026-11-10) | dogfood sooner; no window in which a repo can bind a token |
| D36 | Resolvers (`oci`, `gitlab`, `helm-repo`, `github`) are pin-core cargo features, each built lazily on first use; plugin crates enable what they need; no extra crates (§6.2) | owner (2026-11-10) | `Ctx` is no god object; trimmed builds compile only the clients they use |
| D37 | `CI_JOB_TOKEN` bound to `CI_SERVER_HOST` only, in M5d (pulled from F3, which keeps only native TLS roots) | my call | security fix: today it can reach gitlab.com; must hold before DF1 runs in CI |
| D38 | Plugins resolve only via `Ctx` entry points (`oci_digest`, `oci_tags`, `oci_tag_times`, `gitlab_resolve`; helm-repo/github alike), never via a raw client; the clients are crate-private (§6.2) | my call (2026-11-10, M5a–c plan review) | memo (§6.7) and token binding (D31, D37) in one place; no plugin can bypass them |
| D39 | Routing (M5d T-D10): an explicit file with a plugin subcommand or alias is claimed regardless of globs; in all-plugins mode an explicit path claimed by 0 or 2+ enabled plugins is exit 2 before the run; a **walked** file claimed by 2+ plugins is a per-file error under the first claimant, not routed, the other files run, exit 2 | main (2026-11-10, M5d plan Open 3; walked part PR-L11) | one file = one plugin (D11); one ambiguous glob must not stop every run (e.g. `.gitlab/ci/values.yaml` under gitlab-ci and helm in M8) |
| D40 | GitLab primary host = `--gitlab-host` > `PIN_TOOLKIT_GITLAB_HOST` > `CI_SERVER_HOST` > gitlab.com; resolve set = {primary} ∪ `gitlab-hosts`; `CI_SERVER_HOST` is not added implicitly (M5d T-D12) | main (2026-11-10, Open 10) | D31 + D37; credentials follow only bound hosts |
| D41 | `ignore` keys: the value with quotes stripped for image/service/component, `{project}@{ref}` for includes; `*` (crosses `/`, may be empty) and `?` over the whole key; an ignored target has no row and no count (M5d T-D17) | main (2026-11-10, Open 4) | `registry/*` must match nested repos; ignoring is an explicit choice like a disabled kind (D16) |
| D42 | Human report: `plugin:kind` after `file:line`; the total summary line always, one line per plugin when more than one plugin ran, then `follow-up:` lines; per-file errors on stderr in both modes (M5d T-D21) | main (2026-11-10, Open 5, PR-L1) | §6.4 verbatim; errors on stdout would corrupt `--json` |
| D43 | Fatal errors (exit 2 before the run) print nothing on stdout, also with `--json` (M5d T-D22) | main (2026-11-10, Open 6) | a half-written report is worse than none; CI reads the exit code |
| D44 | CLI `--only`/`--skip` with a plugin subcommand or alias is exit 2; env `PIN_TOOLKIT_ONLY`/`PIN_TOOLKIT_SKIP` are ignored in subcommand/alias mode (M5d T-D28) | main (2026-11-10, Open 15, PR-L6) | ambiguous intent on the command line; a CI-wide env value must not break `pin-ci` |
| D45 | The §7.3 `unresolved (<host>: not trusted for credentials — bind it in …)` message is deferred to M5e'; in M5d an unbound host resolves anonymously with the plain `unresolved` reason (M5d T-D30) | main (2026-11-10, Open 17, PR-M6 option b) | the message names the global config, which exists only from M5e' |
| D46 | JSON `plugins` keyset = the enabled plugins (zero summary when a plugin found nothing); per-plugin `errors` sorted by `(file, line)` whatever their origin; the per-plugin human lines count only plugins that ran (routed files), not conflict- or walk-error-only entries (M5d review-2 L7) | main (2026-11-11) | a consumer can read `plugins["gitlab-ci"].summary` on an empty repo; deterministic output |
| D47 | An explicit directory outside the git root is matched relative to its own git root (else the directory itself); walk errors are per-file errors (exit 2); bad ignore-file lines only warn (M5d review-2 M1, L3) | main (2026-11-11) | `--check ../other` must not be a silent green; §6.3 treats read failures as per-file errors |

## 13. Owner answers

Open for the owner: retiring the oracle, decided after DF1 (D29).

| # | Question | Owner answer | Affects |
|---|---|---|---|
| O1 | What does "plugin disabled in Home Manager" mean? | **Not installed.** Enabling a plugin is a build decision, configuring it is runtime (§8.2). The owner runs with all plugins enabled, i.e. the cached default package; a subset is a local feature-trimmed build. A repo config can still stop a built plugin from running (`run = false`). **M5d: resolved as T-D11** (a repo table for an unbuilt plugin → notice, exit unaffected; an explicit request → exit 2; §8.2 as built) | M6 (HM module, package override), M5d (`run` key, "not in this build" handling), M5e' (global config) |
| O2 | values.yaml map image without a `digest:` key | **Always write `tag: "1.2.3@sha256:<d>"` by default**, opt-out `plugins.helm.values.tag-digest = false` (→ `skipped (no digest key)`). An existing `digest:` key is still filled instead (tag kept). Known risk, accepted: charts that use `.tag` in labels or `semverCompare` | M8a |
| O3 | Name, owner, Cachix, GitLab repo | Name **`pin-toolkit`**. Repo `github.com/<owner>/pin-toolkit` where `<owner>` is the owner's **personal GitHub user** (exact handle to be filled in at M5f), full history pushed. Cachix cache: not set up yet; use the recommended name **`pin-toolkit`** (`pin-toolkit.cachix.org`). The GitLab repo **stays** (mirror) and additionally **hosts the GitLab test components/projects** used by the live component/include tests. Human prerequisites for M6: create the GitHub repo, the Cachix cache + `CACHIX_AUTH_TOKEN` secret, GHCR push rights | M5f, M6, live GitLab tests |
| O4 | License | **`MIT OR Apache-2.0`** | M5f |
| O5 | (2026-11-10) Compatibility with pin-ci? | **None**: the tool has never been used in the real world. No legacy mode; goldens become snapshot tests; R1/R2 fixed in the restructure. Byte-exact edits stay a hard invariant. `pin-ci` only as a cheap plain alias, or dropped | §2, §6.5–6.6, §9, §11, D33 |
| O6 | (2026-11-10) YAML parser for discovery? | **Yes**, span-preserving, gated by a spike; edits stay splices. S2 result: adopt saphyr-parser with fix-ups | §6.9, D32 |
| O7 | (2026-11-10) Order and config scope | **Real use before Helm, smaller config first**: minimal binary → distribution → dogfood → G1/F3/M4b-2 → helm. Full layered config only as needed by the HM module | §7.1, §9, §10, D34, D35 |
| O8 | (2026-11-10) `Ctx` shape | **Lazy, feature-gated resolvers in pin-core**, no extra crates | §6.2, D36 |

## 14. Left to the builder

- Library picks: `ignore`/`globset` for walking, a hand-rolled `toml::Value` deep merge vs `figment` (no release since 2024-05), how the memo coalesces in-flight lookups, clap layout for subcommands plus argv[0].
- Exact toolkit human-output spacing, and the `config show --origin` format. Optional JSON Schema for the config.
- Refining the default globs. Per-kind skip-reason wording (stable once released).
- Verify before coding: helm/helm#32133 status. The first helmfile release containing PR #2398 (may flip D23). GitLab `include:integrity`. Whether Helm's registry login falls back to `~/.docker/config.json`.
- Helm repo credential env naming (the global config binds URL → env var names; `repositories.yaml` is matched by URL).
- The shell tokenizer for C5: quotes, `\` continuations, `&&`, `;`, `|`, `X=1 helm`, `--flag=value`, value-taking vs boolean helm flags. Tokenizer offsets map back to bytes only where the raw scalar equals its decoded value (plain, or quoted without escapes); otherwise `skipped`.
- Snapshot tooling (`insta` suggested) and the redaction of mock-port URLs.
- Whether the `pin-ci` alias is kept (§6.6). The lazy-init primitive (`OnceCell` suffices).
- Filing the saphyr-parser quoted-span bug upstream; drop the re-scan once a fixed release is pinned.
- Whether `devShells.default` replaces devenv. Do not remove devenv without asking.
- Hub API rate-limit handling (backoff or a clear `unresolved`).

### 14.1 Carry-overs from the code reviews (M5a–c: `.superpowers/sdd/2026-11-10-m5abc/code-review.md`; M5c-y below)

- **Before M8 (M1):** `run::pin_file` checks overlap only between findings. Validate each finding's own splices (start ≤ end, no self-overlap) and mark the finding `unresolved` instead of panicking in `edit::apply` or corrupting the file.
- **Done in M5d (M3):** D38 enforced: the raw clients and their constructors are `#[doc(hidden)] pub` only under pin-core `test-support` (fadf5c3), checked by `scripts/gate.sh` (761c591, review-2 L8); `RegError: Clone` for the memo (L4, 4ffff5f).
- **Done in M5c-y:** R1 combined comment in byte order, not scan order (L1, 3a329b1; `run::ordered_splices` orders the splices by `(line, first edit offset)`, rows keep finding order); the missing overlap and lazy-resolver regression tests and the span-based byte-exact check in `tests/common/mod.rs` (L7, d5f4d58 and 3a329b1; `check_spans` ties every replacement to exactly one discovered span and checks the comment in byte order).
- **Done in M5d (L3):** one `LineIndex` per file via `plugin::Source` in `Plugin::resolve` (7217e37).
- **Done in M5d (low):** edits of non-pinned findings are never written (L2, 92c4051); fallible `try_new` client constructors, construction errors cached (L5, 4ffff5f); no `PartialEq<&str>` on `Status` (L6, 968c3ae); one GitLab host set, `GitlabCi` a unit struct (L9, aa2a0c5); kind-id constants and no `unreachable!` in `policy.rs` (L10, N5, cdc717e). L8 (the combined comment on a partially pinned flow line lists only the pinned refs) is documented in the README ("What gets skipped").
- Done before merge (M2): public `Ctx::env()` removed (89f7ffe).
- **From the M5c-y review (`.superpowers/sdd/2026-11-10-m5cy/code-review.md`), L3, for M8a:** an implicit-null span is a marker, not an insertion point (block: before the `:`; flow: at the next token). **Never splice at an empty span.** H3 fills an existing `digest:` key; if that key is null (`digest:` with no value), M8a needs an insertion position (e.g. the byte after the key's `:`) as an extra field on `ScalarRef`. `digest: ""` and `digest: ~` already work.
- **For M8a/M8c (L3, masking limits):** `mask_templates` pairs a `{{` inside a YAML **comment** with the next `}}`, even lines later: `# a {{ comment\nimage: {{ .x }}\nb: c` loses `image`. A `"}}"` inside a template string ends the mask early, which gives a parse error. Known limitations; handle in M8a (mask comments out first, or tokenise the template) or document them.
- The rest of the M5c-y review fits M8 as is: `parent` groups `image: {repository, tag, digest}` and Chart.yaml `dependencies[*]`; `doc` and per-document paths handle multi-document values; `Tagged`/`Alias` are not targets; `templated` exists (M1 fixed before anyone consumed it). The `policy.rs` `unreachable!` nit is fixed in M5d (N5, cdc717e): callers pass the destructured fields.
- **Done in M5d (review `.superpowers/sdd/2026-11-10-m5d/code-review-1.md`):** registry bearer tokens keyed by `(host, realm, service, scope)`, never replayed to another host (H1, 7d28c82); the repo-only `RegistryClient` tag-list cache removed, the per-run memo is the one cache (M1, 750ccc9); L1–L6 and N2 too (`.superpowers/sdd/2026-11-10-m5d/fix-review-1.md`).
- **M5d review N1, open:** `Selection` derives `Default` = empty `KindSet` = scan nothing, so a `..Default::default()` would silently disable every kind; drop `Default` (keep `From<KindSet>`).
- **M5d review N3, documented (README, `ignore`):** an `Unsupported` include ref's ignore key is just the ref, so `ignore = ["g/p@*"]` still leaves its `skipped (block scalar)` row; carrying `project` on `Unsupported` stays an option.
- **Done in M5d (review `.superpowers/sdd/2026-11-10-m5d/code-review-2.md`, log `impl-review2-t26-t31.md`):** M1 and M2 walk fixes (30fc42c), M3 plan amendment (dbd39ec), L1 + N3 empty/trimmed id lists (99aa62d), L2 globs of every plugin table (b20b918), L3 walk errors (e7e83cb), L4 usage says `pin-toolkit` (045b4d2), L5 tests (53bdf23), L6 host syntax (f637f21), L7 `plugins` keyset (d7aa4cc, D46), L8 `scripts/gate.sh` (761c591), N1 one display rule (11f321d).
- **M5d review-2 N2, open:** `#[allow(dead_code)]` on the whole `pub(crate)` modules without `test-support` (`crates/pin-core/src/lib.rs:8`, `crates/pin-core/src/oci/mod.rs:11`) also hides genuinely dead production code; narrow it to the test-only items (`Creds::load_from`, …).
- **M5d review-2 N4, open:** `enable::Plan.plugins: Vec<(&dyn DynPlugin, Selection, GlobSet)>` should become a named struct once per-plugin data grows (M7+).
