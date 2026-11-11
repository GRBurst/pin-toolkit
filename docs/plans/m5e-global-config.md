# Plan: m5e-global-config
<!-- task-router plan v1. Edit only checkboxes; change the plan with amend_plan. -->
Request: Implement M5e' of docs/superpowers/specs/2026-11-09-pin-toolkit-design.md (S10 M5e' row, S7.1-S7.3, D45, S14.1). Full design with all decisions is in .superpowers/sdd/2026-11-11-m5e/design.md — READ IT FIRST. Scope: (1) global config layer ($XDG_CONFIG_HOME|$HOME/.config)/pin-toolkit/config.toml, version=1 required, keys json/concurrency/[hosts], folded defaults<global<repo<env<CLI, --no-config skips it, --config stays repo-only; (2) [hosts] bindings per S7.3 (gitlab token-env / helm-repo username+password-env / oci ca-file; env-var NAMES only, never secrets; host keys via valid_gitlab_host; only the gitlab kind is wired into resolution in M5e'); repo [hosts] stays exit 2; (3) GITLAB_URI in primary-host precedence per owner 2026-11-11: --gitlab-host > PIN_TOOLKIT_GITLAB_HOST > GITLAB_URI > CI_SERVER_HOST > gitlab.com; URL parsed to host[:port]; invalid = exit 2; never grants CI_JOB_TOKEN (D37); (4) D45 message `unresolved (<host>: not trusted for credentials — bind it in <global path>)` for failed lookups on unbound uncredentialed non-primary hosts, composed in pin-core; (5) `pin-toolkit config show [--origin]` subcommand (format in design S5); (6) out-of-root config documented, no code change; (7) nits: drop Selection: Default (keep From<KindSet>), narrow pin-core module-wide allow(dead_code); (8) README + spec close-out. Branches M1-M7 per design S8, each within the 150-line/6-file merge limit.
Invariants:
  - Repo [hosts]/concurrency stay exit 2: repo_hosts_exits_2 (config_layers.rs) and exit_codes row 9 pass unchanged; repo.rs GLOBAL_ONLY unchanged
  - Config files hold env-var NAMES only; no token value is ever printed, logged or shown by Debug/config show
  - A token reaches only its bound host: GITLAB_TOKEN->primary, CI_JOB_TOKEN->CI_SERVER_HOST only (D37), [hosts] token->its own host; env bindings win over [hosts]
  - No process-env reads below the binary edge: config/global paths come from Process.env; tests pass explicit env maps (XDG_CONFIG_HOME/HOME under the scratch repo)
  - Existing snapshots unchanged (INSTA_UPDATE=no); pin-gitlab-ci policy.rs unchanged; gitlab-hosts decides what resolves, never grants credentials
  - bash scripts/gate.sh passes on every branch (tests, fmt, clippy -D warnings, no-default-features, D38, rustdoc)
  - Files created by an earlier task (global.rs, show.rs) are edited, not recreated, by later tasks listing them in create

## M1: Parse the global config file: config/global.rs with version=1 required, json, concurrency and [hosts] bindings (gitlab/helm-repo/oci shapes), deny_unknown_fields, host keys via valid_gitlab_host; Layer gains hosts. (branch: milestone; depends: —)
Acceptance: global::parse returns a Layer (json, concurrency, hosts) or a Fatal naming the file; missing/!=1 version, unknown key, bad TOML, bad host key, wrong per-kind fields -> Fatal; Layer.hosts: Option<BTreeMap<String, HostBinding>>, replace-merged like files
- [ ] M1.T1: Create pub mod config::global: parse(display:&Path, text:&str)->Result<Layer,Fatal> for version (required, ==1), json, concurrency with deny_unknown_fields; messages name the path like repo.rs. Unit tests config::global::tests (version missing/2, unknown key, json+concurrency set).
  files: crates/pin-toolkit/src/config/mod.rs, crates/pin-toolkit/src/config/global.rs (new) · forbidden: changing repo.rs or GLOBAL_ONLY; reading the filesystem or process env in global.rs (pure parse) · check: `cargo test -p pin-toolkit --lib config::global 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: —
- [ ] M1.T2: global.rs (from M1.T1) [hosts]: HostBinding (gitlab: token-env only; helm-repo: username-env+password-env; oci: ca-file; missing/extra field -> Fatal); host keys must pass valid_gitlab_host; Layer.hosts + replace-merge in mod.rs. Tests config::global::tests::hosts_*.
  files: crates/pin-toolkit/src/config/mod.rs, crates/pin-toolkit/src/config/global.rs (new) · forbidden: storing or reading secret values (env-var names only); wiring hosts into resolution here · check: `cargo test -p pin-toolkit --lib config::global::tests::hosts 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M1.T1

## M2: Fold the global layer: global_path from env ($XDG_CONFIG_HOME non-empty absolute, else $HOME/.config)/pin-toolkit/config.toml; load() = [global?, repo?, env, cli]; --no-config skips global and repo; Effective.hosts. (branch: milestone; depends: M1)
Acceptance: precedence defaults<global<repo<env<CLI; fold test covers 5 layers incl. hosts; --config/PIN_TOOLKIT_CONFIG override only the repo path; global still applies; missing global file = no layer; unreadable/bad -> exit 2 naming the path
- [ ] M2.T1: mod.rs: pub fn global_path(env)->Option<PathBuf>; Effective.hosts (empty default) from fold; fold_scalars_last_wins over 5 layers [defaults,global,repo,env,cli] incl. hosts. Unit tests config::tests::global_path_* (XDG over HOME, empty/relative XDG ignored, none).
  files: crates/pin-toolkit/src/config/mod.rs · forbidden: std::env reads; changing load() in this task · check: `cargo test -p pin-toolkit --lib config::tests::global_path 2>&1 | grep -qE 'test result: ok\. [1-9]' && cargo test -p pin-toolkit --lib config::tests::fold 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M1.T2
- [ ] M2.T2: load(): global layer first (global::parse) when the file exists; --no-config skips it; read errors -> Fatal naming the path (cwd-relative where possible). config_layers.rs tests global_* (global<repo<env<CLI, --no-config, --config keeps global, bad global exit 2).
  files: crates/pin-toolkit/src/config/mod.rs, crates/pin-toolkit/tests/config_layers.rs · forbidden: changing repo_hosts_exits_2 or exit_codes.rs; letting --config/PIN_TOOLKIT_CONFIG replace the global path · check: `cargo test -p pin-toolkit --test config_layers 2>&1 | grep -qE 'test result: ok\. [0-9]+ passed; 0 failed' && cargo test -p pin-toolkit --test config_layers global_ 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M2.T1

## M3: pin-core: GitlabEnv.bound (host->token value) used by auth_for after the env bindings; GITLAB_URI in primary precedence with URL->host[:port] parse; D45 not-trusted message composed in Ctx::gitlab_resolve. (branch: task; depends: —)
Acceptance: auth_for: existing GITLAB_TOKEN/CI_JOB_TOKEN checks first, then bound -> Private, else Anonymous; Debug redacts bound values; primary = arg > GITLAB_URI > CI_SERVER_HOST > gitlab.com; invalid GITLAB_URI -> Err("GITLAB_URI: ..."); never grants CI_JOB_TOKEN; failed lookup on non-primary Anonymous host -> `<host>: not trusted for credentials — bind it in <path>`; others unchanged
- [ ] M3.T1: run.rs: GitlabEnv gains pub bound: BTreeMap<String,String> (Default empty, Debug redacts values); auth_for step 3 = bound.get(host) -> Private. Unit test auth_for_bound_host; wiremock bound_token_only_reaches_bound_host in gitlab_hosts.rs.
  files: crates/pin-core/src/run.rs, crates/pin-core/tests/gitlab_hosts.rs · forbidden: reordering the existing GITLAB_TOKEN/CI_JOB_TOKEN checks; changing pin-gitlab-ci · check: `cargo test -p pin-core --all-features --lib auth_for_bound 2>&1 | grep -qE 'test result: ok\. [1-9]' && cargo test -p pin-core --all-features --test gitlab_hosts bound_token 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: —
- [ ] M3.T2: GitlabEnv::from_vars reads GITLAB_URI (empty=unset): strip http(s)://, cut path/trailing slash, validate with valid_gitlab_host, else Err("GITLAB_URI: invalid GitLab host `v`"); primary = arg > GITLAB_URI > CI_SERVER_HOST > gitlab.com. Unit tests gitlab_uri_*.
  files: crates/pin-core/src/run.rs · forbidden: binding CI_JOB_TOKEN to the GITLAB_URI host; falling back to gitlab.com on an invalid GITLAB_URI · check: `cargo test -p pin-core --all-features --lib gitlab_uri 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M3.T1
- [ ] M3.T3: GitlabEnv.global_config: Option<String>; gitlab_resolve maps a client error on host!=primary with Anonymous auth to `{host}: not trusted for credentials — bind it in {path}` (None -> ~/.config/pin-toolkit/config.toml). Wiremock tests unbound_host_failure_*.
  files: crates/pin-core/src/run.rs, crates/pin-core/tests/gitlab_hosts.rs · forbidden: changing pin-gitlab-ci policy.rs; changing primary-host or bound-host error texts · check: `cargo test -p pin-core --all-features --test gitlab_hosts unbound_host_failure 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M3.T1

## M4: Toolkit wiring: fill GitlabEnv.bound from Effective.hosts (kind gitlab, named env var non-empty), pass the global config path for D45, GITLAB_URI end-to-end tests and --gitlab-host help text. (branch: task; depends: M2, M3)
Acceptance: wiremock: bound token sent as PRIVATE-TOKEN only to the bound host; unbound hosts anonymous; GITLAB_URI primary end-to-end; invalid GITLAB_URI exits 2 before any request; no JOB-TOKEN to its host; unbound non-primary failure row reads `unresolved (<host>: not trusted for credentials — bind it in <global path>)`
- [ ] M4.T1: lib.rs: after ResolverEnv::from_vars fill renv.gitlab.bound from eff.hosts (kind gitlab, p.env[token-env] non-empty); no hosts added to resolve set. credentials.rs bound_token_* (global file via XDG_CONFIG_HOME; A gets token, B none; empty var = anonymous).
  files: crates/pin-toolkit/src/lib.rs, crates/pin-toolkit/tests/credentials.rs · forbidden: adding bound hosts to gitlab-hosts/resolve set; reading std::env · check: `cargo test -p pin-toolkit --test credentials bound_token 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M3.T1, M2.T2
- [ ] M4.T2: credentials.rs gitlab_uri_*: GITLAB_URI=https://gitlab.example.com/ is primary (GITLAB_TOKEN there), CI_JOB_TOKEN never sent to it without CI_SERVER_HOST, --gitlab-host beats it, invalid value exits 2, no request. cli.rs --gitlab-host help names GITLAB_URI.
  files: crates/pin-toolkit/tests/credentials.rs, crates/pin-toolkit/src/cli.rs · forbidden: changing existing credentials tests' assertions · check: `cargo test -p pin-toolkit --test credentials gitlab_uri 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M3.T2, M4.T1
- [ ] M4.T3: lib.rs sets renv.gitlab.global_config from global_path(&p.env) (also with --no-config). credentials.rs untrusted_host_*: unbound non-primary 404 -> `unresolved (gitlab.com: not trusted for credentials — bind it in <xdg>/pin-toolkit/config.toml)`; primary keeps text.
  files: crates/pin-toolkit/src/lib.rs, crates/pin-toolkit/tests/credentials.rs · forbidden: composing the message in pin-toolkit or pin-gitlab-ci; updating snapshots · check: `cargo test -p pin-toolkit --test credentials untrusted_host 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M3.T3, M4.T1

## M5: `pin-toolkit config show`: load_sources (layers with Origin, file displays), pub mod show renderer (dotted keys, design S5 format), Cmd::Config/ConfigCmd::Show{origin}/Mode::Config, run_mode branch after ResolverEnv::from_vars. (branch: task; depends: M2, M3)
Acceptance: prints config.global-file/repo-file, update, release-age, concurrency, json, gitlab-host, plugins.<id>.*, hosts.<host>.* lines; bad config or invalid CI_SERVER_HOST/GITLAB_URI exits 2; --only/--skip with config exits 2; env ONLY/SKIP ignored; never prints token values; `./config` is a path
- [ ] M5.T1: mod.rs: pub enum Origin {Default,Global,Repo,Env,Cli} (lowercase Display); pub struct Loaded {eff, global_file, repo_file: Option<String>, layers: Vec<(Origin,Layer)>}; pub fn load_sources, load() delegates. Unit tests config::tests::load_sources_*.
  files: crates/pin-toolkit/src/config/mod.rs · forbidden: changing Layer fields for provenance; changing load() semantics · check: `cargo test -p pin-toolkit --lib config::tests::load_sources 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M2.T2
- [ ] M5.T2: Create pub mod show (declared in lib.rs; also rewrite lib.rs `forced` match as `if let Mode::Plugin(id)`): render(&Loaded, primary)->String for config.*, update.*, release-age.*, concurrency, json, gitlab-host, hosts.* lines. Tests show::tests::renders_*.
  files: crates/pin-toolkit/src/lib.rs, crates/pin-toolkit/src/show.rs (new) · forbidden: printing env-var values; snapshot files for config show · check: `cargo test -p pin-toolkit --lib show::tests::renders 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M5.T1
- [ ] M5.T3: show.rs (from M5.T2): plugins.<id>.{run,files,ignore,kinds.<k>,gitlab-hosts} for compiled-in plugins with defaults filled as enable.rs computes them (expose a helper there if needed), plus set keys of configured unbuilt tables. Tests show::tests::plugins_*.
  files: crates/pin-toolkit/src/enable.rs, crates/pin-toolkit/src/show.rs (new) · forbidden: changing enable.rs semantics · check: `cargo test -p pin-toolkit --lib show::tests::plugins 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M5.T2
- [ ] M5.T4: cli.rs: Cmd::Config{cmd:ConfigCmd}, ConfigCmd::Show{--origin}, Mode::Config{origin}; fix Cli::paths/mode; enable.rs arm for Mode::Config. Unit tests cli::tests::config_show_* (mode parse, `./config` path, `config` after a path is a path).
  files: crates/pin-toolkit/src/cli.rs, crates/pin-toolkit/src/enable.rs · forbidden: args_conflicts_with_subcommands; changing existing subcommand parsing · check: `cargo test -p pin-toolkit --lib cli::tests::config_show 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M5.T3
- [ ] M5.T5: lib.rs run_mode: Mode::Config branch after ResolverEnv::from_vars prints show::render (primary = renv.gitlab.primary); --only/--skip CLI with it -> exit 2. tests/cli.rs config_show_prints_* (inline asserts: defaults, global+repo values, bad config exit 2, token value absent).
  files: crates/pin-toolkit/src/lib.rs, crates/pin-toolkit/tests/cli.rs · forbidden: insta snapshots for config show; walking files or building resolvers in config show · check: `cargo test -p pin-toolkit --test cli config_show_prints 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M5.T3, M5.T4

## M6: `config show --origin`: per-line ` # <origin>` (highest layer that set it, else default; ignore lists all contributing layers highest-first joined by +). (branch: task; depends: M5)
Acceptance: each line ends ` # default|global|repo|env|cli`; ignore e.g. `# global+repo`; gitlab-host from GITLAB_URI/CI_SERVER_HOST shows env; computed from Loaded.layers, no Layer change
- [ ] M6.T1: config/mod.rs: pub fn origin_of(layers:&[(Origin,Layer)], set:impl Fn(&Layer)->bool)->Origin and origins_of(..)->Vec<Origin> (highest first, for ignore). Unit tests config::tests::origin_*.
  files: crates/pin-toolkit/src/config/mod.rs · forbidden: adding provenance fields to Layer · check: `cargo test -p pin-toolkit --lib config::tests::origin 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M5.T1
- [ ] M6.T2: show.rs (from M5): origin flag appends ` # <origin>` to every line via origin_of/origins_of (ignore joined with +); lib.rs passes Mode::Config{origin}. Unit tests show::tests::origin_*.
  files: crates/pin-toolkit/src/lib.rs, crates/pin-toolkit/src/show.rs (new) · forbidden: changing plain output · check: `cargo test -p pin-toolkit --lib show::tests::origin 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M6.T1, M5.T5
- [ ] M6.T3: tests/cli.rs config_show_origin_*: global+repo+env+CLI layers end to end (json global, update repo, concurrency env, --update cli, ignore global+repo), exact inline lines.
  files: crates/pin-toolkit/tests/cli.rs · forbidden: insta snapshots · check: `cargo test -p pin-toolkit --test cli config_show_origin 2>&1 | grep -qE 'test result: ok\. [1-9]'` · depends: M6.T2

## M7: Carry-over nits: N1 drop Selection: Default (keep From<KindSet>); N2 narrow module-wide #[allow(dead_code)] in pin-core lib.rs and oci/mod.rs to the test-only items. (branch: task; depends: —)
Acceptance: Selection derives no Default; workspace builds all targets; no allow(dead_code) in pin-core lib.rs/oci/mod.rs; clippy -D warnings clean across the pin-core feature matrix
- [ ] M7.T1: plugin.rs: remove Default from Selection's derive (doc why: empty KindSet scans nothing); keep From<KindSet>; fix any user.
  files: crates/pin-core/src/plugin.rs · forbidden: removing From<KindSet> · check: `! grep -B1 '^pub struct Selection' crates/pin-core/src/plugin.rs | grep -q Default && cargo check --workspace --all-targets --all-features` · depends: —
- [ ] M7.T2: lib.rs: drop module-wide allow(dead_code) on pub(crate) mod gitlab; put item-level #[cfg_attr(not(feature = "test-support"), allow(dead_code))] on the test-only items in gitlab.rs.
  files: crates/pin-core/src/lib.rs, crates/pin-core/src/gitlab.rs · forbidden: changing D38 visibility cfgs · check: `! grep -q 'allow(dead_code)' crates/pin-core/src/lib.rs && cargo clippy -p pin-core --no-default-features --features gitlab --all-targets -- -D warnings` · depends: —
- [ ] M7.T3: oci/mod.rs: drop module-wide allow(dead_code) on pub(crate) mod registry; item-level cfg_attr allow on test-only items (Creds::load_from, ...) in registry.rs.
  files: crates/pin-core/src/oci/mod.rs, crates/pin-core/src/oci/registry.rs · forbidden: changing D38 visibility cfgs · check: `! grep -q 'allow(dead_code)' crates/pin-core/src/oci/mod.rs && cargo clippy -p pin-core --no-default-features --features oci --all-targets -- -D warnings` · depends: —

## M8: Docs close-out: README config section (global layer, [hosts] schema, config show, GITLAB_URI, D45 message, out-of-root config = cwd repo's); spec §7.1/§7.3 as-built, D45 done, D40 updated, D48 GITLAB_URI row, M5e' rows, §14.1 item closed. (branch: task; depends: M4, M6, M7)
Acceptance: README documents global path, version=1, [hosts] kinds, config show [--origin], GITLAB_URI precedence, out-of-root note; anchors resolve; spec: D40 + new D48 mention GITLAB_URI; order line marks M5e' ✓; §8.2 notes HM writes this schema
- [ ] M8.T1: README: document global config layer and precedence, [hosts] schema (names only), config show [--origin], GITLAB_URI precedence, D45 message, out-of-root directories use the cwd repo's config.
  files: README.md · forbidden: code changes · check: `grep -q 'GITLAB_URI' README.md && grep -q 'config show --origin' README.md && grep -q 'pin-toolkit/config.toml' README.md && grep -q 'token-env' README.md && bash scripts/check-readme-anchors.sh` · depends: —
- [ ] M8.T2: Spec: §7.1/§7.3 as-built for M5e', D45 marked done, D40 precedence with GITLAB_URI, new D48 row (owner 2026-11-11), M5e' rows/order ✓ in §9/§10, §14.1 M5e' bullet closed (N4 stays M7), §8.2 HM note.
  files: docs/superpowers/specs/2026-11-09-pin-toolkit-design.md · forbidden: code changes; rewriting unrelated decisions · check: `f=docs/superpowers/specs/2026-11-09-pin-toolkit-design.md; grep -qE '^\| D48 \|.*GITLAB_URI' $f && grep -qE '^\| D40 \|.*GITLAB_URI' $f && grep -q "M5e' ✓" $f` · depends: —

<details><summary>skeleton</summary>

```json
{
  "invariants": [
    "Repo [hosts]/concurrency stay exit 2: repo_hosts_exits_2 (config_layers.rs) and exit_codes row 9 pass unchanged; repo.rs GLOBAL_ONLY unchanged",
    "Config files hold env-var NAMES only; no token value is ever printed, logged or shown by Debug/config show",
    "A token reaches only its bound host: GITLAB_TOKEN->primary, CI_JOB_TOKEN->CI_SERVER_HOST only (D37), [hosts] token->its own host; env bindings win over [hosts]",
    "No process-env reads below the binary edge: config/global paths come from Process.env; tests pass explicit env maps (XDG_CONFIG_HOME/HOME under the scratch repo)",
    "Existing snapshots unchanged (INSTA_UPDATE=no); pin-gitlab-ci policy.rs unchanged; gitlab-hosts decides what resolves, never grants credentials",
    "bash scripts/gate.sh passes on every branch (tests, fmt, clippy -D warnings, no-default-features, D38, rustdoc)",
    "Files created by an earlier task (global.rs, show.rs) are edited, not recreated, by later tasks listing them in create"
  ],
  "milestones": [
    {
      "acceptance": [
        "global::parse returns a Layer (json, concurrency, hosts) or a Fatal naming the file",
        "missing/!=1 version, unknown key, bad TOML, bad host key, wrong per-kind fields -> Fatal",
        "Layer.hosts: Option<BTreeMap<String, HostBinding>>, replace-merged like files"
      ],
      "branch": "milestone",
      "dependsOn": [],
      "goal": "Parse the global config file: config/global.rs with version=1 required, json, concurrency and [hosts] bindings (gitlab/helm-repo/oci shapes), deny_unknown_fields, host keys via valid_gitlab_host; Layer gains hosts.",
      "id": "M1",
      "tasks": [
        {
          "check": "cargo test -p pin-toolkit --lib config::global 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "create": [
            "crates/pin-toolkit/src/config/global.rs"
          ],
          "dependsOn": [],
          "files": [
            "crates/pin-toolkit/src/config/mod.rs"
          ],
          "forbidden": [
            "changing repo.rs or GLOBAL_ONLY",
            "reading the filesystem or process env in global.rs (pure parse)"
          ],
          "goal": "Create pub mod config::global: parse(display:&Path, text:&str)->Result<Layer,Fatal> for version (required, ==1), json, concurrency with deny_unknown_fields; messages name the path like repo.rs. Unit tests config::global::tests (version missing/2, unknown key, json+concurrency set).",
          "id": "M1.T1"
        },
        {
          "check": "cargo test -p pin-toolkit --lib config::global::tests::hosts 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "create": [
            "crates/pin-toolkit/src/config/global.rs"
          ],
          "dependsOn": [
            "M1.T1"
          ],
          "files": [
            "crates/pin-toolkit/src/config/mod.rs"
          ],
          "forbidden": [
            "storing or reading secret values (env-var names only)",
            "wiring hosts into resolution here"
          ],
          "goal": "global.rs (from M1.T1) [hosts]: HostBinding (gitlab: token-env only; helm-repo: username-env+password-env; oci: ca-file; missing/extra field -> Fatal); host keys must pass valid_gitlab_host; Layer.hosts + replace-merge in mod.rs. Tests config::global::tests::hosts_*.",
          "id": "M1.T2"
        }
      ]
    },
    {
      "acceptance": [
        "precedence defaults<global<repo<env<CLI; fold test covers 5 layers incl. hosts",
        "--config/PIN_TOOLKIT_CONFIG override only the repo path; global still applies",
        "missing global file = no layer; unreadable/bad -> exit 2 naming the path"
      ],
      "branch": "milestone",
      "dependsOn": [
        "M1"
      ],
      "goal": "Fold the global layer: global_path from env ($XDG_CONFIG_HOME non-empty absolute, else $HOME/.config)/pin-toolkit/config.toml; load() = [global?, repo?, env, cli]; --no-config skips global and repo; Effective.hosts.",
      "id": "M2",
      "tasks": [
        {
          "check": "cargo test -p pin-toolkit --lib config::tests::global_path 2>&1 | grep -qE 'test result: ok\\. [1-9]' && cargo test -p pin-toolkit --lib config::tests::fold 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [
            "M1.T2"
          ],
          "files": [
            "crates/pin-toolkit/src/config/mod.rs"
          ],
          "forbidden": [
            "std::env reads",
            "changing load() in this task"
          ],
          "goal": "mod.rs: pub fn global_path(env)->Option<PathBuf>; Effective.hosts (empty default) from fold; fold_scalars_last_wins over 5 layers [defaults,global,repo,env,cli] incl. hosts. Unit tests config::tests::global_path_* (XDG over HOME, empty/relative XDG ignored, none).",
          "id": "M2.T1"
        },
        {
          "check": "cargo test -p pin-toolkit --test config_layers 2>&1 | grep -qE 'test result: ok\\. [0-9]+ passed; 0 failed' && cargo test -p pin-toolkit --test config_layers global_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [
            "M2.T1"
          ],
          "files": [
            "crates/pin-toolkit/src/config/mod.rs",
            "crates/pin-toolkit/tests/config_layers.rs"
          ],
          "forbidden": [
            "changing repo_hosts_exits_2 or exit_codes.rs",
            "letting --config/PIN_TOOLKIT_CONFIG replace the global path"
          ],
          "goal": "load(): global layer first (global::parse) when the file exists; --no-config skips it; read errors -> Fatal naming the path (cwd-relative where possible). config_layers.rs tests global_* (global<repo<env<CLI, --no-config, --config keeps global, bad global exit 2).",
          "id": "M2.T2"
        }
      ]
    },
    {
      "acceptance": [
        "auth_for: existing GITLAB_TOKEN/CI_JOB_TOKEN checks first, then bound -> Private, else Anonymous; Debug redacts bound values",
        "primary = arg > GITLAB_URI > CI_SERVER_HOST > gitlab.com; invalid GITLAB_URI -> Err(\"GITLAB_URI: ...\"); never grants CI_JOB_TOKEN",
        "failed lookup on non-primary Anonymous host -> `<host>: not trusted for credentials — bind it in <path>`; others unchanged"
      ],
      "branch": "task",
      "dependsOn": [],
      "goal": "pin-core: GitlabEnv.bound (host->token value) used by auth_for after the env bindings; GITLAB_URI in primary precedence with URL->host[:port] parse; D45 not-trusted message composed in Ctx::gitlab_resolve.",
      "id": "M3",
      "tasks": [
        {
          "check": "cargo test -p pin-core --all-features --lib auth_for_bound 2>&1 | grep -qE 'test result: ok\\. [1-9]' && cargo test -p pin-core --all-features --test gitlab_hosts bound_token 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [],
          "files": [
            "crates/pin-core/src/run.rs",
            "crates/pin-core/tests/gitlab_hosts.rs"
          ],
          "forbidden": [
            "reordering the existing GITLAB_TOKEN/CI_JOB_TOKEN checks",
            "changing pin-gitlab-ci"
          ],
          "goal": "run.rs: GitlabEnv gains pub bound: BTreeMap<String,String> (Default empty, Debug redacts values); auth_for step 3 = bound.get(host) -> Private. Unit test auth_for_bound_host; wiremock bound_token_only_reaches_bound_host in gitlab_hosts.rs.",
          "id": "M3.T1"
        },
        {
          "check": "cargo test -p pin-core --all-features --lib gitlab_uri 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [
            "M3.T1"
          ],
          "files": [
            "crates/pin-core/src/run.rs"
          ],
          "forbidden": [
            "binding CI_JOB_TOKEN to the GITLAB_URI host",
            "falling back to gitlab.com on an invalid GITLAB_URI"
          ],
          "goal": "GitlabEnv::from_vars reads GITLAB_URI (empty=unset): strip http(s)://, cut path/trailing slash, validate with valid_gitlab_host, else Err(\"GITLAB_URI: invalid GitLab host `v`\"); primary = arg > GITLAB_URI > CI_SERVER_HOST > gitlab.com. Unit tests gitlab_uri_*.",
          "id": "M3.T2"
        },
        {
          "check": "cargo test -p pin-core --all-features --test gitlab_hosts unbound_host_failure 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [
            "M3.T1"
          ],
          "files": [
            "crates/pin-core/src/run.rs",
            "crates/pin-core/tests/gitlab_hosts.rs"
          ],
          "forbidden": [
            "changing pin-gitlab-ci policy.rs",
            "changing primary-host or bound-host error texts"
          ],
          "goal": "GitlabEnv.global_config: Option<String>; gitlab_resolve maps a client error on host!=primary with Anonymous auth to `{host}: not trusted for credentials — bind it in {path}` (None -> ~/.config/pin-toolkit/config.toml). Wiremock tests unbound_host_failure_*.",
          "id": "M3.T3"
        }
      ]
    },
    {
      "acceptance": [
        "wiremock: bound token sent as PRIVATE-TOKEN only to the bound host; unbound hosts anonymous",
        "GITLAB_URI primary end-to-end; invalid GITLAB_URI exits 2 before any request; no JOB-TOKEN to its host",
        "unbound non-primary failure row reads `unresolved (<host>: not trusted for credentials — bind it in <global path>)`"
      ],
      "branch": "task",
      "dependsOn": [
        "M2",
        "M3"
      ],
      "goal": "Toolkit wiring: fill GitlabEnv.bound from Effective.hosts (kind gitlab, named env var non-empty), pass the global config path for D45, GITLAB_URI end-to-end tests and --gitlab-host help text.",
      "id": "M4",
      "tasks": [
        {
          "check": "cargo test -p pin-toolkit --test credentials bound_token 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [
            "M3.T1",
            "M2.T2"
          ],
          "files": [
            "crates/pin-toolkit/src/lib.rs",
            "crates/pin-toolkit/tests/credentials.rs"
          ],
          "forbidden": [
            "adding bound hosts to gitlab-hosts/resolve set",
            "reading std::env"
          ],
          "goal": "lib.rs: after ResolverEnv::from_vars fill renv.gitlab.bound from eff.hosts (kind gitlab, p.env[token-env] non-empty); no hosts added to resolve set. credentials.rs bound_token_* (global file via XDG_CONFIG_HOME; A gets token, B none; empty var = anonymous).",
          "id": "M4.T1"
        },
        {
          "check": "cargo test -p pin-toolkit --test credentials gitlab_uri 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [
            "M3.T2",
            "M4.T1"
          ],
          "files": [
            "crates/pin-toolkit/tests/credentials.rs",
            "crates/pin-toolkit/src/cli.rs"
          ],
          "forbidden": [
            "changing existing credentials tests' assertions"
          ],
          "goal": "credentials.rs gitlab_uri_*: GITLAB_URI=https://gitlab.example.com/ is primary (GITLAB_TOKEN there), CI_JOB_TOKEN never sent to it without CI_SERVER_HOST, --gitlab-host beats it, invalid value exits 2, no request. cli.rs --gitlab-host help names GITLAB_URI.",
          "id": "M4.T2"
        },
        {
          "check": "cargo test -p pin-toolkit --test credentials untrusted_host 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [
            "M3.T3",
            "M4.T1"
          ],
          "files": [
            "crates/pin-toolkit/src/lib.rs",
            "crates/pin-toolkit/tests/credentials.rs"
          ],
          "forbidden": [
            "composing the message in pin-toolkit or pin-gitlab-ci",
            "updating snapshots"
          ],
          "goal": "lib.rs sets renv.gitlab.global_config from global_path(&p.env) (also with --no-config). credentials.rs untrusted_host_*: unbound non-primary 404 -> `unresolved (gitlab.com: not trusted for credentials — bind it in <xdg>/pin-toolkit/config.toml)`; primary keeps text.",
          "id": "M4.T3"
        }
      ]
    },
    {
      "acceptance": [
        "prints config.global-file/repo-file, update, release-age, concurrency, json, gitlab-host, plugins.<id>.*, hosts.<host>.* lines",
        "bad config or invalid CI_SERVER_HOST/GITLAB_URI exits 2; --only/--skip with config exits 2; env ONLY/SKIP ignored",
        "never prints token values; `./config` is a path"
      ],
      "branch": "task",
      "dependsOn": [
        "M2",
        "M3"
      ],
      "goal": "`pin-toolkit config show`: load_sources (layers with Origin, file displays), pub mod show renderer (dotted keys, design S5 format), Cmd::Config/ConfigCmd::Show{origin}/Mode::Config, run_mode branch after ResolverEnv::from_vars.",
      "id": "M5",
      "tasks": [
        {
          "check": "cargo test -p pin-toolkit --lib config::tests::load_sources 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [
            "M2.T2"
          ],
          "files": [
            "crates/pin-toolkit/src/config/mod.rs"
          ],
          "forbidden": [
            "changing Layer fields for provenance",
            "changing load() semantics"
          ],
          "goal": "mod.rs: pub enum Origin {Default,Global,Repo,Env,Cli} (lowercase Display); pub struct Loaded {eff, global_file, repo_file: Option<String>, layers: Vec<(Origin,Layer)>}; pub fn load_sources, load() delegates. Unit tests config::tests::load_sources_*.",
          "id": "M5.T1"
        },
        {
          "check": "cargo test -p pin-toolkit --lib show::tests::renders 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "create": [
            "crates/pin-toolkit/src/show.rs"
          ],
          "dependsOn": [
            "M5.T1"
          ],
          "files": [
            "crates/pin-toolkit/src/lib.rs"
          ],
          "forbidden": [
            "printing env-var values",
            "snapshot files for config show"
          ],
          "goal": "Create pub mod show (declared in lib.rs; also rewrite lib.rs `forced` match as `if let Mode::Plugin(id)`): render(&Loaded, primary)->String for config.*, update.*, release-age.*, concurrency, json, gitlab-host, hosts.* lines. Tests show::tests::renders_*.",
          "id": "M5.T2"
        },
        {
          "check": "cargo test -p pin-toolkit --lib show::tests::plugins 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "create": [
            "crates/pin-toolkit/src/show.rs"
          ],
          "dependsOn": [
            "M5.T2"
          ],
          "files": [
            "crates/pin-toolkit/src/enable.rs"
          ],
          "forbidden": [
            "changing enable.rs semantics"
          ],
          "goal": "show.rs (from M5.T2): plugins.<id>.{run,files,ignore,kinds.<k>,gitlab-hosts} for compiled-in plugins with defaults filled as enable.rs computes them (expose a helper there if needed), plus set keys of configured unbuilt tables. Tests show::tests::plugins_*.",
          "id": "M5.T3"
        },
        {
          "check": "cargo test -p pin-toolkit --lib cli::tests::config_show 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [
            "M5.T3"
          ],
          "files": [
            "crates/pin-toolkit/src/cli.rs",
            "crates/pin-toolkit/src/enable.rs"
          ],
          "forbidden": [
            "args_conflicts_with_subcommands",
            "changing existing subcommand parsing"
          ],
          "goal": "cli.rs: Cmd::Config{cmd:ConfigCmd}, ConfigCmd::Show{--origin}, Mode::Config{origin}; fix Cli::paths/mode; enable.rs arm for Mode::Config. Unit tests cli::tests::config_show_* (mode parse, `./config` path, `config` after a path is a path).",
          "id": "M5.T4"
        },
        {
          "check": "cargo test -p pin-toolkit --test cli config_show_prints 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [
            "M5.T3",
            "M5.T4"
          ],
          "files": [
            "crates/pin-toolkit/src/lib.rs",
            "crates/pin-toolkit/tests/cli.rs"
          ],
          "forbidden": [
            "insta snapshots for config show",
            "walking files or building resolvers in config show"
          ],
          "goal": "lib.rs run_mode: Mode::Config branch after ResolverEnv::from_vars prints show::render (primary = renv.gitlab.primary); --only/--skip CLI with it -> exit 2. tests/cli.rs config_show_prints_* (inline asserts: defaults, global+repo values, bad config exit 2, token value absent).",
          "id": "M5.T5"
        }
      ]
    },
    {
      "acceptance": [
        "each line ends `  # default|global|repo|env|cli`; ignore e.g. `# global+repo`",
        "gitlab-host from GITLAB_URI/CI_SERVER_HOST shows env; computed from Loaded.layers, no Layer change"
      ],
      "branch": "task",
      "dependsOn": [
        "M5"
      ],
      "goal": "`config show --origin`: per-line `  # <origin>` (highest layer that set it, else default; ignore lists all contributing layers highest-first joined by +).",
      "id": "M6",
      "tasks": [
        {
          "check": "cargo test -p pin-toolkit --lib config::tests::origin 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [
            "M5.T1"
          ],
          "files": [
            "crates/pin-toolkit/src/config/mod.rs"
          ],
          "forbidden": [
            "adding provenance fields to Layer"
          ],
          "goal": "config/mod.rs: pub fn origin_of(layers:&[(Origin,Layer)], set:impl Fn(&Layer)->bool)->Origin and origins_of(..)->Vec<Origin> (highest first, for ignore). Unit tests config::tests::origin_*.",
          "id": "M6.T1"
        },
        {
          "check": "cargo test -p pin-toolkit --lib show::tests::origin 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "create": [
            "crates/pin-toolkit/src/show.rs"
          ],
          "dependsOn": [
            "M6.T1",
            "M5.T5"
          ],
          "files": [
            "crates/pin-toolkit/src/lib.rs"
          ],
          "forbidden": [
            "changing plain output"
          ],
          "goal": "show.rs (from M5): origin flag appends `  # <origin>` to every line via origin_of/origins_of (ignore joined with +); lib.rs passes Mode::Config{origin}. Unit tests show::tests::origin_*.",
          "id": "M6.T2"
        },
        {
          "check": "cargo test -p pin-toolkit --test cli config_show_origin 2>&1 | grep -qE 'test result: ok\\. [1-9]'",
          "dependsOn": [
            "M6.T2"
          ],
          "files": [
            "crates/pin-toolkit/tests/cli.rs"
          ],
          "forbidden": [
            "insta snapshots"
          ],
          "goal": "tests/cli.rs config_show_origin_*: global+repo+env+CLI layers end to end (json global, update repo, concurrency env, --update cli, ignore global+repo), exact inline lines.",
          "id": "M6.T3"
        }
      ]
    },
    {
      "acceptance": [
        "Selection derives no Default; workspace builds all targets",
        "no allow(dead_code) in pin-core lib.rs/oci/mod.rs; clippy -D warnings clean across the pin-core feature matrix"
      ],
      "branch": "task",
      "dependsOn": [],
      "goal": "Carry-over nits: N1 drop Selection: Default (keep From<KindSet>); N2 narrow module-wide #[allow(dead_code)] in pin-core lib.rs and oci/mod.rs to the test-only items.",
      "id": "M7",
      "tasks": [
        {
          "check": "! grep -B1 '^pub struct Selection' crates/pin-core/src/plugin.rs | grep -q Default && cargo check --workspace --all-targets --all-features",
          "dependsOn": [],
          "files": [
            "crates/pin-core/src/plugin.rs"
          ],
          "forbidden": [
            "removing From<KindSet>"
          ],
          "goal": "plugin.rs: remove Default from Selection's derive (doc why: empty KindSet scans nothing); keep From<KindSet>; fix any user.",
          "id": "M7.T1"
        },
        {
          "check": "! grep -q 'allow(dead_code)' crates/pin-core/src/lib.rs && cargo clippy -p pin-core --no-default-features --features gitlab --all-targets -- -D warnings",
          "dependsOn": [],
          "files": [
            "crates/pin-core/src/lib.rs",
            "crates/pin-core/src/gitlab.rs"
          ],
          "forbidden": [
            "changing D38 visibility cfgs"
          ],
          "goal": "lib.rs: drop module-wide allow(dead_code) on pub(crate) mod gitlab; put item-level #[cfg_attr(not(feature = \"test-support\"), allow(dead_code))] on the test-only items in gitlab.rs.",
          "id": "M7.T2"
        },
        {
          "check": "! grep -q 'allow(dead_code)' crates/pin-core/src/oci/mod.rs && cargo clippy -p pin-core --no-default-features --features oci --all-targets -- -D warnings",
          "dependsOn": [],
          "files": [
            "crates/pin-core/src/oci/mod.rs",
            "crates/pin-core/src/oci/registry.rs"
          ],
          "forbidden": [
            "changing D38 visibility cfgs"
          ],
          "goal": "oci/mod.rs: drop module-wide allow(dead_code) on pub(crate) mod registry; item-level cfg_attr allow on test-only items (Creds::load_from, ...) in registry.rs.",
          "id": "M7.T3"
        }
      ]
    },
    {
      "acceptance": [
        "README documents global path, version=1, [hosts] kinds, config show [--origin], GITLAB_URI precedence, out-of-root note; anchors resolve",
        "spec: D40 + new D48 mention GITLAB_URI; order line marks M5e' ✓; §8.2 notes HM writes this schema"
      ],
      "branch": "task",
      "dependsOn": [
        "M4",
        "M6",
        "M7"
      ],
      "goal": "Docs close-out: README config section (global layer, [hosts] schema, config show, GITLAB_URI, D45 message, out-of-root config = cwd repo's); spec §7.1/§7.3 as-built, D45 done, D40 updated, D48 GITLAB_URI row, M5e' rows, §14.1 item closed.",
      "id": "M8",
      "tasks": [
        {
          "check": "grep -q 'GITLAB_URI' README.md && grep -q 'config show --origin' README.md && grep -q 'pin-toolkit/config.toml' README.md && grep -q 'token-env' README.md && bash scripts/check-readme-anchors.sh",
          "dependsOn": [],
          "files": [
            "README.md"
          ],
          "forbidden": [
            "code changes"
          ],
          "goal": "README: document global config layer and precedence, [hosts] schema (names only), config show [--origin], GITLAB_URI precedence, D45 message, out-of-root directories use the cwd repo's config.",
          "id": "M8.T1"
        },
        {
          "check": "f=docs/superpowers/specs/2026-11-09-pin-toolkit-design.md; grep -qE '^\\| D48 \\|.*GITLAB_URI' $f && grep -qE '^\\| D40 \\|.*GITLAB_URI' $f && grep -q \"M5e' ✓\" $f",
          "dependsOn": [],
          "files": [
            "docs/superpowers/specs/2026-11-09-pin-toolkit-design.md"
          ],
          "forbidden": [
            "code changes",
            "rewriting unrelated decisions"
          ],
          "goal": "Spec: §7.1/§7.3 as-built for M5e', D45 marked done, D40 precedence with GITLAB_URI, new D48 row (owner 2026-11-11), M5e' rows/order ✓ in §9/§10, §14.1 M5e' bullet closed (N4 stays M7), §8.2 HM note.",
          "id": "M8.T2"
        }
      ]
    }
  ]
}
```
</details>
