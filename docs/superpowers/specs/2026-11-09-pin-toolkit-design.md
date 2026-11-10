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
| C2 | `service` | `services:` items (string or `name:`) | OCI | same as C1 | same | exists. Reported as `image` until M5d adds the `service` kind tag |
| C3 | `component` | top-level `component:` line | GL | `gitlab.com/g/p/name@<sha40>  # 1.0.0` | catalog-managed (`~latest`, `1.0.x`, partial `1`/`1.0`) → skipped. Host ≠ configured host → skipped | exists |
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

### 6.3 One-pass run

1. Load the config (§7) and build `Ctx`: env and host bindings, an empty memo, `now`. No resolver client is built yet (§6.2).
2. Inputs: explicit files and directories, or the git root (falling back to cwd). Walk with `.gitignore` respected (the `ignore` crate), match each enabled plugin's `files` globs, sort paths.
3. Per file, sequentially as today: read once. Run `plan` (scan → resolve, with at most `--concurrency` in flight). Stable-sort by line. Check for overlaps. Unless `--dry-run`/`--check`, apply the edits and write only if there is at least one splice.
4. Merge rows, per-plugin summaries, follow-ups and per-file errors into one `Report`. Render it and exit (§6.5).

A file that fails to read, parse (§6.9) or write is recorded under `plugins.<id>.errors`. The other files and plugins continue, and the exit code is 2.

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

### 6.5 Exit codes

| Code | `pin-toolkit` (and every alias) |
|---|---|
| 0 | no `unresolved`, no error. With `--check`: also nothing would change |
| 1 | any `unresolved`. With `--check`: also any row that would be `pinned`/`updated` |
| 2 | fatal: usage or config error, a global-only key in the repo config, IO or YAML parse error in a file (§6.9), a plugin that is enabled but not compiled in, an explicit path claimed by 0 or 2+ plugins |

`--check` = `--dry-run` plus the exit-1 rule above. Its intended use is a CI gate. From M4b-2 it
also verifies that a `# <ref>` comment still resolves to the pinned SHA, like pinact's verify.

### 6.6 Front-ends

- `pin-toolkit [OPTIONS] [PATHS]…` runs all enabled plugins. `pin-toolkit <plugin> …` runs one plugin, as does the argv[0] alias `pin-<plugin>`. Also `pin-toolkit plugins` (lists compiled-in plugins, kinds, effective enable state) and, from M5e', `pin-toolkit config show [--origin]`.
- Options: `--dry-run --check --json --update --update-major --min-age-days N --concurrency N --only/--skip <p,…> --kind <p>:<k>=on|off --config FILE --no-config --gitlab-host H`.
- **No `pin-ci` legacy mode** (D33). M5d deletes the crate's `[[bin]] pin-ci`. `pin-ci` may remain only as a plain argv[0] alias of `pin-toolkit gitlab-ci` (toolkit flags, config, output and exit codes) if it costs no more than one dispatch entry and one symlink; otherwise it is dropped. The README says which.

### 6.7 Resolver cache

`Ctx.memo` is in-memory and lasts for one run. It is keyed by `(host, repo, tag)` for digests,
`(host, repo)` for tag lists and timestamps, `(host, project, ref)` for GitLab, the URL for helm
indexes, and `(owner/repo, ref)` for GitHub. Results **including errors** are cached, so one ref
cannot get two outcomes in a run. Concurrent identical lookups are coalesced. There is **no disk
cache**: freshness is the product. Revisit only if the Hub API limit (180 req/h per IP, used by the
M4b-1 age guard) bites. Existing per-client token caches stay.

### 6.8 Release age

`--min-age-days` (M4b-1, docker.io only via the Hub API) generalises to every resolver that has
timestamps: Hub API tags, `index.yaml` `created`, the OCI `org.opencontainers.image.created`
annotation, GitLab tag dates (M4b-2), GitHub (M9). When the age is unknown, `release_age.unknown =
"allow"` is the default. That matches today's documented no-op for non-docker.io hosts, and
`"block"` is the alternative. The default stays 0 (off) and 14 is documented as the recommended value.

### 6.9 YAML discovery (D32)

Discovery (finding targets and their byte spans) uses the span-preserving event parser
`saphyr-parser` 0.3.x (MIT OR Apache-2.0, MSRV 1.85, +2 crates: saphyr-parser, arraydeque). Edits
stay byte splices on those spans (§6.2); the parser never writes. Spike S2 (2026-11-10) reproduced
all 37 `discover()` targets of `tests/fixtures/*.yml` and `ci/.gitlab-ci.yml` at identical bytes,
given the fix-ups below. Rejected: `yaml-rust2`, `marked-yaml` (start marker only, no end).

`core::yaml` = parser + a ~40-line hand-rolled fix-up layer:

| Parser fact (S2, saphyr-parser 0.3.3) | `core::yaml` handling |
|---|---|
| `Marker::index()` counts **chars**; a BOM counts as char 0; `col()` on line 1 excludes the BOM; `ScanError` says "byte N" but N is a char index | one char→byte table per file (`char_indices` + `len`); use `index()` only, never `col()` |
| quoted-scalar span end overshoots into trailing spaces and `# comment` (in flow also a space before `]`/`,`). Upstream bug in `scan_flow_scalar`; the start is correct | re-scan from the start to the closing quote (skip `''` and `\x` escapes) |
| scalar spans include node properties `&anchor `, `!tag ` | skip the properties |
| no comments in events; anchor names absent (numeric ids); merge keys `<<:` not expanded | trailing-comment detection stays hand-rolled (`edit::trailing_comment`). Aliases (`Event::Alias`), merge keys and `!reference […]` are **not targets**: the bytes live at the definition |
| after the first `ScanError` the iterator repeats it (sticky); unquoted `{{ … }}` mis-parses as flow mappings **without** an error | templated files (helm values, `.gotmpl`): mask every `{{…}}` with equal-length spaces before parsing (offsets unchanged); a scalar overlapping a mask is `skipped (templated)` |
| block scalars (`\|`, `>`) span the content only, header excluded | never spliced: `skipped (block scalar)` at a target path |
| CRLF: `\r` is one char, excluded from spans; multi-document spans are file-global | none |

Shape (S2 proposal): `scan(text, ScanOpts { mask_templates }) -> Result<Vec<Node>, YamlError>`,
`Node::{Scalar(ScalarRef), Alias { path, id, span }, Tagged { path, tag }}`,
`ScalarRef { path, doc, span /* byte range incl. quotes */, style, value /* decoded */, anchor, templated }`.
Plugins match path patterns over `Vec<Node>` (`image`, `services[*].name`, `include[*].ref`,
`dependencies[*].version`, `**.image.{repository,tag,digest}`), so `scan` stays pure.

A file that does not parse (after masking) is a per-file error under `plugins.<id>.errors` (exit 2)
with the line from `YamlError`: no partial targets, no second hand-rolled discoverer (my call;
revisit in DF1). Hand-rolled code stays only for the fix-ups, template masking, trailing-comment
detection, `requote` and the C5 shell tokenizer. Behaviour change versus `discover.rs` (a bug it
fixes, reviewed as a snapshot diff): `image: &img "ubuntu:24.04"` no longer yields a value containing
`&img`, and `image: *img` is no longer a target.

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

### 7.2 Where settings may live

| Setting | Global (HM) | Repo | Env / CLI |
|---|---|---|---|
| `plugins.<p>.run`, `kinds`, `files`, `ignore`, `update`, `release_age`, plugin options | default | ✓ overrides, so CI and local runs agree | ✓ |
| `concurrency`, output format | ✓ | — | ✓ |
| **`[hosts]`**: GitLab hosts that may receive tokens, which env var holds each token, helm repo URL → credential env names, CA file, registry aliases/mirrors | **✓ only** | ✗ (exit 2) | ✓ (`--gitlab-host`, `CI_SERVER_HOST`) |
| secrets | never | never | env, `~/.docker/config.json`/`$DOCKER_CONFIG`, `$HELM_REGISTRY_CONFIG`, `$HELM_REPOSITORY_CONFIG` |

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
| M5c-y | Own step right after M5c (keeps M5c manageable): `core::yaml` = saphyr-parser + fix-ups (§6.9); gitlab-ci discovery becomes path matching over `Vec<Node>`; the `discover.rs` YAML primitives are deleted. Fixes the anchor/alias bug | discovery-parity test (§11); snapshots unchanged except the reviewed anchor/alias diff |
| M5d | Minimal `pin-toolkit` binary, replaces `pin-ci`: walk, routing, `pin-toolkit gitlab-ci` and `pin-<plugin>` aliases (optional `pin-ci` alias, §6.6), `service` kind tag, combined report, exit codes 0/1/2, `--check`, memo. Config: repo `.pin-toolkit.toml` + env + CLI only (§7.1, D35). `CI_JOB_TOKEN` bound to `CI_SERVER_HOST` only (D37). Delete the crate's `[[bin]] pin-ci` | toolkit snapshots (human + JSON) over the M5a fixtures, diffs reviewed; exit-code table tests; repo `[hosts]` → exit 2; wiremock: `CI_JOB_TOKEN` never sent to another host |
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
| R7 | `discover()` keeps `&img` in an anchored value and treats the alias `*img` as a target (found by S2) | M5c-y: fixed by parser discovery (§6.9) |
| R8 | `discover()` misses block children in two cases (found by the M5a fixture run): under CRLF a `key:\r` line counts as a key with a value, so `include:`/`services:` blocks and every job-nested `image:` are not targets; children of an anchored key (`.base: &base`) are never targets | not fixed in M5c (the hand-rolled code is deleted by M5c-y); recorded in the M5a snapshots (`ci-r2-crlf`, `ci-anchors`); M5c-y: fixed by parser discovery, listed with R7 as a documented parity exception |

## 10. Milestones

S ≈ 1–2 tasks, M ≈ 3–4, L ≈ 5+. Each milestone gets its own plan in `docs/superpowers/plans/`.

| # | Milestone | Size | Behaviour change |
|---|---|---|---|
| M4b-1 | done (0e2047b): release-age guard, `Default` for `PinOptions`, oracle + README | M | planned feature |
| S2 | YAML discovery spike (done, §6.9) | S | none |
| M5a–c | snapshots → workspace move → core split + lazy resolvers + R1/R2 fixes (§9) | S + S + L | R1/R2 fixed (reviewed snapshot diffs) |
| M5c-y | gitlab-ci discovery on `core::yaml` (saphyr-parser + fix-ups) | M | R7 fixed (reviewed snapshot diff) |
| M5d | minimal `pin-toolkit` binary (replaces `pin-ci`); repo config + env + CLI; `CI_JOB_TOKEN` → `CI_SERVER_HOST` only | M | new binary; `pin-ci` gone or a plain alias |
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
| Discovery parity (M5c-y) | ∀ fixture: parser discovery yields the `discover.rs` targets except the documented R7 cases, and ∀ target `text[span]` equals the token incl. quotes. Fixtures cover non-ASCII before a target, BOM, CRLF, quoted value + `# comment`, `&anchor`/`!tag`, aliases, `<<:`, multi-document, masked `{{ }}` |
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
| D32 | YAML discovery via the span-preserving `saphyr-parser` (S2: adopt with caveats); char→byte table, quoted-span re-scan, property skip, `{{ }}` masking (§6.9). Edits stay byte splices. Aliases/merge keys/`!reference` are not targets. Parse error → file error, exit 2, no `discover.rs` fallback | owner (2026-11-10); parse-error rule my call | robust structure handling (flow, anchors, multi-doc) without giving up byte-exact edits; fixes R7 |
| D33 | No compatibility layer: no `pin-ci` legacy mode, legacy exit codes or JSON, legacy goldens, A/B script or compat tag. Snapshot tests replace goldens. Byte-exact, idempotent edits stay hard invariants with property tests | owner (2026-11-10) | never used in the real world; no users |
| D34 | Order: M5a–c → M5c-y → M5d → M5f → M5e' → M6 → DF1 → G1 → F3 → M4b-2 → M7 → M8a–c → M9 (§10) | owner (2026-11-10); M5c-y, M5f/M5e' placement my call | real use before Helm; M6's HM module needs the global layer |
| D35 | Smaller config first: M5d reads repo `.pin-toolkit.toml` + env + CLI; the global layer, `[hosts]` and `config show --origin` come in M5e'. D18 holds from M5d: a repo file never binds credentials (global-only keys → exit 2) | owner (2026-11-10) | dogfood sooner; no window in which a repo can bind a token |
| D36 | Resolvers (`oci`, `gitlab`, `helm-repo`, `github`) are pin-core cargo features, each built lazily on first use; plugin crates enable what they need; no extra crates (§6.2) | owner (2026-11-10) | `Ctx` is no god object; trimmed builds compile only the clients they use |
| D37 | `CI_JOB_TOKEN` bound to `CI_SERVER_HOST` only, in M5d (pulled from F3, which keeps only native TLS roots) | my call | security fix: today it can reach gitlab.com; must hold before DF1 runs in CI |
| D38 | Plugins resolve only via `Ctx` entry points (`oci_digest`, `oci_tags`, `oci_tag_times`, `gitlab_resolve`; helm-repo/github alike), never via a raw client; the clients are crate-private (§6.2) | my call (2026-11-10, M5a–c plan review) | memo (§6.7) and token binding (D31, D37) in one place; no plugin can bypass them |

## 13. Owner answers

Open for the owner: retiring the oracle, decided after DF1 (D29).

| # | Question | Owner answer | Affects |
|---|---|---|---|
| O1 | What does "plugin disabled in Home Manager" mean? | **Not installed.** Enabling a plugin is a build decision, configuring it is runtime (§8.2). The owner runs with all plugins enabled, i.e. the cached default package; a subset is a local feature-trimmed build. A repo config can still stop a built plugin from running (`run = false`) | M6 (HM module, package override), M5d (`run` key, "not in this build" handling), M5e' (global config) |
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

### 14.1 Carry-overs from the M5a–c code review (`.superpowers/sdd/2026-11-10-m5abc/code-review.md`)

- **Before M8 (M1):** `run::pin_file` checks overlap only between findings. Validate each finding's own splices (start ≤ end, no self-overlap) and mark the finding `unresolved` instead of panicking in `edit::apply` or corrupting the file.
- **In M5d (M3):** enforce D38. The raw `RegistryClient`/`GitLabHosts` and their constructors are still `pub`; make them crate-private (or `#[doc(hidden)]` behind a test-support feature) so plugins cannot bypass the memo and token binding. `RegError: Clone` for the memo (L4).
- **M5c-y or M5d (low):** R1 combined comment in byte order, not scan order (L1); skip writing edits of non-pinned findings (L2); one `LineIndex` per file in `trailing_comment` (L3); unreachable `.expect` in the resolver construction (L5); `PartialEq<&str>` on `Status` out of the public API (L6); missing tests and the span-based byte-exact check in `tests/common/mod.rs` (L7); single GitLab host set, kind-id constants (L8–L10).
- Done before merge (M2): public `Ctx::env()` removed (89f7ffe).
