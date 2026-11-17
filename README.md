# pin-toolkit

Pin CI references to immutable ids: container images in GitLab CI files to
digests, GitLab components and includes to commit SHAs.

`pin-toolkit` is one multi-call binary with one plugin per file format
(see [Plugins](#plugins)). It replaces the former `pin-ci` binary, which
no longer exists: `pin-ci` is now only an argv[0] alias of
`pin-toolkit gitlab-ci` (see [Aliases](#aliases)). Installation (Nix, Home
Manager, release tarballs) and the GitLab CI image are described in
[Install](#install) and [CI image](#ci-image).

## What it does

Every image or service reference is resolved to its current digest and
rewritten in place as `original-as-written@sha256:<digest>` (an existing
`@sha256:<old>` has its digest replaced; tags are never changed without
`--update`). Images on every docker-v2 registry host are pinned (see
[Multi-registry support](#multi-registry-support)). With `--update`, each
reference's tag is first bumped to the latest eligible stable tag (see
[Tag updates](#tag-updates---update)). The same run pins GitLab
**components** and **classic includes** to commit SHAs (see
[Components & includes](#components--includes)).

Edits are byte-exact and idempotent: bytes outside a reference are never
touched, and a second run reports everything `ok` and writes nothing.

## Install

No release has been tagged yet: the Cachix cache, the GHCR image and the
tarballs described below appear once the owner has done the setup steps and
pushed the first `v*` tag. Until then, build from source.

The Nix flake (`github:GRBurst/pin-toolkit`) builds for `x86_64-linux`,
`aarch64-linux` and `aarch64-darwin`. Its `default` package has every plugin
and is the store path that the Cachix cache serves. Release tarballs cover
Linux only (see [Release tarballs](#release-tarballs)).

### Nix flake input

```nix
inputs.pin-toolkit.url = "github:GRBurst/pin-toolkit";
# optional, safe: only dedups the Home Manager input of your lock
inputs.pin-toolkit.inputs.home-manager.follows = "home-manager";
```

Do **not** add `inputs.pin-toolkit.inputs.nixpkgs.follows` or
`inputs.pin-toolkit.inputs.crane.follows`. The package's store path depends
on the flake's own nixpkgs and crane, so either `follows` changes it and the
Cachix cache misses (the build then runs locally). `home-manager.follows` is
the exception: the package never uses that input.

### Nix profile and Cachix

```sh
cachix use pin-toolkit            # the cache is `pin-toolkit` (pin-toolkit.cachix.org)
nix profile install github:GRBurst/pin-toolkit
```

`cachix use` needs the `cachix` CLI. Until the owner has created the cache,
`cachix use pin-toolkit` fails and every build runs from source.

### Home Manager

```nix
# in your Home Manager modules (the flake input above)
imports = [ inputs.pin-toolkit.homeManagerModules.default ];
programs.pin-toolkit = {
  enable = true;
  # every plugin enabled (the default): the cached package
  settings = {
    concurrency = 4;
    hosts."gitlab.example.com" = {
      kind = "gitlab";
      token-env = "GITLAB_EXAMPLE_TOKEN";   # the NAME of an environment variable
    };
  };
};
```

`settings` is written to the global config file `pin-toolkit/config.toml`
(`version = 1` is added). It takes `json`, `concurrency` and `[hosts]` only;
`update`, `release_age`, `plugins` and `version` belong in a repository's
`.pin-toolkit.toml`. The `*-env` host values are environment-variable **names**: a value
that is not a valid variable name (for example a pasted `glpat-…` token with a
dash) fails evaluation. A well-formed name such as `ghp_…` is accepted, so the
rule is "names only", not "no token-looking text". The Nix store is
world-readable: never put a secret in `settings`.

`programs.pin-toolkit.plugins.<name>.enable` decides whether a plugin is
**in the build**. Disabling one makes `finalPackage` a feature-trimmed build
that is not in Cachix and compiles locally. To stop a built plugin from running
in one repository, set `[plugins.<name>] run = false` in its
`.pin-toolkit.toml`.

### Plugin override (uncached)

```nix
# a different store path: not in Cachix, compiled locally
pin-toolkit.packages.${system}.default.override { plugins = [ ]; }
```

Today the only plugin is `gitlab-ci`, so the default is already every plugin:
`override { plugins = [ "gitlab-ci" ]; }` is the same set and the cached path.
Any other set (here `[ ]`) is a different store path that Cachix does not have.

The list names the plugins to build in (`pin-toolkit plugins` shows the
built set). An unknown plugin name fails evaluation.

### Overlay

```nix
nixpkgs.overlays = [ inputs.pin-toolkit.overlays.default ];  # pkgs.pin-toolkit
```

The overlay builds with **your** nixpkgs: unless that is the same nixpkgs
revision as the flake's lock, the store path differs from the flake's package
and nothing is substituted from Cachix. The build needs rustc
>= 1.88 (let-chains in `pin-core`); nixpkgs-unstable has it.

### Release tarballs

Each `v*` release (Linux only) attaches
`pin-toolkit-v<version>-<triple>.tar.gz` and its `.sha256`, for
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`. The binaries are
static. Each tarball holds `pin-toolkit`, the `pin-<plugin>` and `pin-ci` aliases, `LICENSE`
and `README.md`.

```sh
sha256sum -c pin-toolkit-v<version>-x86_64-unknown-linux-musl.tar.gz.sha256
gh attestation verify pin-toolkit-v<version>-x86_64-unknown-linux-musl.tar.gz -R GRBurst/pin-toolkit
```

### macOS

There is no macOS release tarball, and the flake has no `x86_64-darwin`
output (Intel Macs: build from source). On `aarch64-darwin`, use the flake
(`nix profile install github:GRBurst/pin-toolkit`, cached once CI has pushed the
darwin build) or build from source. `cargo install` needs rustc >= 1.88 and
installs `pin-toolkit` only (no `pin-*` aliases):

```sh
cargo install --locked --git https://github.com/GRBurst/pin-toolkit pin-toolkit
```

## CI image

The image `ghcr.io/grburst/pin-toolkit` runs on GitLab CI. It contains the
static `pin-toolkit` binary (all plugins), a static busybox with `sh` and
`grep`, and the CA bundle. It has **no entrypoint**, so GitLab can pipe the
job script into `sh`. Tags are `<version>` and `:latest` (not for pre-release
versions, which carry a `-`), and each index is attested. Pin the digest:

```yaml
pin-check:
  image: ghcr.io/grburst/pin-toolkit:<version>@sha256:<digest>   # from: docker buildx imagetools inspect ghcr.io/grburst/pin-toolkit:<version>
  script: [ pin-toolkit --check --json > pin-report.json ]
  artifacts: { when: always, paths: [pin-report.json] }
  rules: [ { if: $CI_PIPELINE_SOURCE == "merge_request_event" } ]
```

The image is public once the owner has set the GHCR package to public. Only
private registries need credentials: write them to `$HOME/.docker/config.json`
(the image sets no `HOME`, so the runner's value applies; set `HOME` in the job
if unsure. This is the only location pin-toolkit reads; it does not
read `DOCKER_CONFIG` or `DOCKER_AUTH_CONFIG`). For GitLab, the
job's `CI_JOB_TOKEN` is used for `CI_SERVER_HOST` only; set `GITLAB_TOKEN`
for the primary host (see [GitLab hosts and auth](#gitlab-hosts-and-auth)).

## Plugins

One plugin per file format, selected by subcommand or alias. This build
ships `gitlab-ci`; `helm` and `github-actions` are known plugin ids that
are not built yet (`pin-toolkit plugins` shows the state of each).

| Plugin | State | Pins |
|---|---|---|
| [gitlab-ci](#plugin-gitlab-ci) | built | images and services to digests, components and includes to commit SHAs |
| [helm](#plugin-helm) | known, not built | Chart.yaml dependencies, values images, helmfile releases (planned) |
| [github-actions](#plugin-github-actions) | known, not built | actions, docker-action, container (planned) |

## Usage

```
pin-toolkit [OPTIONS] [PATHS]...             # every enabled plugin
pin-toolkit gitlab-ci [OPTIONS] [PATHS]...   # only the gitlab-ci plugin
pin-toolkit plugins                          # compiled-in plugins, kinds, enable state
pin-toolkit config show [--origin]           # the effective configuration
```

- `PATHS` — files or directories. Default: the git root (the first
  ancestor of the current directory that contains `.git`), else the
  current directory. Without paths the whole repository is walked, also
  from a subdirectory; paths are shown relative to the current directory
  (`../.gitlab-ci.yml`). See [Which files are pinned](#which-files-are-pinned).
- A plugin subcommand must come **before** the paths:
  `pin-toolkit a.yml gitlab-ci` treats `gitlab-ci` as a path, so it is an
  all-plugins run that fails with `no plugin claims …` (exit `2`).
- clap's `help` subcommand exists (`pin-toolkit help`, `pin-toolkit help
  gitlab-ci`), so a directory named `help` must be passed as `./help`;
  `pin-toolkit help config` shows the `config` subcommand's help.
- A file named `config` in the current directory must be passed as
  `./config`: the `config` subcommand wins over a bare path.
- Options may appear before or after the subcommand.

| Option | Meaning |
|---|---|
| `--dry-run` | resolve and report without writing files |
| `--check` | `--dry-run`, and exit `1` if anything would change (CI gate) |
| `--json` | print the report as JSON (schema v1, see [Report](#report)) |
| `--update` | bump versioned references to the latest compatible stable tag before pinning |
| `--update-major` | with `--update`, also allow a higher major version |
| `--min-age-days <N>` | with `--update`, skip candidates younger than N days (`0` = off, the default; recommended `14`) |
| `--concurrency <N>` | max resolutions in flight per file (default `8`) |
| `--only <P,…>` | run only these plugins |
| `--skip <P,…>` | do not run these plugins |
| `--kind <P:K=on\|off>` | turn one kind on or off, repeatable (e.g. `--kind gitlab-ci:service=off`) |
| `--config <FILE>` | repository config file (default: `.pin-toolkit.toml` at the git root); the [global config file](#global-config-file) is read separately |
| `--no-config` | read neither the repository config nor the [global config file](#global-config-file) |
| `--gitlab-host <H>` | primary GitLab host (default: `PIN_TOOLKIT_GITLAB_HOST`, `GITLAB_URI`, `CI_SERVER_HOST`, else `gitlab.com`; see [GitLab hosts and auth](#gitlab-hosts-and-auth)) |

Flags are presence-only (there is no `--no-update`; use
`PIN_TOOLKIT_UPDATE=0` over a repo `enabled = true`).

`--only`/`--skip` take known plugin ids (`gitlab-ci`, `helm`,
`github-actions`); items are trimmed, empty items are dropped, and an
empty list counts as not given. An unknown id exits `2`
(``unknown plugin `x` (known: gitlab-ci, helm, github-actions)``), for
`--skip` too. `--only helm` in this build exits `2` with
`plugin helm not in this build`; `pin-toolkit --only helm plugins` (or
`PIN_TOOLKIT_ONLY=helm pin-toolkit plugins`) still lists, exit `0`, with
`pin-toolkit: note: plugin helm not in this build (named by --only/PIN_TOOLKIT_ONLY; a run exits 2)`
on stderr. On the command line, `--only`/`--skip`
together with a plugin subcommand or alias exit `2`
(`--only/--skip cannot be combined with a plugin subcommand`).

Example:

```console
$ pin-toolkit --update --update-major
.gitlab-ci.yml:1  gitlab-ci:image  updated  alpine:3.20 -> alpine:3.24@sha256:25b0e91ad7048476c484f3a5128e2b5d9a4f2c1e8b7a6d5c4f3e2d1a0b9c8d7e  # alpine:3.20
.gitlab-ci.yml:4  gitlab-ci:service  updated  postgres:16 -> postgres:17@sha256:8083fbb2465d13d61f77932a49d1a1fde3c2b1a09f8e7d6c5b4a39281706f5e4  # postgres:16
pinned: 0, ok: 0, updated: 2, skipped: 0, unresolved: 0
```

### Aliases

`pin-<plugin>` (e.g. `pin-gitlab-ci`) and `pin-ci` are argv[0] aliases of
`pin-toolkit <plugin>` / `pin-toolkit gitlab-ci`: the same flags, config,
output and exit codes. The `pin-ci` alias is kept as one dispatch entry;
the Nix package and the release tarballs ship the symlinks. Elsewhere (for
example `cargo install`), create one by hand (`ln -s pin-toolkit pin-ci`). `pin-ci` has no legacy mode: without paths it
walks the repository (there is no `ci/.gitlab-ci.yml` default any more),
and it accepts any explicit file. Usage and help texts always say
`pin-toolkit`. `pin-helm` (a known plugin that is not built) exits `2` with
`plugin helm not in this build`.

### Which files are pinned

- **Walk.** A directory (the default git root, or one passed as a path) is
  walked with the `ignore` crate's defaults: `.ignore` files and, inside a
  git repository, `.gitignore`, `.git/info/exclude` and the user's global
  gitignore are honoured, so results can differ per machine. Hidden files are walked
  (`.gitlab-ci.yml`, `.gitlab/`); `.git` is skipped. Symlinks are not
  followed during a walk; a symlink passed explicitly is used as given.
- **Globs.** Each walked file is matched against the `files` globs of
  every enabled plugin, **relative to the git root** (also for an explicit
  directory such as `ci/`). gitlab-ci defaults:
  `.gitlab-ci.yml`, `**/*.gitlab-ci.yml`, `.gitlab/ci/**/*.yml`; `*` does
  not cross `/`, and `**/*.gitlab-ci.yml` also matches
  `ci/.gitlab-ci.yml`. A directory outside the git root is matched
  relative to its own git root, or to itself when it is in no repository;
  an explicit file outside the git root likewise, relative to its
  directory's git root, or to its directory.
- **Walked files** nobody claims are ignored. A walked file claimed by two
  plugins is a per-file error (exit `2`; the other files still run):
  ``x.yml is claimed by p1 and p2 (narrow their `files` globs, or choose one: pin-toolkit <plugin> x.yml)``.
- **Explicit files.** With a plugin subcommand or alias a file is claimed
  regardless of the globs. Otherwise it must match exactly one enabled
  plugin's globs (relative as above), else exit `2` before the run:
  `no plugin claims docs/readme.yml (choose one: pin-toolkit <plugin> docs/readme.yml)`.
- A file passed twice (`a.yml`, `./a.yml`, `ci/../a.yml`, or a file and its
  directory) runs once, in any argument order with the same claim. It is
  shown as its first occurrence: an explicit file as spelled, a walked
  file relative to the cwd (`./ci/` shows `ci/x.yml`). Outside a
  repository a file named with its directory is matched relative to the
  directory. Files run in path order, one after the other.
- **Walk errors** (e.g. an unreadable directory) are per-file errors:
  `pin-toolkit: secret: Permission denied (os error 13)`, exit `2`. A bad
  line in an ignore file only warns, exit unaffected:
  `pin-toolkit: warning: .gitignore:3: …`.

### Report

Human (stdout): one row per reference, the total summary line, then (when
more than one plugin ran) one summary line per plugin, then `follow-up:`
lines (gitlab-ci has none):

```
{file}:{line}  {plugin}:{kind}  {status}[ ({reason})]  {old} -> {new|-}  # {reference}
pinned: N, ok: N, updated: N, skipped: N, unresolved: N
{plugin}: pinned: N, ok: N, updated: N, skipped: N, unresolved: N
follow-up: {text}
```

Kinds of the gitlab-ci plugin: `image`, `service`, `component`, `include`
(`helm-oci` is accepted in config and `--kind` but has no targets until M7).
`services:` items are reported as `service` (before M5d: `image`). A
disabled kind produces no row.

JSON (`--json`, stdout), schema version 1:

```json
{
  "schema_version": 1,
  "results": [
    { "plugin": "gitlab-ci", "kind": "image", "file": ".gitlab-ci.yml", "line": 1,
      "reference": "alpine:3.20", "old": "alpine:3.20", "new": "alpine:3.20@sha256:…",
      "status": "pinned" }
  ],
  "summary": { "pinned": 1, "ok": 0, "updated": 0, "skipped": 0, "unresolved": 0 },
  "plugins": {
    "gitlab-ci": {
      "summary": { "pinned": 1, "ok": 0, "updated": 0, "skipped": 0, "unresolved": 0 },
      "errors": [ { "file": "bad.gitlab-ci.yml", "line": 2, "message": "invalid YAML: …" } ]
    }
  },
  "follow_ups": []
}
```

- A row has `reason` only when it has one; `new` is `null` when nothing
  would be written (`--dry-run`/`--check` rows carry `new`). An error has
  `line` only when known.
- `plugins` has one entry per **enabled** plugin, even if it found no file
  (zero summary, no errors). The per-plugin summaries add up to `summary`.
- `errors` are sorted by file, then line, whatever their origin (read,
  parse or write error, walk error, walked file claimed twice). A walk
  error is listed under the first enabled plugin.
- Per-file errors also go to **stderr**, in both modes:
  `pin-toolkit: {file}:{line}: {message}` or `pin-toolkit: {file}: {message}`.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | nothing `unresolved` and no error; with `--check` also nothing would change |
| `1` | at least one reference `unresolved`; with `--check` also any row that would be `pinned` or `updated` |
| `2` | an error (below) |

Exit `2` comes in two forms:

- **Fatal, before the run:** a usage error, a config error (global or
  repo config file, `GITLAB_URI` or `CI_SERVER_HOST`, env value,
  `--kind`, `--gitlab-host`), an explicit
  path claimed by no or two plugins, a plugin that is requested
  (subcommand, alias, `--only`, `PIN_TOOLKIT_ONLY`) but not in this build
  (`plugins` only notes the latter two). stderr says
  `pin-toolkit: {message}`; **stdout is empty, also with `--json`**;
  nothing is written.
- **Per-file:** a file that cannot be read, parsed or written, a walk
  error, a walked file claimed twice. The file is recorded under
  `plugins.<id>.errors`; the other files are still pinned and the report
  is printed.

A file that is not valid YAML (e.g. a tab as block indentation) is a
per-file error (`<path>:<line>: invalid YAML: …`); nothing in it is
changed. A tab as separator after `key:` (`image:<TAB>alpine`) is valid
YAML and is read as such. A file with bare-CR (`\r`-only) line endings is
an error too (`bare CR line endings are not supported`).

`--check` is meant as a CI gate: exit `1` when a run would change a file,
nothing is written, and `0` again after a pin run.

## Configuration

Settings come from five layers, each overriding the one before:
**defaults < global config file < repo `.pin-toolkit.toml` < env
`PIN_TOOLKIT_*` < CLI**. The [global config file](#global-config-file)
(`$XDG_CONFIG_HOME/pin-toolkit/config.toml`, else
`$HOME/.config/pin-toolkit/config.toml`) is the per-user layer, shared by
all of the user's repositories (the HM module generates it).

- Scalars: the highest layer that sets them wins. `files` and
  `gitlab-hosts` lists are **replaced** by a higher layer; `ignore` lists
  **concatenate**; `kinds` merge per kind.
- The repo file is `--config FILE` if given, else `PIN_TOOLKIT_CONFIG`,
  else `.pin-toolkit.toml` at the git root if it exists. A named file must
  exist (`config file not found: …`); a file that cannot be read exits `2`
  with `{path}: {error}`. `--no-config` reads neither the repo file nor
  the global file; `--config`/`PIN_TOOLKIT_CONFIG` replace only the repo
  path, the global file still applies. An explicit path outside the
  current repository is pinned with the current repository's config —
  its own `.pin-toolkit.toml`, if any, is not read; the global layer is
  independent of the current directory.

### `.pin-toolkit.toml`

```toml
version = 1                                   # required, must be 1
[update]
enabled = false                               # = --update
major = false                                 # = --update-major
[release_age]
min_days = 0                                  # = --min-age-days
unknown = "allow"                             # "allow" | "block", see Release age guard
[plugins.gitlab-ci]
run = true                                    # false: not run in all-plugins mode
files = [".gitlab-ci.yml", "**/*.gitlab-ci.yml", ".gitlab/ci/**/*.yml"]   # replaces the defaults
ignore = ["registry.example.com/legacy/*"]    # references never touched
gitlab-hosts = ["gitlab.com", "gitlab.example.com"]   # hosts to resolve; never credentials
kinds = { image = true, service = true, component = true, include = true, helm-oci = true }
[plugins.helm]                                # accepted and validated, unused until helm is built
run = true
kinds = { chart-dependency = true, values-image = true, helmfile-release = true }
values = { tag-digest = true }
helmfile = { oci-digest = false }
aliases = { stable = "https://charts.helm.sh/stable" }
[plugins.github-actions]                      # accepted and validated, unused until built
run = true
kinds = { action = true, docker-action = true, container = true }
```

Exit `2` (the message names the file) for: a missing `version` or one
other than `1`; any unknown key, plugin id or kind; a wrong type; an
invalid glob in any plugin's `files`
(``invalid glob `[` in plugins.gitlab-ci.files: …``); a `gitlab-hosts`
entry that is not a plain host (see [GitLab hosts and auth](#gitlab-hosts-and-auth));
a helm `aliases` URL with credentials
(`plugins.helm.aliases.<name>: a repository URL must not contain credentials`);
and the **global-only** keys `hosts` and `concurrency`
(`` `hosts` is a global-only key; it is not allowed in a repository config ``).
A repository cannot set the primary GitLab host, the output format,
`--dry-run`/`--check` or `--only`/`--skip`: no such key exists.

`run = false` stops a plugin in all-plugins mode; `pin-toolkit <plugin>`
still runs it. A table for a known plugin that is not in this build (and
not `run = false`) prints one notice in all-plugins mode,
`pin-toolkit: skipped (plugin helm not in this build)`, exit unaffected.

`ignore` patterns match a reference's key as a whole: `*` matches any run
of characters (including `/` and none), `?` one character.

- images, services, components: the value as written, quotes stripped.
  `ignore = ["alpine:3.20"]` therefore does not match an already pinned
  `alpine:3.20@sha256:…`; use `alpine:3.20*`.
- includes: `{project}@{ref}`, e.g. `gitlab-org/example@main` or
  `gitlab-org/example@*`. An include ref that cannot be rewritten (block
  or multi-line scalar) has just the ref as its key, so `g/p@*` keeps its
  `skipped (block scalar)` row.

An ignored reference has no row and is never resolved.

### Global config file

`$XDG_CONFIG_HOME/pin-toolkit/config.toml` — only when `XDG_CONFIG_HOME`
is set to a non-empty absolute path — else
`$HOME/.config/pin-toolkit/config.toml` (the same on macOS); if neither
variable is set, there is no global layer. A missing file is simply no
layer; an unreadable or invalid one exits `2` naming the path (shown
relative to the current directory, like the repo file).

The file may set only the scalar keys `version`, `json` and
`concurrency` and the `[hosts]` table — not `update`, `release_age` or
`[plugins]` (those are repository-level; an unknown key exits `2`):

```toml
version = 1              # required, must be 1

concurrency = 2          # = --concurrency (default 8)
json = true              # = --json

[hosts.<host>]           # one table per host, a plain host with optional :port
kind = "gitlab"          # "gitlab" | "helm-repo" | "oci"
```

Each `[hosts.<host>]` table carries exactly the keys of its kind:

- `kind = "gitlab"`: `token-env = "ENV"` — the **name** of an
  environment variable that holds the token; its value is sent as
  `PRIVATE-TOKEN` to that host (see
  [GitLab hosts and auth](#gitlab-hosts-and-auth)).
- `kind = "helm-repo"`: `username-env = "ENV"` and
  `password-env = "ENV"` — variable names for a helm repository
  (validated and shown by [`config show`](#config-show); consumed by
  the helm plugin, not yet built).
- `kind = "oci"`: `ca-file = "PATH"` — a CA bundle for an OCI registry
  (validated and shown; consumed by a later milestone).

Only environment-variable **names** may appear in the file, never
secret values: the HM module generates this file into the
world-readable Nix store.

A binding grants a **credential, not a lookup**: the host still must be
the primary host or in `plugins.gitlab-ci.gitlab-hosts` to be resolved
at all. Environment tokens win over bindings — `GITLAB_TOKEN` still goes
to the primary host, `CI_JOB_TOKEN` still to `CI_SERVER_HOST` only.

Exit `2` (the message names the file) for: a missing `version` or one
other than `1`; an unknown key (including `update`, `release_age` and
`plugins`) or host kind; a key that does not belong to the table's kind;
and a host key that is not a plain host
(``invalid GitLab host `x` in hosts``).

### `config show`

`pin-toolkit config show` prints the effective configuration without
walking the repository: the two config-file paths (or `none`), the
scalar settings, every plugin's settings (defaults included for
compiled-in plugins; only set keys for plugins not in this build) and
the `[hosts]` bindings. It prints environment-variable *names* from
`[hosts]`, never their values.

`--origin` appends the layer each value comes from — `default`,
`global`, `repo`, `env` or `cli`; for `ignore` — the only key whose
layers concatenate — it lists all contributing layers in layer order,
joined by `+` (today only the repo layer can set it, so in practice
`# repo`). `gitlab-host` shows `# env` when the primary host
comes from `GITLAB_URI` or `CI_SERVER_HOST`. Invalid
`CI_SERVER_HOST`/`GITLAB_URI` values still exit `2`, and
`--only`/`--skip` cannot be combined with `config show` (exit `2`).

```console
$ pin-toolkit config show --origin
config.global-file = .config/pin-toolkit/config.toml  # global
config.repo-file = .pin-toolkit.toml  # repo
update.enabled = true  # repo
update.major = false  # default
release-age.min-days = 0  # default
release-age.unknown = allow  # default
concurrency = 2  # global
json = false  # default
gitlab-host = gitlab.com  # default
plugins.gitlab-ci.run = true  # default
plugins.gitlab-ci.files = [".gitlab-ci.yml", "**/*.gitlab-ci.yml", ".gitlab/ci/**/*.yml"]  # default
plugins.gitlab-ci.ignore = ["registry.example.com/legacy/*"]  # repo
plugins.gitlab-ci.kinds.image = true  # default
plugins.gitlab-ci.kinds.service = true  # default
plugins.gitlab-ci.kinds.component = true  # default
plugins.gitlab-ci.kinds.include = true  # default
plugins.gitlab-ci.gitlab-hosts = ["gitlab.example.com"]  # repo
hosts.charts.example.com.kind = helm-repo  # global
hosts.charts.example.com.username-env = CHARTS_USER  # global
hosts.charts.example.com.password-env = CHARTS_PASSWORD  # global
hosts.gitlab.example.com.kind = gitlab  # global
hosts.gitlab.example.com.token-env = GITLAB_EXAMPLE_TOKEN  # global
hosts.registry.example.com.kind = oci  # global
hosts.registry.example.com.ca-file = /etc/ssl/certs/corp-ca.pem  # global
```

(Without `--origin` the same lines print without the `# …` comments.
Paths are shown relative to the current directory — in this example
`$HOME` is the repository root itself.)

### Environment

| Variable | Setting | Values |
|---|---|---|
| `PIN_TOOLKIT_UPDATE`, `PIN_TOOLKIT_UPDATE_MAJOR`, `PIN_TOOLKIT_JSON` | `--update`, `--update-major`, `--json` | boolean: `1/0`, `true/false`, `yes/no`, `on/off` (any case) |
| `PIN_TOOLKIT_MIN_AGE_DAYS` | `--min-age-days` | number |
| `PIN_TOOLKIT_RELEASE_AGE_UNKNOWN` | `release_age.unknown` | `allow`, `block` (any case) |
| `PIN_TOOLKIT_CONCURRENCY` | `--concurrency` | number |
| `PIN_TOOLKIT_ONLY`, `PIN_TOOLKIT_SKIP` | `--only`, `--skip` | comma list of plugin ids (as on the CLI) |
| `PIN_TOOLKIT_GITLAB_HOST` | `--gitlab-host` | host |
| `PIN_TOOLKIT_CONFIG` | repo config path (below `--config`) | path |

- An empty value counts as unset.
- A bad value exits `2`: `PIN_TOOLKIT_UPDATE: expected a boolean (1/0, true/false, yes/no, on/off), got "maybe"`,
  `…: expected a number, got "x"`, `…: expected allow or block, got "x"`,
  ``PIN_TOOLKIT_GITLAB_HOST: invalid GitLab host `x` ``.
- Any other `PIN_TOOLKIT_*` variable only warns, exit unaffected
  (GitLab exports every CI/CD variable to the job):
  `pin-toolkit: ignoring unknown environment variable PIN_TOOLKIT_VERSION`.
- `PIN_TOOLKIT_ONLY`/`PIN_TOOLKIT_SKIP` are **ignored** with a plugin
  subcommand or alias, so a CI-wide value cannot break `pin-ci`. In
  all-plugins mode an unknown id exits `2`, as on the CLI.

Credentials are read from the environment too, see
[GitLab hosts and auth](#gitlab-hosts-and-auth) and
[Per-host logins](#per-host-logins). Nothing secret is written to files or
logs.

## Plugin: gitlab-ci

### Tag updates (`--update`)

For every image reference (any registry host), the tag list is fetched
from its registry and the tag is bumped to the **highest stable tag
compatible with the current one** (docker versioning: pre-release/beta/nightly tags are
ignored). Compatibility means the same dash-suffix (e.g. `-alpine`) and the same
number of release components (`3.20` ↔ `3.24`, `24.04` ↔ `26.04`; `24` is
not compatible with `24.0`). By default the major must not change;
`--update-major` lifts that restriction. The bumped tag is then pinned as
`repo:new-tag@sha256:<digest>` in the same rewrite.

A reference is never bumped (digest/SHA re-pin only) when:

- its tag is not semver-parseable (`latest`, `main`, `stable`, `rolling`),
- it has no tag (treated as `latest`),
- it is already pinned — an existing `@sha256:<old>` is only re-pinned to
  the digest of the tag it carries.

If the tag list cannot be fetched, the reference is reported as
`unresolved (tag list: <reason>)`, the tool exits `1`, and all other
references in the file are still written.

#### Differences from the old pipeline

Two deliberate divergences from `pin-ci-images.sh` (renovate + crane):

- **Same major by default.** Renovate bumps to the newest tag of any major;
  pin-toolkit keeps the current major unless `--update-major` is given.
- **Release-age guard is opt-in.** Renovate's config sets
  `minimumReleaseAge: 14d`; by default pin-toolkit has no such guard and will
  pick a just-published stable tag — `--min-age-days 14` restores parity
  (see [Release age guard](#release-age-guard)). With the
  guard off, the M2 oracle accepts a tag divergence from the old pipeline
  only as a versioning-policy difference (both tags crane-verified; e.g.
  renovate's `ubuntu` versioning treats odd-minor interludes as unstable).

### Release age guard

`--min-age-days <N>` (default `0` = off) makes `--update` wait for newly
published tags: a bump candidate must have been pushed at least N days
before the run, per its Docker Hub push timestamp (`tag_last_pushed`,
fallback `last_updated`).

- **Default off.** Without the flag, pin/update behaviour is unchanged —
  the newest stable tag is picked even if it was pushed minutes ago.
- **Docker Hub refs only.** Timestamps exist only for `docker.io`
  references (including the no-slash default).
- **Other hosts: `release_age.unknown`.** Their age is unknown. With
  `allow` (the default) they update as without the guard. With `block`
  (`[release_age] unknown = "block"` or `PIN_TOOLKIT_RELEASE_AGE_UNKNOWN=block`)
  and `N > 0`, a reference with an update candidate is left untouched:
  `skipped (release age unknown (<host>; release_age.unknown = block))`.
  Without a candidate it is pinned as usual.
- **Recommended: `14`.** The old pipeline (`pin-ci-images.sh`, renovate)
  sets `minimumReleaseAge: 14d` in its config — but that rule is scoped to
  `matchUpdateTypes: ['pinDigest','digest']`, so it did not apply to tag
  bumps (proven in the oracle run log); `--min-age-days 14` gates bumps
  too, matching renovate's documented intent.
- **No timestamp = excluded.** A tag missing from the Hub timestamp map
  (neither `tag_last_pushed` nor `last_updated`) is treated as NOT old
  enough (conservative; parity with renovate's `timestamp-required`).
- **No eligible candidate → skip.** When bump candidates exist but none is
  old enough, the reference is left byte-identical and reported as
  `skipped (release age: no candidate ≥ N d)`, exit `0`.
- **Hub API failures are loud.** A 404 repo yields an empty timestamp map
  (→ the skip above); other Hub errors (network, 429, malformed page) make
  the reference `unresolved (hub api: …)`, exit `1`.

### Components & includes

The same run resolves and pins:

- **component references** — a top-level (unindented)
  `component: <host>/<group…/project>/<name>@<version>` line. The version
  is resolved to a commit SHA (tag first, then branch) and rewritten as
  `@<40-hex sha>`, with the original version kept as a trailing comment:

  ```yaml
  component: gitlab.com/acme/widget@1.0.0
  # →
  component: gitlab.com/acme/widget@0d7a3b1c2e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b  # 1.0.0
  ```

- **classic includes with an explicit `ref`** — a top-level `include:`
  mapping (block or flow, any key order) carrying both `project` and
  `ref`. The ref is resolved on the primary GitLab host to a commit SHA
  and replaced, with the original ref kept as a trailing comment:

  ```yaml
  include:
    project: gitlab-org/example
    ref: v1.2.3
  # →
  include:
    project: gitlab-org/example
    ref: 0d7a3b1c2e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b  # v1.2.3
  ```

  In a flow include the comment goes **after the closing bracket** (a
  `#` inside `{ … }` would end the flow mapping):

  ```yaml
  include: { project: gitlab-org/example, ref: main }
  # →
  include: { project: gitlab-org/example, ref: 0d7a3b1c2e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b }  # main
  ```

  Several refs pinned on one line (e.g. a flow `include:` sequence) get
  **one** trailing comment with the originals in source order:
  `include: [{…, ref: <sha>}, {…, ref: <sha>}]  # main, dev` (fixed in
  M5c, R1; before: one comment per ref in reverse order).

  CRLF files: the trailing comment goes before `\r\n` on every line (fixed
  in M5c, R2; before: inside `\r\n` on line 1, dropped on later lines).

  Discovery reads the YAML structure (M5c-y): anchored values (`&a`,
  `!!str`) are pinned at their definition; aliases (`*a`), merge keys
  (`<<:`) and `!reference [...]` are not targets. CRLF files, children of
  anchored keys (`.base: &base`) and a first key after a BOM are
  discovered (R7, R8, R9; before: missed or
  `skipped (unparseable reference)`). Also fixed (R10): flow
  `image: { name: x }` is pinned (before: the whole flow text was an
  unparseable target), `services: [a:1, {name: b:1, alias: x}]` and
  multi-line flow includes are found, and a value followed by
  `\t# comment` no longer includes the comment.

  A line that already has a trailing comment is left as-is (no second
  comment is added).

#### What gets skipped

Skipped targets are left untouched and keep the exit code `0`:

- **host not enabled** — a component reference whose host is neither the
  primary GitLab host nor listed in `plugins.gitlab-ci.gitlab-hosts`:
  `skipped (host not enabled: <host>)`. A classic include's `project` is a
  project path on the primary host, so includes are always eligible.
- **catalog-managed versions** — component versions that GitLab's CI/CD
  Catalog resolves at pipeline time and that have no tag/branch to pin
  against: versions starting with `~` (e.g. `~latest`), containing a
  `.x` wildcard (e.g. `1.0.x`), or a **partial** numeric version (all
  digits, fewer than 3 dot-separated components: `1`, `1.0`):
  `skipped (catalog-managed version)`. **Full versions resolve as tags** —
  including full numeric semvers like `1.0.0`; if no such tag (or branch)
  exists the reference is `unresolved` (fail loud). Documented edges:
  `2024` (one numeric component) is catalog-managed; `1.2.3.4` is treated
  as a tag name.
- **templated references** — a ref value containing `$` (e.g.
  `ref: ${CI_REF}`): `skipped (templated reference)`.
- **unparseable references** — an image value that does not parse as an
  image reference (e.g. `$CI_REGISTRY_IMAGE:1`), or a component with no
  `@`, too few path segments, or an empty version:
  `skipped (unparseable reference)`.
- **block scalars** — a `|` / `>` value at an image, component or include
  path is never rewritten: `skipped (block scalar)`.
- **multi-line scalars** — a quoted or plain value continued on the next
  line is never rewritten: `skipped (multi-line scalar)`.
- **no safe place for a comment** — the `# ref` comment of a pinned
  component or include would land inside another multi-line scalar of the
  same flow mapping (a flow include whose `file:` value continues on the
  next line): nothing is written for it,
  `skipped (no safe place for comment)`.
- **release age** — see [Release age guard](#release-age-guard).

On a flow line with several includes where only some are pinned, the
combined `# …` comment lists only the pinned refs; a later run treats the
line as commented and adds none for the rest.

Never targets at all (not even reported): include mappings with
`project` but no `ref` (bare `{project}`); include items that are plain
local templates (`"local-template.yml"`), `remote:` URLs, or `template:`
names; indented (job-level) `component:` / `include:`.

#### GitLab hosts and auth

- **Primary host:** `--gitlab-host` > `PIN_TOOLKIT_GITLAB_HOST` >
  `GITLAB_URI` > `CI_SERVER_HOST` > `gitlab.com`. Includes resolve there.
  A repository config cannot set it.
- **`GITLAB_URI`:** a URL for the primary host; the scheme (any case) is
  optional and the value is reduced to `host[:port]`
  (`https://gitlab.example.com` → `gitlab.example.com`). A value with
  credentials (`user@host`) is rejected (``GITLAB_URI: a GitLab URL must
  not contain credentials``); an invalid value exits `2` before any
  request, even when `--gitlab-host` is also given (``GITLAB_URI:
  invalid GitLab host `x` ``); like `CI_SERVER_HOST` it never carries
  `CI_JOB_TOKEN`; and like it, a URL under a relative URL root
  (`https://ex.com/gitlab`) makes `ex.com` the primary.
- **Resolved hosts:** the primary plus `plugins.gitlab-ci.gitlab-hosts`;
  components on other hosts are `skipped (host not enabled: …)`.
  `CI_SERVER_HOST` is not added implicitly when it is not the primary.
- **`GITLAB_TOKEN`** is sent as `PRIVATE-TOKEN` to the primary host only.
- **`CI_JOB_TOKEN`** is sent as `JOB-TOKEN` to `CI_SERVER_HOST` only —
  never to gitlab.com or another host. When both tokens apply to the same
  host, `GITLAB_TOKEN` wins. With the job token, branch resolution uses
  the list endpoint `GET /projects/:id/repository/branches?search=<name>`
  plus an exact-name match (the single-branch endpoint is not in the
  job-token allowlist); PAT/anonymous use the single-branch endpoint.
- Every other host is resolved **anonymously** (public projects work),
  unless the [global config file](#global-config-file) binds it:
  `[hosts.<host>]` with `kind = "gitlab"` and `token-env = "ENV"` sends
  the value of the named variable as `PRIVATE-TOKEN` to that host (a
  binding whose variable is unset or empty grants nothing). A
  binding grants a credential, not a lookup — the host still must be the
  primary or in `gitlab-hosts` to be resolved. Environment tokens win
  over bindings (`GITLAB_TOKEN` to the primary, `CI_JOB_TOKEN` to
  `CI_SERVER_HOST` only).
- A host value (`--gitlab-host`, `PIN_TOOLKIT_GITLAB_HOST`, `gitlab-hosts`
  entries, and a non-empty `CI_SERVER_HOST`) must be a plain host with an
  optional port: dot-separated labels of letters, digits and `-` (no empty
  label, no label starting or ending with `-`), then optionally `:port`
  with port 1–65535. Anything else (a URL, `user@host`, a path) exits `2`
  before any request: ``--gitlab-host: invalid GitLab host `x` ``,
  ``invalid GitLab host `x` in plugins.gitlab-ci.gitlab-hosts``,
  ``CI_SERVER_HOST: invalid GitLab host `x` `` (no fallback to
  `gitlab.com`).

Resolution failures are reported per reference and never abort the run —
other targets in the file are still written:

- no tag and no branch for the ref:
  `unresolved (<project>:<ref> not found (no tag or branch))`
- `401`/`403`:
  `unresolved (gitlab api: <status> unauthorized (check GITLAB_TOKEN/CI_JOB_TOKEN))`
- `429`: `unresolved (gitlab api: rate limited (429), retry later)` —
  **no retry**; run again later
- other API/network failures: `unresolved (gitlab api: …)`
- a failed lookup on a host that has **no credential** — not the
  primary with `GITLAB_TOKEN`, not `CI_SERVER_HOST` with `CI_JOB_TOKEN`,
  and no non-empty `[hosts]` binding — for **any** error (this replaces
  the `gitlab api: …` reasons above, including `429` and network
  failures):
  `unresolved (gitlab.example.com: not trusted for credentials — bind it
  in /home/user/.config/pin-toolkit/config.toml)`; that path is
  `$XDG_CONFIG_HOME/pin-toolkit/config.toml` when set, else
  `$HOME/.config/pin-toolkit/config.toml` (or `~/.config/…` when neither
  variable is set)

### Multi-registry support

Image pinning works on **every docker-v2 registry host** — `docker.io`,
`ghcr.io`, GCP Artifact Registry, ECR (public and private), JFrog
Artifactory, and any self-hosted registry.

#### Per-host logins

| Registry host | How to authenticate |
|---|---|
| `docker.io` | `DOCKERHUB_USER` + `DOCKERHUB_TOKEN` (env, used first) or `~/.docker/config.json` |
| `ghcr.io` | `GITHUB_TOKEN` (a PAT with `read:packages`), sent as Bearer on the token request, or config.json |
| GCP Artifact Registry (`<region>-docker.pkg.dev`) | `gcloud auth configure-docker <host>` — writes an `identitytoken` entry to config.json, sent as Bearer |
| ECR public (`public.ecr.aws`) | none — anonymous |
| ECR private (`registry-…amazonaws.com`) | `docker login -u AWS -p "$(aws ecr get-login-password …)" registry-…` — lands in config.json. SigV4 credential minting is deliberately **not** in the tool |
| JFrog Artifactory / self-hosted | `docker login <host>` — lands in config.json; both `Basic` and `Bearer` challenges are handled |

config.json (`$HOME/.docker/config.json`) entries resolve per host, `auths`
keys in order `https://index.docker.io/v1/`, `https://docker.io`,
`index.docker.io/v1/`, `docker.io` for Docker Hub; `ghcr.io` /
`https://ghcr.io` for GitHub; `{host}` / `https://{host}` for anything
else. An entry's `username` + `password` are Basic credentials, otherwise
an `identitytoken` is a Bearer token. Precedence is per host: env vars
first, then config.json. A missing or malformed config.json is never
fatal — the host simply has no credentials. A registry token is only ever
sent to the host it was issued for.

#### Fail loud (no silent skips)

When a registry demands auth — anonymous 401 with the token request
denied, or a `Basic` challenge — and no credentials are configured for
that host, the reference is `unresolved` with a login hint and the exit
code is `1`:

- `ghcr.io`: `unresolved (ghcr.io: token denied — set GITHUB_TOKEN (PAT with read:packages) or docker login ghcr.io)`
- other hosts: `unresolved (<host>: token denied — docker login <host>)`; a `Basic` challenge with no (or Bearer-only) credentials reports `unresolved (<host>: registry requires basic login — docker login <host>)`

Everything else in the file is still resolved and written (partial-write
semantics).

## Plugin: helm

Known plugin id, **not built yet**: `pin-toolkit helm` (and the
`pin-helm` alias) exit `2` with `plugin helm not in this build`. Its
config table in `.pin-toolkit.toml` is accepted and validated; planned
kinds: `chart-dependency` (Chart.yaml dependencies), `values-image`
(values files), `helmfile-release` (helmfile).

## Plugin: github-actions

Known plugin id, **not built yet**: `pin-toolkit github-actions` exits
`2` with `plugin github-actions not in this build`. Its config table is
accepted and validated; planned kinds: `action`, `docker-action`,
`container`.

## Tests

- `scripts/nix-check.sh` — the Nix gate: refuses any untracked file in the repository
  (Nix sees tracked files only), then `nix flake check -L` (builds and runs the
  checks for the current system) and `nix flake check --no-build --all-systems`
  (evaluates the other systems). CI's `nix` job runs the same checks on
  Linux and macOS runners; `advisories` runs `cargo deny check advisories`.
- `scripts/gate.sh` — the repository gate, run before every commit: the
  whole test suite with all features (`INSTA_UPDATE=no`, snapshots must
  match), `cargo fmt --check`, clippy `-D warnings` (workspace, the
  pin-core feature matrix, pin-toolkit `--no-default-features`), the
  pin-toolkit tests without plugins, the D38 checks (`cargo check -p
  pin-gitlab-ci -p pin-toolkit` and a `cargo tree` check that no normal
  dependency enables pin-core's `test-support`; `cargo tree` failing is a
  failure too) and `RUSTDOCFLAGS=-D warnings cargo doc`. Every cargo call
  is `--locked`. The tests must not leave `*.snap.new` files or change
  anything under `crates/` (compared with the state before them, so the
  gate also runs on uncommitted work). It uses the `cargo` on `PATH`;
  logs go to `.tmp/gate-*.log`.
- `cargo test --workspace --all-features` — unit, mock-registry and
  end-to-end tests. Always runs without network, registry credentials, or
  external tools. pin-toolkit's own tests (`crates/pin-toolkit/tests/`):
  `cli.rs`, `config_layers.rs`, `run.rs`, `exit_codes.rs` (the exit-code
  table), `credentials.rs` (tokens on the wire, wiremock), `aliases.rs`
  (argv[0] aliases through the real binary, unix), `fixture_snapshots.rs`
  (human + JSON snapshots over the gitlab-ci fixtures).
- `PIN_CI_LIVE=1 cargo test -p pin-toolkit --test live` — opt-in live smoke
  of the real binary: pins `alpine:3.20` from Docker Hub, then `--check`
  exits `0`.
- `PIN_CI_LIVE=1 cargo test -p pin-gitlab-ci --test live` — opt-in live tests against real
  registries: Docker Hub digest resolution for `alpine:3.20` /
  `ubuntu:24.04` plus a full `--update --update-major` run whose tag
  choice is checked against an independent `list_tags` +
  `pick_latest_stable` recomputation and whose digests are checked
  against `crane digest` (a second run must change nothing); a guarded
  `--update --min-age-days 14` run on `alpine:3.19` that must update to a
  ≥ 14-days-old 3.x stable tag (learned live, not frozen) and pin its
  digest, cross-checked against the real Hub push timestamps and `crane
  digest` (skips with a hint when `crane` is missing); ECR public
  anonymous resolution of `public.ecr.aws/nginx/nginx:latest`,
  cross-checked against `crane digest` (skips with a hint when `crane`
  is missing); and a GHCR public-package resolution gated on
  `GITHUB_TOKEN` (skips with a hint otherwise), likewise cross-checked
  against `crane digest`.
- `PIN_CI_LIVE=1 cargo test -p pin-core --all-features --test live_gitlab` — opt-in
  live GitLab ref resolution against gitlab.com (cross-checked with `git ls-remote`).
- `PIN_CI_ORACLE=1 cargo test -p pin-gitlab-ci --test oracle` — opt-in differential tests
  against the old pipeline, which runs `pin-ci-images.sh` as-is on a temp
  checkout:
  - M1 (pin-only): the new tool must pin exactly the same
    `name@sha256:...` digests the old pipeline wrote.
  - M2 (`--update --update-major` vs. the old pipeline, which bumps tags by
    default): per original reference position (line + repo), equal tags
    need byte-identical digests; a differing tag passes only as the
    age-explained divergence — both tags parse-stable-compatible and the
    new tool's strictly greater, AND the real Hub push timestamps show the
    old pick ≥ 14 days old while the new pick is younger (renovate's
    `minimumReleaseAge: 14d`) — with both digests sanity-checked via
    `crane digest`. A missing Hub timestamp fails loudly (a data bug,
    never a silent pass); anything else fails.

  Both require `renovate`, `crane`, `python3` and `git` on `PATH` (and
  outbound network); without the env flag or the tools they print a SKIP
  notice and pass.

The pin-gitlab-ci live and oracle tests drive the plugin through a test
helper (`crates/pin-gitlab-ci/tests/common/driver.rs`, the former `pin-ci`
driver), not through the binary.

## Old pipeline

`pin-ci-images.sh`, `extract-docker-pins.py`, and `config.js` are kept
unmodified at the repo root and serve as the correctness oracle (see
`PIN_CI_ORACLE=1` above). Their retirement is a later decision.

## License

MIT OR Apache-2.0 — see [LICENSE](LICENSE).
