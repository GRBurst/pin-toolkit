# Plan: m6-nix-release
<!-- task-router plan v1. Edit only checkboxes; change the plan with amend_plan. -->
Request: Implement M6 of docs/superpowers/specs/2026-11-09-pin-toolkit-design.md (S10 M6 row, S8.1-S8.4, O1/O3, S14.1 DF1/M6): Nix flake with crane (packages incl. static musl + container + release tarballs), flake checks mirroring scripts/gate.sh plus cargo-deny, Home Manager module, GitHub Actions + Cachix, tagged releases + GHCR image. The design with every decision, pinned action SHAs and the spike-verified reference files (Appendix A) is .superpowers/sdd/2026-11-11-m6/design.md — READ IT FIRST; copy Appendix A files, do not redesign them. Branches stack in milestone order (M1 <- M2 <- ... <- M7); the owner merges each by hand (the `depends` fields encode this linear stack). No tag is pushed and no release is made in M6 (brief decision 9).
Invariants:
  - Nix sees git-tracked files only: every nix check stages only its own task's source paths first (`git add -A -- <paths>`, never a bare `git add -A`: it would sweep the untracked plan file, checkbox ticks and stray out-links into the task commit) and ends with `git diff --cached --quiet`; checks run on the committed task, so a file left out of the commit is red. The orchestrator commits this plan file before execution. Never evaluate the repo as `path:.` (copies .tmp/ and target/ into the store)
  - Sandbox: every nix command runs with `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache` (the main checkout's cache, shared by worktrees); only x86_64-linux builds locally; aarch64-linux and aarch64-darwin are evaluated locally (`nix flake check --no-build --all-systems`) and built only in GitHub CI
  - bash scripts/gate.sh passes on every branch with the developer's usual toolchain (tests, fmt, clippy -D warnings, no-default-features, D38, rustdoc); from M2.T3 on it also passes under `nix develop` (the flake toolchain, rustc 1.99.1)
  - No product behaviour change: the only Rust edits are the #[allow(clippy::result_large_err)] on the test helper pin_files (M2.T3) and `publish = false` in the 3 crate manifests (M2.T5); existing tests and snapshots unchanged (INSTA_UPDATE=no)
  - allPlugins comes from crates/pin-toolkit/Cargo.toml [features] minus default (nix/plugins.nix) and nowhere else; checks derive expectations from passthru.plugins/version, not literals
  - Every `uses:` is pinned by a 40-hex commit SHA with a `# vX` comment (design §5.1); workflow-level permissions are {} and every job declares its own; secrets are never echoed; Cachix pushes, and the Cachix auth token, only on push events (main, v* tags); the release jobs (binaries, images) use no Cachix; runner labels are pinned (ubuntu-24.04, ubuntu-24.04-arm, macos-15), no *-latest
  - The HM module writes only the global config schema (version=1 injected, json, concurrency, [hosts]); env-var names only, never secret values
  - Every new check is seen RED once with its planted violation (listed in the task) before it is accepted; note "planted: <what> -> red seen" in the commit body; revert the plant
  - Files created by an earlier task (flake.nix, nix/checks.nix, nix/hm-module.nix, .github/workflows/ci.yml, …) are edited, not recreated, by later tasks listing them in create
  - Each milestone diff stays reviewable (target <= 400 changed lines, flake.lock excluded)

## M1: Flake skeleton: packages.{default,pin-toolkit} via nix/package.nix (callPackage, plugins ? allPlugins), layout property checks per plugin set, overlays.default, devShells.default, scripts/nix-check.sh (~230 lines incl. flake.lock 65). (branch: milestone; depends: —)
Acceptance: nix build .#pin-toolkit -> `pin-toolkit --version` = "pin-toolkit <workspace.package.version>"; layout(v) holds for v in {default, override [], override [p] for each plugin}; an unknown plugin fails evaluation; overlay drvPath == packages.pin-toolkit drvPath; scripts/nix-check.sh green (x86_64 build + all-systems eval) and refuses untracked source files; gate.sh green
- [x] M1.T1: Create flake.nix (inputs nixpkgs nixpkgs-unstable, crane v0.24.0, home-manager follows nixpkgs; systems x86_64-linux aarch64-linux aarch64-darwin via lib.genAttrs; packages.<sys>.{default,pin-toolkit} only), flake.lock copied verbatim from the spike file <main checkout>/.tmp/m6-spike/ws/flake.lock (not retyped from markdown; same content as design Appendix A: nixpkgs 5d2b81a, crane 692f7e9, home-manager a1293da; the check asserts the three revs), nix/plugins.nix, nix/common.nix (fileset Cargo.toml Cargo.lock crates pin-ci-images.sh; deps = buildDepsOnly --workspace --all-features), nix/package.nix (doCheck=false; symlinks pin-<p> and pin-ci iff gitlab-ci; passthru plugins/allPlugins; meta.mainProgram; assertMsg on unknown plugins); .gitignore `/result*` and `/dist`.
  files: flake.nix (new), flake.lock (new), nix/plugins.nix (new), nix/common.nix (new), nix/package.nix (new), .gitignore · forbidden: rust-overlay, flake-utils or a rust-toolchain file; crane cleanCargoSource (drops fixtures/snapshots); listing plugin names anywhere but Cargo.toml; outputs beyond packages in this task; editing flake.lock by hand or running nix flake update/lock (copy the spike file) · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix flake.lock nix .gitignore && nix flake metadata --json | jq -e '.locks.nodes.nixpkgs.original.ref == "nixpkgs-unstable" and .locks.nodes.nixpkgs.locked.rev == "5d2b81a7a3be3e2f832879028e4d60abbf58e9a0" and .locks.nodes.crane.locked.rev == "692f7e9ef2ece8125b466f66f2af532b3edaed0d" and .locks.nodes["home-manager"].locked.rev == "a1293da7dd8e497e08d26173f97e2976b36a31ea"' > /dev/null && out=$(nix build --no-link --print-out-paths .#pin-toolkit) && v=$(nix eval --raw .#pin-toolkit.version) && [ "$v" = "$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml)" ] && "$out/bin/pin-toolkit" --version | grep -qx "pin-toolkit $v" && git diff --cached --quiet` · depends: —
- [x] M1.T2: Create nix/layout.nix (property over a build v: ls bin = {pin-toolkit} ∪ {pin-p | p ∈ v.plugins} ∪ {pin-ci | gitlab-ci ∈ v.plugins}; aliases are symlinks to pin-toolkit; enabled lines of `pin-toolkit plugins` = v.plugins; --version) and nix/checks.nix with pin-toolkit = pkg, plugins-none and plugins-<p> for every p (layout of each override); wire checks in flake.nix. Planted: delete the pin-ci symlink line in package.nix -> plugins-gitlab-ci red "bin: [pin-gitlab-ci pin-toolkit ] expected [pin-ci pin-gitlab-ci pin-toolkit]".
  files: flake.nix (new), nix/package.nix (new), nix/layout.nix (new), nix/checks.nix (new) · forbidden: hard-coding the expected bin list or plugin names (derive from passthru.plugins); running tests in package builds · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix nix && nix build --no-link -L .#checks.x86_64-linux.pin-toolkit .#checks.x86_64-linux.plugins-none .#checks.x86_64-linux.plugins-gitlab-ci && git diff --cached --quiet` · depends: M1.T1
- [x] M1.T3: Add overlays.default (final.callPackage ./nix/package.nix { craneLib = crane.mkLib final; plugins = import ./nix/plugins.nix final.lib; }: explicit, so a consumer overlay's own `plugins` attribute cannot leak in; drvPath unchanged), eval assertions on checks.pin-toolkit ((pkgs.extend overlay).pin-toolkit.drvPath == pkg.drvPath; tryEval of override { plugins = [ "no-such-plugin" ]; } fails), devShells.default (crane devShell, checks as inputsFrom, + cargo-nextest cargo-deny actionlint), scripts/nix-check.sh (design Appendix A: untracked-source guard, nix flake check -L, nix flake check --no-build --all-systems). Planted: tryEval with "gitlab-ci" -> red "unknown plugin accepted"; untracked nix/x.nix -> nix-check.sh exit 1.
  files: flake.nix (new), nix/checks.nix (new), scripts/nix-check.sh (new) · forbidden: an overlay that pins the flake's own nixpkgs (it must use final); building anything in nix-check.sh beyond flake check · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix nix scripts/nix-check.sh && bash scripts/nix-check.sh && nix develop -c cargo --version | grep -q '^cargo ' && git diff --cached --quiet` · depends: M1.T2

## M2: The cargo gate as flake checks: fmt, doc, nextest (all features) + nextest-no-default, clippy, feature-matrix, d38, deny (offline bans/licenses/sources); clippy fix for the newer toolchain; publish = false (~140 lines). (branch: milestone; depends: M1)
Acceptance: checks fmt doc nextest nextest-no-default clippy feature-matrix d38 deny green on x86_64-linux; clippy, nextest, nextest-no-default and doc install no cargo artifacts; nextest runs the whole workspace with 0 skips needed (402 tests at plan time) and fails if no test runs; each check was seen red with its planted violation; gate.sh green with the developer toolchain and under nix develop
- [x] M2.T1: Add checks fmt (craneLib.cargoFmt on common.src) and doc (cargoDoc, RUSTDOCFLAGS=-D warnings, doInstallCargoArtifacts = false: leaf-check outputs must not bloat Cachix). Planted: misformat one line in crates/pin-core/src/lib.rs -> fmt red; a broken intra-doc link -> doc red.
  files: nix/checks.nix (new) · forbidden: changing crates/ beyond the reverted plants · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix && nix build --no-link -L .#checks.x86_64-linux.fmt .#checks.x86_64-linux.doc && git diff --cached --quiet` · depends: —
- [x] M2.T2: Add checks nextest (--workspace --all-features via common args) and nextest-no-default (-p pin-toolkit --no-default-features), both cargoNextestExtraArgs "--no-tests=fail", env INSTA_UPDATE=no + CI=true, __darwinAllowLocalNetworking = true, doInstallCargoArtifacts = false (89 + 13 MB of useless artifacts otherwise). Planted: drop ../pin-ci-images.sh from the fileset -> oracle_repo_root_has_script red; a #[cfg(not(feature = "gitlab-ci"))] panicking test in crates/pin-toolkit/tests -> nextest-no-default red.
  files: nix/checks.nix (new), nix/common.nix (new) · forbidden: skipping or filtering tests (if one fails in the sandbox, stop and report: main allows only a short named skip list); network access; writing snapshots · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix && nix build --no-link -L .#checks.x86_64-linux.nextest .#checks.x86_64-linux.nextest-no-default && git diff --cached --quiet` · depends: M2.T1
- [x] M2.T3: RED first: add check clippy (cargoClippy, --all-targets -- -D warnings over --workspace --all-features, doInstallCargoArtifacts = false) and see it fail with clippy::result_large_err at crates/pin-gitlab-ci/tests/common/driver.rs:106. GREEN: `#[allow(clippy::result_large_err)] // test helper: the error carries the earlier rows (review L2)` directly above `pub async fn pin_files(`.
  files: nix/checks.nix (new), crates/pin-gitlab-ci/tests/common/driver.rs · forbidden: changing PinFilesError, pin_files' signature or any non-test code; workspace-wide lint allows · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix crates/pin-gitlab-ci/tests/common/driver.rs && nix build --no-link -L .#checks.x86_64-linux.clippy && nix develop -c env CARGO_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.cargo-home bash scripts/gate.sh && git diff --cached --quiet` · depends: M2.T2
- [x] M2.T4: Add checks feature-matrix (mkCargoDerivation, nativeBuildInputs [pkgs.clippy]; clippy -p pin-core --no-default-features for "" oci gitlab yaml oci,gitlab,test-support and -p pin-toolkit --no-default-features, --all-targets -D warnings) and d38 (cargo check -p pin-gitlab-ci -p pin-toolkit; cargo tree -e normal,build -f '{p} {f}' must not match pin-core.*test-support). Planted: `#[cfg(all(feature = "oci", not(feature = "gitlab")))] fn planted() {}` in pin-core -> feature-matrix red; features = ["test-support"] on pin-toolkit's normal pin-core dependency -> d38 red.
  files: nix/checks.nix (new) · forbidden: diverging from the feature sets in scripts/gate.sh · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix && nix build --no-link -L .#checks.x86_64-linux.feature-matrix .#checks.x86_64-linux.d38 && git diff --cached --quiet` · depends: M2.T3
- [x] M2.T5: RED first: add check deny (craneLib.cargoDeny, src = common.src + deny.toml) with deny.toml from design Appendix A (explicit [advisories] table: unmaintained = "workspace", yanked = "deny"; the offline check ignores it, CI's advisories job uses it); see error[wildcard] / bug[unresolved-workspace-dependency]. GREEN: `publish = false` after `license.workspace = true` in crates/{pin-core,pin-gitlab-ci,pin-toolkit}/Cargo.toml. Planted: drop "ISC" from licenses.allow -> red.
  files: nix/checks.nix (new), deny.toml (new), crates/pin-core/Cargo.toml, crates/pin-gitlab-ci/Cargo.toml, crates/pin-toolkit/Cargo.toml · forbidden: advisories in a flake check (network); allow-listing a license not reported by cargo-deny; touching dependencies or Cargo.lock · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix deny.toml crates/pin-core/Cargo.toml crates/pin-gitlab-ci/Cargo.toml crates/pin-toolkit/Cargo.toml && nix build --no-link -L .#checks.x86_64-linux.deny && bash scripts/gate.sh && git diff --cached --quiet` · depends: M2.T4

## M3: Linux artefacts: packages.pin-toolkit-static (crane on pkgsStatic), container (buildLayeredImage), dist (release tar.gz + .sha256) with checks static, container, dist (~110 lines). (branch: milestone; depends: M2)
Acceptance: static binary is static(-pie) linked and satisfies layout; image has no Entrypoint/Cmd, SSL_CERT_FILE, arch matching the system, sh/grep/pin-toolkit/CA bundle inside, < 20 MB; dist holds exactly pin-toolkit-v<ver>-<triple>.tar.gz + .sha256 with LICENSE, README.md and every bin entry; outputs absent on darwin (packages.aarch64-darwin attrNames = [default, pin-toolkit]); nix-check.sh green
- [x] M3.T1: Add packages.<linux>.pin-toolkit-static = pkgs.pkgsStatic.callPackage ./nix/package.nix { craneLib = crane.mkLib pkgs.pkgsStatic; } and Linux-only check static (`file -L` matches 'static-pie linked\|statically linked'; layout of the static build). Planted: build it with pkgs instead of pkgsStatic -> static red.
  files: flake.nix (new), nix/checks.nix (new) · forbidden: rust-overlay or pkgsCross unless pkgsStatic fails on a system (then stop and report); exposing static/container/dist on darwin · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix nix && nix build --no-link -L .#checks.x86_64-linux.static && git diff --cached --quiet` · depends: —
- [x] M3.T2: Create nix/container.nix (design Appendix A: static pin-toolkit, pkgsStatic.busybox, dockerTools.caCertificates, /tmp 1777, Env PATH=/bin + SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt, OCI labels, no Entrypoint/Cmd), packages.container, check container (manifest/config via jq incl. architecture amd64|arm64 per system; in-image symlink resolution of /bin/sh /bin/grep /bin/pin-toolkit and the CA bundle; busybox `sh -c`; --version; size < 20 MB). Planted: Entrypoint = [ "/bin/pin-toolkit" ] -> container red.
  files: flake.nix (new), nix/checks.nix (new), nix/container.nix (new) · forbidden: Entrypoint or Cmd; a dynamically linked binary or busybox; running docker/podman in checks · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix nix && nix build --no-link -L .#checks.x86_64-linux.container && git diff --cached --quiet` · depends: M3.T1
- [x] M3.T3: Create nix/dist.nix (runCommand: pin-toolkit-v<version>-<pkgsStatic hostPlatform.config> dir with `cp -P` of static bin/*, LICENSE, README.md; deterministic tar --sort=name --mtime=@1 --owner=0 --group=0 --numeric-owner; .sha256 via sha256sum), packages.dist, check dist (exact file set, sha256sum -c, tar members = LICENSE README.md + ls static/bin). Planted: omit README.md from the tarball -> dist red.
  files: flake.nix (new), nix/checks.nix (new), nix/dist.nix (new) · forbidden: hard-coding version or triple (derive from pin-toolkit.version and pkgsStatic.stdenv.hostPlatform.config) · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix nix && nix build --no-link -L .#checks.x86_64-linux.dist && bash scripts/nix-check.sh && [ "$(nix eval --json .#packages.aarch64-darwin --apply builtins.attrNames)" = '["default","pin-toolkit"]' ] && git diff --cached --quiet` · depends: M3.T2

## M4: Home Manager module programs.pin-toolkit (O1, S8.2 as built M5e'): enable, package, plugins.<p>.enable, settings (global config), read-only finalPackage, xdg config file validated by the binary, assertions; outputs homeManagerModules.default + homeModules.default; check hm-module on all systems (~130 lines). (branch: milestone; depends: M3)
Acceptance: all plugins enabled -> finalPackage == flake default (same store path); a disabled plugin -> override with the enabled set; enabled plugin missing from package.passthru.plugins and repo-only settings keys fail evaluation with their specific assertion messages (and the default config has no failed assertion); the written config.toml has version = 1 and is accepted by `pin-toolkit config show`; option descriptions state the uncached trimmed build and the no-secrets rule; module evaluates on all 3 systems; homeManagerModules.default is a named module (_file); package default on an unlisted system throws a clear message
- [x] M4.T1: Create nix/hm-module.nix (self: module) with options enable, package (default self.packages.${system}.default or (throw "pin-toolkit: no package for ${system}; set programs.pin-toolkit.package")), plugins = genAttrs allPlugins { enable (default true) }, settings (pkgs.formats.toml {}), finalPackage (readOnly; package if naturalSort enabled == naturalSort package.passthru.plugins else package.override { plugins = enabled; }); config: home.packages = [ finalPackage ], xdg.configFile."pin-toolkit/config.toml".source = toml.generate ({ version = 1; } // settings). Outputs homeManagerModules.default = { _file = ./nix/hm-module.nix; imports = [ (import ./nix/hm-module.nix self) ]; } (named for error messages and dedup); homeModules.default alias. Check hm-module via home-manager.lib.homeManagerConfiguration (stateVersion 25.05): full -> finalPackage == pkg; gitlab-ci disabled -> finalPackage.passthru.plugins == [ ]; generated file contains `version = 1`. Planted: finalPackage always the override -> red "all plugins enabled: finalPackage must be the flake default".
  files: flake.nix (new), nix/checks.nix (new), nix/hm-module.nix (new) · forbidden: plugins.<name>.settings (dropped, design §1 #4); secret values in options or examples; reading the flake's nixpkgs for anything but the default package · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix nix && nix build --no-link -L .#checks.x86_64-linux.hm-module && git diff --cached --quiet` · depends: —
- [x] M4.T2: hm-module.nix assertions: enabled ⊆ package.passthru.plugins ("programs.pin-toolkit: plugin(s) … enabled but not in programs.pin-toolkit.package …"); settings ∩ {version, plugins, update, release_age} = ∅ (message points to .pin-toolkit.toml). Check negatives by evaluating the module with lib.evalModules and stub options (assertions, home.packages, xdg.configFile), reading the failed assertion messages: lib.hasInfix "enabled but not in programs.pin-toolkit.package" and "programs.pin-toolkit.settings: update not allowed" (not a bare tryEval: it is red for any error), plus the positive control failedAssertions { } == [ ]. Planted: remove the repo-only assertion -> red "repo-only key in settings: assertion message".
  files: nix/hm-module.nix (new), nix/checks.nix (new) · forbidden: silently dropping or rewriting user settings; a negative check that passes on any evaluation error (match the specific message) · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix && nix build --no-link -L .#checks.x86_64-linux.hm-module && git diff --cached --quiet` · depends: M4.T1
- [x] M4.T3: Validate the generated file at build time: the xdg source becomes a runCommand that copies it to xdg/pin-toolkit/config.toml and runs `XDG_CONFIG_HOME=$PWD/xdg ${lib.getExe cfg.finalPackage} config show > /dev/null` before copying to $out (not --no-config: it skips the global file). Check: copy the file, `config show` prints `concurrency = 4` and `hosts.gitlab.example.com.token-env = GITLAB_EXAMPLE_TOKEN`. Planted: drop `{ version = 1; } //` -> red "missing `version = 1`".
  files: nix/hm-module.nix (new), nix/checks.nix (new) · forbidden: validating with a second parser in Nix (the binary is the schema) · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix && nix build --no-link -L .#checks.x86_64-linux.hm-module && bash scripts/nix-check.sh && git diff --cached --quiet` · depends: M4.T2

## M5: CI for every push/PR: .github/workflows/ci.yml jobs nix (3-OS matrix, Cachix probe-guarded, push only on push events) and advisories; check workflows (actionlint + shellcheck, SHA pins, least-privilege permissions incl. top-level) (~110 lines). (branch: milestone; depends: M4)
Acceptance: workflows check green and seen red for an unpinned uses, for a job without permissions, for a missing top-level permissions and for a job with permissions: write-all; ci.yml nix job = checkout (persist-credentials false), install-nix-action, Cachix probe + guarded cachix-action (skipPush unless push; authToken only on push events), nix flake check -L, nix build .#pin-toolkit; advisories runs cargo deny check advisories via nix shell --inputs-from .; nix-check.sh green
- [x] M5.T1: Add check workflows (runCommand over lib.fileset .github/workflows; actionlint -no-color with shellcheck on PATH; grep that every uses: matches `uses: [^@ ]+@[0-9a-f]{40} # v[0-9]`; yq-go `(.permissions | tag) == "!!map" and (.permissions | length) == 0 and ([.jobs[] | .permissions | tag == "!!map"] | all)` (red when the top-level key is absent or a job uses a string like write-all)) and create .github/workflows/ci.yml with triggers, permissions {}, concurrency, env IMAGE, jobs nix and advisories exactly as design Appendix A. Cachix authToken is `${{ github.event_name == 'push' && secrets.CACHIX_AUTH_TOKEN || '' }}`; runner labels pinned ubuntu-24.04 / ubuntu-24.04-arm / macos-15. Planted: actions/checkout@v7 -> red; delete the advisories job's permissions -> red; delete the top-level `permissions: {}` -> red; `permissions: write-all` in a job -> red.
  files: nix/checks.nix (new), .github/workflows/ci.yml (new) · forbidden: tag-only refs (@v4) or unpinned actions; secrets in run: lines; cachix-action without the probe guard; pull_request_target; *-latest runner labels; the Cachix token on pull_request runs · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix .github/workflows/ci.yml && nix build --no-link -L .#checks.x86_64-linux.workflows && bash scripts/nix-check.sh && git diff --cached --quiet` · depends: —

## M6: Release path in ci.yml on v* tags: tag-version (tag == workspace version), create-release (draft once; fails on a published release; --prerelease for tags with `-`), binaries (nix build .#dist, actions/attest, gh release upload --clobber; no Cachix), images (skopeo push <ver>-<arch>; no Cachix), manifest (imagetools :<ver> + :latest, attest), publish (draft=false) (~130 lines). (branch: milestone; depends: M5)
Acceptance: workflows check green; jobs graph tag-version (tag only), create-release<-[nix, tag-version], binaries<-create-release, images<-create-release, manifest<-images, publish<-[binaries, manifest]; every release job gated by the tag (directly or via needs); every new job has timeout-minutes; permissions per design §5.3 (no artifact-metadata); binaries and images use no Cachix (attested bytes: source + cache.nixos.org only); re-runs of a draft are idempotent (view-or-keep-draft, --clobber); an already published release makes create-release fail; tags with "-" get --prerelease; :latest only for versions without "-"
- [x] M6.T1: ci.yml: add tag-version (if startsWith(github.ref, 'refs/tags/v'); contents: read; python3 tomllib check tag == v + workspace.package.version; timeout 10), create-release (same if; needs [nix, tag-version]; timeout 10; if `gh release view "$TAG"` succeeds and `--json isDraft` is not true -> ::error:: and exit 1, else keep the draft; otherwise gh release create "$TAG" --draft --verify-tag --generate-notes --title "$TAG" plus --prerelease when $TAG contains "-") and binaries (matrix ubuntu-24.04, ubuntu-24.04-arm; no Cachix; permissions contents/id-token/attestations write, no artifact-metadata; nix build -L .#dist --out-link dist; actions/attest subject-path dist/*.tar.gz; gh release upload "$TAG" dist/*.tar.gz dist/*.tar.gz.sha256 --clobber) as design Appendix A. Planted: create-release needs [nix, advisories] -> yq check red; a cachix-action step in binaries -> yq check red.
  files: .github/workflows/ci.yml (new) · forbidden: packaging shell in YAML (dist is a Nix output); `|| true` around release creation; actions/attest-build-provenance (design §1 #6a); Cachix in binaries or images; an artifact-metadata permission (not needed for a User-owned repo) · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- .github/workflows/ci.yml && nix build --no-link -L .#checks.x86_64-linux.workflows && nix shell --inputs-from . nixpkgs#yq-go -c yq -e '.jobs.binaries.needs == "create-release" and (.jobs.create-release.needs | sort | join(",")) == "nix,tag-version" and (.jobs.create-release.if | test("refs/tags/v")) and (.jobs.tag-version.if | test("refs/tags/v")) and ([.jobs.binaries.steps[].uses] | map(select(. != null) | test("cachix-action")) | any | not)' .github/workflows/ci.yml && git diff --cached --quiet` · depends: —
- [ ] M6.T2: ci.yml: add images (matrix amd64/ubuntu-24.04, arm64/ubuntu-24.04-arm; timeout 60; no Cachix; nix build .#container; skopeo from nix run --inputs-from . nixpkgs#skopeo, in a step whose env sets GH_TOKEN and REGISTRY_AUTH_FILE: ${{ runner.temp }}/containers-auth.json (step env, not job env: the runner context is unavailable there; no job-level GH_TOKEN): login ghcr.io --password-stdin, --insecure-policy copy docker-archive:result docker://$IMAGE:${TAG#v}-$ARCH), manifest (timeout 15, no artifact-metadata; docker login; buildx imagetools create -t :<ver> [+ :latest unless ver contains "-"]; digest = imagetools inspect --format '{{json .Manifest}}' | jq -r .digest; actions/attest subject-name/subject-digest/push-to-registry) and publish (timeout 10; gh release edit "$TAG" --draft=false).
  files: .github/workflows/ci.yml (new) · forbidden: :latest for pre-release versions; tokens on the command line (use --password-stdin); uppercase image names; Cachix in images; artifact-metadata permission · check: `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- .github/workflows/ci.yml && nix build --no-link -L .#checks.x86_64-linux.workflows && nix shell --inputs-from . nixpkgs#yq-go -c yq -e '.jobs.images.needs == "create-release" and ([.jobs.images.steps[].uses] | map(select(. != null) | test("cachix-action")) | any | not) and .jobs.manifest.needs == "images" and (.jobs.publish.needs | sort | join(",")) == "binaries,manifest"' .github/workflows/ci.yml && git diff --cached --quiet` · depends: M6.T1

## M7: Docs and close-out: README Install + CI image sections, spec S8 as built + D49-D56 + M6 row + S14.1 + O3 human steps, gate.sh header (~190 lines). (branch: milestone; depends: M6)
Acceptance: README documents flake input (neither inputs.nixpkgs.follows nor inputs.crane.follows, and why: either loses the Cachix hit; inputs.pin-toolkit.inputs.home-manager.follows is safe and only dedups the consumer's lock), nix profile install, cachix use pin-toolkit, HM example (settings without secrets), plugin override (uncached), overlay (needs rustc >= 1.88), release tarballs + attestation verify, macOS via Nix/Cachix or cargo install --locked, the GitLab CI image job with digest pin, scripts/nix-check.sh; readme anchor check green; spec rows D49-D56 present, plugins.<name>.settings removed from the S8.2 sketch, M6 row marked done with branches; O3 human steps incl. immutable releases + v* tag ruleset
- [ ] M7.T1: README.md: add `## Install` (flake input, nix profile, Cachix, Home Manager example from design §6 (plus: do not follow nixpkgs or crane; home-manager.follows is safe), plugin override (uncached trimmed build), overlay (state next to it: needs rustc >= 1.88, let-chains), release tarballs + `gh attestation verify`, macOS) and `## CI image` (spec §8.4 example with ghcr.io/grburst/pin-toolkit:<ver>@sha256:<digest>, DOCKER_AUTH_CONFIG, CI_JOB_TOKEN/GITLAB_TOKEN); Tests section names scripts/nix-check.sh; link the new sections from the overview.
  files: README.md · forbidden: secret values in examples; claiming a release or image exists (none until the owner tags); inputs.nixpkgs.follows or inputs.crane.follows in any example (they lose the Cachix hit) · check: `bash scripts/check-readme-anchors.sh && grep -q '^## Install' README.md && grep -q 'cachix use pin-toolkit' README.md && grep -q 'ghcr.io/grburst/pin-toolkit' README.md && grep -q 'nix-check.sh' README.md && grep -q '1\.88' README.md && grep -q 'home-manager.follows' README.md` · depends: —
- [ ] M7.T2: Spec docs/superpowers/specs/2026-11-09-pin-toolkit-design.md: "As built (M6)" bullets in S8.1-S8.4 (design §2-§6), drop plugins.<name>.settings from the S8.2 sketch, D49-D56 rows (design §9), M6 row done with branch names, S14.1 DF1/M6 gate item resolved (D51), O3 human steps (Cachix cache + CACHIX_AUTH_TOKEN, GHCR package public after first push, GitHub immutable releases + a v* tag ruleset, first tag, owner drops `- ci builds + cachix` from their local untracked TODO.md); scripts/gate.sh header comment: CI runs these legs as flake checks (scripts/nix-check.sh).
  files: docs/superpowers/specs/2026-11-09-pin-toolkit-design.md, scripts/gate.sh · forbidden: changing gate.sh behaviour (comment only); rewriting unrelated spec sections; touching TODO.md (owner-local, untracked, outside git) · check: `S=docs/superpowers/specs/2026-11-09-pin-toolkit-design.md && grep -q '^| D56 ' $S && ! grep -q 'plugins.<name>.settings = { }' $S && bash -n scripts/gate.sh && bash scripts/gate.sh` · depends: M7.T1

## GitHub-only verification (after the owner merges; not router tasks)
See design §8. On the first PR/main run: `gh run view <id> --json jobs --jq '.jobs[]|[.name,.conclusion]'` shows nix (ubuntu-24.04, ubuntu-24.04-arm, macos-15) and advisories `success` (aarch64-linux + darwin builds, darwin nextest). Cachix: notice "Cachix cache pin-toolkit not found" until the owner creates it; then `nix path-info --store https://pin-toolkit.cachix.org <outPath>` on main. First tag (owner, v0.1.0): `gh release view v0.1.0 --json isDraft,assets` (not draft, 4 assets), `sha256sum -c`, `gh attestation verify <tar.gz> -R GRBurst/pin-toolkit`, `docker buildx imagetools inspect ghcr.io/grburst/pin-toolkit:0.1.0` (linux/amd64 + linux/arm64), `docker run --rm ghcr.io/grburst/pin-toolkit:0.1.0 pin-toolkit --version`, `gh attestation verify oci://ghcr.io/grburst/pin-toolkit:0.1.0 -R GRBurst/pin-toolkit`; set the GHCR package public. A failure leaves a draft release: fix on main, re-run failed jobs or delete draft + tag and re-tag (design §5.5; only drafts are re-runnable). Darwin/arm failures become follow-up tasks on a new branch. Owner: enable GitHub immutable releases and a `v*` tag ruleset; drop `- ci builds + cachix` from your local, untracked TODO.md (no task edits it). A release that is already published is never modified by CI: `create-release` fails on it, and tags with `-` (e.g. v0.2.0-rc.1) are pre-releases that do not move `:latest`.

<details><summary>skeleton</summary>

```json
{
  "invariants": [
    "Nix sees git-tracked files only: every nix check stages only its own task's source paths first (`git add -A -- <paths>`, never a bare `git add -A`: it would sweep the untracked plan file, checkbox ticks and stray out-links into the task commit) and ends with `git diff --cached --quiet`; checks run on the committed task, so a file left out of the commit is red. The orchestrator commits this plan file before execution. Never evaluate the repo as `path:.` (copies .tmp/ and target/ into the store)",
    "Sandbox: every nix command runs with `export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache` (the main checkout's cache, shared by worktrees); only x86_64-linux builds locally; aarch64-linux and aarch64-darwin are evaluated locally (`nix flake check --no-build --all-systems`) and built only in GitHub CI",
    "bash scripts/gate.sh passes on every branch with the developer's usual toolchain (tests, fmt, clippy -D warnings, no-default-features, D38, rustdoc); from M2.T3 on it also passes under `nix develop` (the flake toolchain, rustc 1.99.1)",
    "No product behaviour change: the only Rust edits are the #[allow(clippy::result_large_err)] on the test helper pin_files (M2.T3) and `publish = false` in the 3 crate manifests (M2.T5); existing tests and snapshots unchanged (INSTA_UPDATE=no)",
    "allPlugins comes from crates/pin-toolkit/Cargo.toml [features] minus default (nix/plugins.nix) and nowhere else; checks derive expectations from passthru.plugins/version, not literals",
    "Every `uses:` is pinned by a 40-hex commit SHA with a `# vX` comment (design §5.1); workflow-level permissions are {} and every job declares its own; secrets are never echoed; Cachix pushes, and the Cachix auth token, only on push events (main, v* tags); the release jobs (binaries, images) use no Cachix; runner labels are pinned (ubuntu-24.04, ubuntu-24.04-arm, macos-15), no *-latest",
    "The HM module writes only the global config schema (version=1 injected, json, concurrency, [hosts]); env-var names only, never secret values",
    "Every new check is seen RED once with its planted violation (listed in the task) before it is accepted; note \"planted: <what> -> red seen\" in the commit body; revert the plant",
    "Files created by an earlier task (flake.nix, nix/checks.nix, nix/hm-module.nix, .github/workflows/ci.yml, …) are edited, not recreated, by later tasks listing them in create",
    "Each milestone diff stays reviewable (target <= 400 changed lines, flake.lock excluded)"
  ],
  "milestones": [
    {
      "acceptance": [
        "nix build .#pin-toolkit -> `pin-toolkit --version` = \"pin-toolkit <workspace.package.version>\"",
        "layout(v) holds for v in {default, override [], override [p] for each plugin}",
        "an unknown plugin fails evaluation",
        "overlay drvPath == packages.pin-toolkit drvPath",
        "scripts/nix-check.sh green (x86_64 build + all-systems eval) and refuses untracked source files",
        "gate.sh green"
      ],
      "branch": "milestone",
      "dependsOn": [],
      "goal": "Flake skeleton: packages.{default,pin-toolkit} via nix/package.nix (callPackage, plugins ? allPlugins), layout property checks per plugin set, overlays.default, devShells.default, scripts/nix-check.sh (~230 lines incl. flake.lock 65).",
      "id": "M1",
      "tasks": [
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix flake.lock nix .gitignore && nix flake metadata --json | jq -e '.locks.nodes.nixpkgs.original.ref == \"nixpkgs-unstable\" and .locks.nodes.nixpkgs.locked.rev == \"5d2b81a7a3be3e2f832879028e4d60abbf58e9a0\" and .locks.nodes.crane.locked.rev == \"692f7e9ef2ece8125b466f66f2af532b3edaed0d\" and .locks.nodes[\"home-manager\"].locked.rev == \"a1293da7dd8e497e08d26173f97e2976b36a31ea\"' > /dev/null && out=$(nix build --no-link --print-out-paths .#pin-toolkit) && v=$(nix eval --raw .#pin-toolkit.version) && [ \"$v\" = \"$(sed -n 's/^version = \"\\(.*\\)\"$/\\1/p' Cargo.toml)\" ] && \"$out/bin/pin-toolkit\" --version | grep -qx \"pin-toolkit $v\" && git diff --cached --quiet",
          "create": [
            "flake.nix",
            "flake.lock",
            "nix/plugins.nix",
            "nix/common.nix",
            "nix/package.nix"
          ],
          "dependsOn": [],
          "files": [
            "flake.nix",
            "flake.lock",
            "nix/plugins.nix",
            "nix/common.nix",
            "nix/package.nix",
            ".gitignore"
          ],
          "forbidden": [
            "rust-overlay, flake-utils or a rust-toolchain file",
            "crane cleanCargoSource (drops fixtures/snapshots)",
            "listing plugin names anywhere but Cargo.toml",
            "outputs beyond packages in this task",
            "editing flake.lock by hand or running nix flake update/lock (copy the spike file)"
          ],
          "goal": "Create flake.nix (inputs nixpkgs nixpkgs-unstable, crane v0.24.0, home-manager follows nixpkgs; systems x86_64-linux aarch64-linux aarch64-darwin via lib.genAttrs; packages.<sys>.{default,pin-toolkit} only), flake.lock copied verbatim from the spike file <main checkout>/.tmp/m6-spike/ws/flake.lock (not retyped from markdown; same content as design Appendix A: nixpkgs 5d2b81a, crane 692f7e9, home-manager a1293da; the check asserts the three revs), nix/plugins.nix, nix/common.nix (fileset Cargo.toml Cargo.lock crates pin-ci-images.sh; deps = buildDepsOnly --workspace --all-features), nix/package.nix (doCheck=false; symlinks pin-<p> and pin-ci iff gitlab-ci; passthru plugins/allPlugins; meta.mainProgram; assertMsg on unknown plugins); .gitignore `/result*` and `/dist`.",
          "id": "M1.T1"
        },
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix nix && nix build --no-link -L .#checks.x86_64-linux.pin-toolkit .#checks.x86_64-linux.plugins-none .#checks.x86_64-linux.plugins-gitlab-ci && git diff --cached --quiet",
          "create": [
            "flake.nix",
            "nix/package.nix",
            "nix/layout.nix",
            "nix/checks.nix"
          ],
          "dependsOn": [
            "M1.T1"
          ],
          "files": [
            "flake.nix",
            "nix/package.nix",
            "nix/layout.nix",
            "nix/checks.nix"
          ],
          "forbidden": [
            "hard-coding the expected bin list or plugin names (derive from passthru.plugins)",
            "running tests in package builds"
          ],
          "goal": "Create nix/layout.nix (property over a build v: ls bin = {pin-toolkit} ∪ {pin-p | p ∈ v.plugins} ∪ {pin-ci | gitlab-ci ∈ v.plugins}; aliases are symlinks to pin-toolkit; enabled lines of `pin-toolkit plugins` = v.plugins; --version) and nix/checks.nix with pin-toolkit = pkg, plugins-none and plugins-<p> for every p (layout of each override); wire checks in flake.nix. Planted: delete the pin-ci symlink line in package.nix -> plugins-gitlab-ci red \"bin: [pin-gitlab-ci pin-toolkit ] expected [pin-ci pin-gitlab-ci pin-toolkit]\".",
          "id": "M1.T2"
        },
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix nix scripts/nix-check.sh && bash scripts/nix-check.sh && nix develop -c cargo --version | grep -q '^cargo ' && git diff --cached --quiet",
          "create": [
            "flake.nix",
            "nix/checks.nix",
            "scripts/nix-check.sh"
          ],
          "dependsOn": [
            "M1.T2"
          ],
          "files": [
            "flake.nix",
            "nix/checks.nix",
            "scripts/nix-check.sh"
          ],
          "forbidden": [
            "an overlay that pins the flake's own nixpkgs (it must use final)",
            "building anything in nix-check.sh beyond flake check"
          ],
          "goal": "Add overlays.default (final.callPackage ./nix/package.nix { craneLib = crane.mkLib final; plugins = import ./nix/plugins.nix final.lib; }: explicit, so a consumer overlay's own `plugins` attribute cannot leak in; drvPath unchanged), eval assertions on checks.pin-toolkit ((pkgs.extend overlay).pin-toolkit.drvPath == pkg.drvPath; tryEval of override { plugins = [ \"no-such-plugin\" ]; } fails), devShells.default (crane devShell, checks as inputsFrom, + cargo-nextest cargo-deny actionlint), scripts/nix-check.sh (design Appendix A: untracked-source guard, nix flake check -L, nix flake check --no-build --all-systems). Planted: tryEval with \"gitlab-ci\" -> red \"unknown plugin accepted\"; untracked nix/x.nix -> nix-check.sh exit 1.",
          "id": "M1.T3"
        }
      ]
    },
    {
      "acceptance": [
        "checks fmt doc nextest nextest-no-default clippy feature-matrix d38 deny green on x86_64-linux; clippy, nextest, nextest-no-default and doc install no cargo artifacts",
        "nextest runs the whole workspace with 0 skips needed (402 tests at plan time) and fails if no test runs",
        "each check was seen red with its planted violation",
        "gate.sh green with the developer toolchain and under nix develop"
      ],
      "branch": "milestone",
      "dependsOn": [
        "M1"
      ],
      "goal": "The cargo gate as flake checks: fmt, doc, nextest (all features) + nextest-no-default, clippy, feature-matrix, d38, deny (offline bans/licenses/sources); clippy fix for the newer toolchain; publish = false (~140 lines).",
      "id": "M2",
      "tasks": [
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix && nix build --no-link -L .#checks.x86_64-linux.fmt .#checks.x86_64-linux.doc && git diff --cached --quiet",
          "create": [
            "nix/checks.nix"
          ],
          "dependsOn": [],
          "files": [
            "nix/checks.nix"
          ],
          "forbidden": [
            "changing crates/ beyond the reverted plants"
          ],
          "goal": "Add checks fmt (craneLib.cargoFmt on common.src) and doc (cargoDoc, RUSTDOCFLAGS=-D warnings, doInstallCargoArtifacts = false: leaf-check outputs must not bloat Cachix). Planted: misformat one line in crates/pin-core/src/lib.rs -> fmt red; a broken intra-doc link -> doc red.",
          "id": "M2.T1"
        },
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix && nix build --no-link -L .#checks.x86_64-linux.nextest .#checks.x86_64-linux.nextest-no-default && git diff --cached --quiet",
          "create": [
            "nix/checks.nix",
            "nix/common.nix"
          ],
          "dependsOn": [
            "M2.T1"
          ],
          "files": [
            "nix/checks.nix",
            "nix/common.nix"
          ],
          "forbidden": [
            "skipping or filtering tests (if one fails in the sandbox, stop and report: main allows only a short named skip list)",
            "network access",
            "writing snapshots"
          ],
          "goal": "Add checks nextest (--workspace --all-features via common args) and nextest-no-default (-p pin-toolkit --no-default-features), both cargoNextestExtraArgs \"--no-tests=fail\", env INSTA_UPDATE=no + CI=true, __darwinAllowLocalNetworking = true, doInstallCargoArtifacts = false (89 + 13 MB of useless artifacts otherwise). Planted: drop ../pin-ci-images.sh from the fileset -> oracle_repo_root_has_script red; a #[cfg(not(feature = \"gitlab-ci\"))] panicking test in crates/pin-toolkit/tests -> nextest-no-default red.",
          "id": "M2.T2"
        },
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix crates/pin-gitlab-ci/tests/common/driver.rs && nix build --no-link -L .#checks.x86_64-linux.clippy && nix develop -c env CARGO_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.cargo-home bash scripts/gate.sh && git diff --cached --quiet",
          "create": [
            "nix/checks.nix"
          ],
          "dependsOn": [
            "M2.T2"
          ],
          "files": [
            "nix/checks.nix",
            "crates/pin-gitlab-ci/tests/common/driver.rs"
          ],
          "forbidden": [
            "changing PinFilesError, pin_files' signature or any non-test code",
            "workspace-wide lint allows"
          ],
          "goal": "RED first: add check clippy (cargoClippy, --all-targets -- -D warnings over --workspace --all-features, doInstallCargoArtifacts = false) and see it fail with clippy::result_large_err at crates/pin-gitlab-ci/tests/common/driver.rs:106. GREEN: `#[allow(clippy::result_large_err)] // test helper: the error carries the earlier rows (review L2)` directly above `pub async fn pin_files(`.",
          "id": "M2.T3"
        },
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix && nix build --no-link -L .#checks.x86_64-linux.feature-matrix .#checks.x86_64-linux.d38 && git diff --cached --quiet",
          "create": [
            "nix/checks.nix"
          ],
          "dependsOn": [
            "M2.T3"
          ],
          "files": [
            "nix/checks.nix"
          ],
          "forbidden": [
            "diverging from the feature sets in scripts/gate.sh"
          ],
          "goal": "Add checks feature-matrix (mkCargoDerivation, nativeBuildInputs [pkgs.clippy]; clippy -p pin-core --no-default-features for \"\" oci gitlab yaml oci,gitlab,test-support and -p pin-toolkit --no-default-features, --all-targets -D warnings) and d38 (cargo check -p pin-gitlab-ci -p pin-toolkit; cargo tree -e normal,build -f '{p} {f}' must not match pin-core.*test-support). Planted: `#[cfg(all(feature = \"oci\", not(feature = \"gitlab\")))] fn planted() {}` in pin-core -> feature-matrix red; features = [\"test-support\"] on pin-toolkit's normal pin-core dependency -> d38 red.",
          "id": "M2.T4"
        },
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix deny.toml crates/pin-core/Cargo.toml crates/pin-gitlab-ci/Cargo.toml crates/pin-toolkit/Cargo.toml && nix build --no-link -L .#checks.x86_64-linux.deny && bash scripts/gate.sh && git diff --cached --quiet",
          "create": [
            "nix/checks.nix",
            "deny.toml"
          ],
          "dependsOn": [
            "M2.T4"
          ],
          "files": [
            "nix/checks.nix",
            "deny.toml",
            "crates/pin-core/Cargo.toml",
            "crates/pin-gitlab-ci/Cargo.toml",
            "crates/pin-toolkit/Cargo.toml"
          ],
          "forbidden": [
            "advisories in a flake check (network)",
            "allow-listing a license not reported by cargo-deny",
            "touching dependencies or Cargo.lock"
          ],
          "goal": "RED first: add check deny (craneLib.cargoDeny, src = common.src + deny.toml) with deny.toml from design Appendix A (explicit [advisories] table: unmaintained = \"workspace\", yanked = \"deny\"; the offline check ignores it, CI's advisories job uses it); see error[wildcard] / bug[unresolved-workspace-dependency]. GREEN: `publish = false` after `license.workspace = true` in crates/{pin-core,pin-gitlab-ci,pin-toolkit}/Cargo.toml. Planted: drop \"ISC\" from licenses.allow -> red.",
          "id": "M2.T5"
        }
      ]
    },
    {
      "acceptance": [
        "static binary is static(-pie) linked and satisfies layout",
        "image has no Entrypoint/Cmd, SSL_CERT_FILE, arch matching the system, sh/grep/pin-toolkit/CA bundle inside, < 20 MB",
        "dist holds exactly pin-toolkit-v<ver>-<triple>.tar.gz + .sha256 with LICENSE, README.md and every bin entry",
        "outputs absent on darwin (packages.aarch64-darwin attrNames = [default, pin-toolkit])",
        "nix-check.sh green"
      ],
      "branch": "milestone",
      "dependsOn": [
        "M2"
      ],
      "goal": "Linux artefacts: packages.pin-toolkit-static (crane on pkgsStatic), container (buildLayeredImage), dist (release tar.gz + .sha256) with checks static, container, dist (~110 lines).",
      "id": "M3",
      "tasks": [
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix nix && nix build --no-link -L .#checks.x86_64-linux.static && git diff --cached --quiet",
          "create": [
            "flake.nix",
            "nix/checks.nix"
          ],
          "dependsOn": [],
          "files": [
            "flake.nix",
            "nix/checks.nix"
          ],
          "forbidden": [
            "rust-overlay or pkgsCross unless pkgsStatic fails on a system (then stop and report)",
            "exposing static/container/dist on darwin"
          ],
          "goal": "Add packages.<linux>.pin-toolkit-static = pkgs.pkgsStatic.callPackage ./nix/package.nix { craneLib = crane.mkLib pkgs.pkgsStatic; } and Linux-only check static (`file -L` matches 'static-pie linked\\|statically linked'; layout of the static build). Planted: build it with pkgs instead of pkgsStatic -> static red.",
          "id": "M3.T1"
        },
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix nix && nix build --no-link -L .#checks.x86_64-linux.container && git diff --cached --quiet",
          "create": [
            "flake.nix",
            "nix/checks.nix",
            "nix/container.nix"
          ],
          "dependsOn": [
            "M3.T1"
          ],
          "files": [
            "flake.nix",
            "nix/checks.nix",
            "nix/container.nix"
          ],
          "forbidden": [
            "Entrypoint or Cmd",
            "a dynamically linked binary or busybox",
            "running docker/podman in checks"
          ],
          "goal": "Create nix/container.nix (design Appendix A: static pin-toolkit, pkgsStatic.busybox, dockerTools.caCertificates, /tmp 1777, Env PATH=/bin + SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt, OCI labels, no Entrypoint/Cmd), packages.container, check container (manifest/config via jq incl. architecture amd64|arm64 per system; in-image symlink resolution of /bin/sh /bin/grep /bin/pin-toolkit and the CA bundle; busybox `sh -c`; --version; size < 20 MB). Planted: Entrypoint = [ \"/bin/pin-toolkit\" ] -> container red.",
          "id": "M3.T2"
        },
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix nix && nix build --no-link -L .#checks.x86_64-linux.dist && bash scripts/nix-check.sh && [ \"$(nix eval --json .#packages.aarch64-darwin --apply builtins.attrNames)\" = '[\"default\",\"pin-toolkit\"]' ] && git diff --cached --quiet",
          "create": [
            "flake.nix",
            "nix/checks.nix",
            "nix/dist.nix"
          ],
          "dependsOn": [
            "M3.T2"
          ],
          "files": [
            "flake.nix",
            "nix/checks.nix",
            "nix/dist.nix"
          ],
          "forbidden": [
            "hard-coding version or triple (derive from pin-toolkit.version and pkgsStatic.stdenv.hostPlatform.config)"
          ],
          "goal": "Create nix/dist.nix (runCommand: pin-toolkit-v<version>-<pkgsStatic hostPlatform.config> dir with `cp -P` of static bin/*, LICENSE, README.md; deterministic tar --sort=name --mtime=@1 --owner=0 --group=0 --numeric-owner; .sha256 via sha256sum), packages.dist, check dist (exact file set, sha256sum -c, tar members = LICENSE README.md + ls static/bin). Planted: omit README.md from the tarball -> dist red.",
          "id": "M3.T3"
        }
      ]
    },
    {
      "acceptance": [
        "all plugins enabled -> finalPackage == flake default (same store path)",
        "a disabled plugin -> override with the enabled set",
        "enabled plugin missing from package.passthru.plugins and repo-only settings keys fail evaluation with their specific assertion messages (and the default config has no failed assertion)",
        "the written config.toml has version = 1 and is accepted by `pin-toolkit config show`",
        "option descriptions state the uncached trimmed build and the no-secrets rule",
        "module evaluates on all 3 systems",
        "homeManagerModules.default is a named module (_file); package default on an unlisted system throws a clear message"
      ],
      "branch": "milestone",
      "dependsOn": [
        "M3"
      ],
      "goal": "Home Manager module programs.pin-toolkit (O1, S8.2 as built M5e'): enable, package, plugins.<p>.enable, settings (global config), read-only finalPackage, xdg config file validated by the binary, assertions; outputs homeManagerModules.default + homeModules.default; check hm-module on all systems (~130 lines).",
      "id": "M4",
      "tasks": [
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- flake.nix nix && nix build --no-link -L .#checks.x86_64-linux.hm-module && git diff --cached --quiet",
          "create": [
            "flake.nix",
            "nix/checks.nix",
            "nix/hm-module.nix"
          ],
          "dependsOn": [],
          "files": [
            "flake.nix",
            "nix/checks.nix",
            "nix/hm-module.nix"
          ],
          "forbidden": [
            "plugins.<name>.settings (dropped, design §1 #4)",
            "secret values in options or examples",
            "reading the flake's nixpkgs for anything but the default package"
          ],
          "goal": "Create nix/hm-module.nix (self: module) with options enable, package (default self.packages.${system}.default or (throw \"pin-toolkit: no package for ${system}; set programs.pin-toolkit.package\")), plugins = genAttrs allPlugins { enable (default true) }, settings (pkgs.formats.toml {}), finalPackage (readOnly; package if naturalSort enabled == naturalSort package.passthru.plugins else package.override { plugins = enabled; }); config: home.packages = [ finalPackage ], xdg.configFile.\"pin-toolkit/config.toml\".source = toml.generate ({ version = 1; } // settings). Outputs homeManagerModules.default = { _file = ./nix/hm-module.nix; imports = [ (import ./nix/hm-module.nix self) ]; } (named for error messages and dedup); homeModules.default alias. Check hm-module via home-manager.lib.homeManagerConfiguration (stateVersion 25.05): full -> finalPackage == pkg; gitlab-ci disabled -> finalPackage.passthru.plugins == [ ]; generated file contains `version = 1`. Planted: finalPackage always the override -> red \"all plugins enabled: finalPackage must be the flake default\".",
          "id": "M4.T1"
        },
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix && nix build --no-link -L .#checks.x86_64-linux.hm-module && git diff --cached --quiet",
          "create": [
            "nix/hm-module.nix",
            "nix/checks.nix"
          ],
          "dependsOn": [
            "M4.T1"
          ],
          "files": [
            "nix/hm-module.nix",
            "nix/checks.nix"
          ],
          "forbidden": [
            "silently dropping or rewriting user settings",
            "a negative check that passes on any evaluation error (match the specific message)"
          ],
          "goal": "hm-module.nix assertions: enabled ⊆ package.passthru.plugins (\"programs.pin-toolkit: plugin(s) … enabled but not in programs.pin-toolkit.package …\"); settings ∩ {version, plugins, update, release_age} = ∅ (message points to .pin-toolkit.toml). Check negatives by evaluating the module with lib.evalModules and stub options (assertions, home.packages, xdg.configFile), reading the failed assertion messages: lib.hasInfix \"enabled but not in programs.pin-toolkit.package\" and \"programs.pin-toolkit.settings: update not allowed\" (not a bare tryEval: it is red for any error), plus the positive control failedAssertions { } == [ ]. Planted: remove the repo-only assertion -> red \"repo-only key in settings: assertion message\".",
          "id": "M4.T2"
        },
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix && nix build --no-link -L .#checks.x86_64-linux.hm-module && bash scripts/nix-check.sh && git diff --cached --quiet",
          "create": [
            "nix/hm-module.nix",
            "nix/checks.nix"
          ],
          "dependsOn": [
            "M4.T2"
          ],
          "files": [
            "nix/hm-module.nix",
            "nix/checks.nix"
          ],
          "forbidden": [
            "validating with a second parser in Nix (the binary is the schema)"
          ],
          "goal": "Validate the generated file at build time: the xdg source becomes a runCommand that copies it to xdg/pin-toolkit/config.toml and runs `XDG_CONFIG_HOME=$PWD/xdg ${lib.getExe cfg.finalPackage} config show > /dev/null` before copying to $out (not --no-config: it skips the global file). Check: copy the file, `config show` prints `concurrency = 4` and `hosts.gitlab.example.com.token-env = GITLAB_EXAMPLE_TOKEN`. Planted: drop `{ version = 1; } //` -> red \"missing `version = 1`\".",
          "id": "M4.T3"
        }
      ]
    },
    {
      "acceptance": [
        "workflows check green and seen red for an unpinned uses, for a job without permissions, for a missing top-level permissions and for a job with permissions: write-all",
        "ci.yml nix job = checkout (persist-credentials false), install-nix-action, Cachix probe + guarded cachix-action (skipPush unless push; authToken only on push events), nix flake check -L, nix build .#pin-toolkit",
        "advisories runs cargo deny check advisories via nix shell --inputs-from .",
        "nix-check.sh green"
      ],
      "branch": "milestone",
      "dependsOn": [
        "M4"
      ],
      "goal": "CI for every push/PR: .github/workflows/ci.yml jobs nix (3-OS matrix, Cachix probe-guarded, push only on push events) and advisories; check workflows (actionlint + shellcheck, SHA pins, least-privilege permissions incl. top-level) (~110 lines).",
      "id": "M5",
      "tasks": [
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- nix .github/workflows/ci.yml && nix build --no-link -L .#checks.x86_64-linux.workflows && bash scripts/nix-check.sh && git diff --cached --quiet",
          "create": [
            "nix/checks.nix",
            ".github/workflows/ci.yml"
          ],
          "dependsOn": [],
          "files": [
            "nix/checks.nix",
            ".github/workflows/ci.yml"
          ],
          "forbidden": [
            "tag-only refs (@v4) or unpinned actions",
            "secrets in run: lines",
            "cachix-action without the probe guard",
            "pull_request_target",
            "*-latest runner labels; the Cachix token on pull_request runs"
          ],
          "goal": "Add check workflows (runCommand over lib.fileset .github/workflows; actionlint -no-color with shellcheck on PATH; grep that every uses: matches `uses: [^@ ]+@[0-9a-f]{40} # v[0-9]`; yq-go `(.permissions | tag) == \"!!map\" and (.permissions | length) == 0 and ([.jobs[] | .permissions | tag == \"!!map\"] | all)` (red when the top-level key is absent or a job uses a string like write-all)) and create .github/workflows/ci.yml with triggers, permissions {}, concurrency, env IMAGE, jobs nix and advisories exactly as design Appendix A. Cachix authToken is `${{ github.event_name == 'push' && secrets.CACHIX_AUTH_TOKEN || '' }}`; runner labels pinned ubuntu-24.04 / ubuntu-24.04-arm / macos-15. Planted: actions/checkout@v7 -> red; delete the advisories job's permissions -> red; delete the top-level `permissions: {}` -> red; `permissions: write-all` in a job -> red.",
          "id": "M5.T1"
        }
      ]
    },
    {
      "acceptance": [
        "workflows check green",
        "jobs graph tag-version (tag only), create-release<-[nix, tag-version], binaries<-create-release, images<-create-release, manifest<-images, publish<-[binaries, manifest]",
        "every release job gated by the tag (directly or via needs); every new job has timeout-minutes",
        "permissions per design §5.3 (no artifact-metadata)",
        "binaries and images use no Cachix (attested bytes: source + cache.nixos.org only)",
        "re-runs of a draft are idempotent (view-or-keep-draft, --clobber); an already published release makes create-release fail; tags with \"-\" get --prerelease",
        ":latest only for versions without \"-\""
      ],
      "branch": "milestone",
      "dependsOn": [
        "M5"
      ],
      "goal": "Release path in ci.yml on v* tags: tag-version (tag == workspace version), create-release (draft once; fails on a published release; --prerelease for tags with `-`), binaries (nix build .#dist, actions/attest, gh release upload --clobber; no Cachix), images (skopeo push <ver>-<arch>; no Cachix), manifest (imagetools :<ver> + :latest, attest), publish (draft=false) (~130 lines).",
      "id": "M6",
      "tasks": [
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- .github/workflows/ci.yml && nix build --no-link -L .#checks.x86_64-linux.workflows && nix shell --inputs-from . nixpkgs#yq-go -c yq -e '.jobs.binaries.needs == \"create-release\" and (.jobs.create-release.needs | sort | join(\",\")) == \"nix,tag-version\" and (.jobs.create-release.if | test(\"refs/tags/v\")) and (.jobs.tag-version.if | test(\"refs/tags/v\")) and ([.jobs.binaries.steps[].uses] | map(select(. != null) | test(\"cachix-action\")) | any | not)' .github/workflows/ci.yml && git diff --cached --quiet",
          "create": [
            ".github/workflows/ci.yml"
          ],
          "dependsOn": [],
          "files": [
            ".github/workflows/ci.yml"
          ],
          "forbidden": [
            "packaging shell in YAML (dist is a Nix output)",
            "`|| true` around release creation",
            "actions/attest-build-provenance (design §1 #6a)",
            "Cachix in binaries or images",
            "an artifact-metadata permission (not needed for a User-owned repo)"
          ],
          "goal": "ci.yml: add tag-version (if startsWith(github.ref, 'refs/tags/v'); contents: read; python3 tomllib check tag == v + workspace.package.version; timeout 10), create-release (same if; needs [nix, tag-version]; timeout 10; if `gh release view \"$TAG\"` succeeds and `--json isDraft` is not true -> ::error:: and exit 1, else keep the draft; otherwise gh release create \"$TAG\" --draft --verify-tag --generate-notes --title \"$TAG\" plus --prerelease when $TAG contains \"-\") and binaries (matrix ubuntu-24.04, ubuntu-24.04-arm; no Cachix; permissions contents/id-token/attestations write, no artifact-metadata; nix build -L .#dist --out-link dist; actions/attest subject-path dist/*.tar.gz; gh release upload \"$TAG\" dist/*.tar.gz dist/*.tar.gz.sha256 --clobber) as design Appendix A. Planted: create-release needs [nix, advisories] -> yq check red; a cachix-action step in binaries -> yq check red.",
          "id": "M6.T1"
        },
        {
          "check": "export XDG_CACHE_HOME=$(git rev-parse --path-format=absolute --git-common-dir)/../.tmp/nixcache && git add -A -- .github/workflows/ci.yml && nix build --no-link -L .#checks.x86_64-linux.workflows && nix shell --inputs-from . nixpkgs#yq-go -c yq -e '.jobs.images.needs == \"create-release\" and ([.jobs.images.steps[].uses] | map(select(. != null) | test(\"cachix-action\")) | any | not) and .jobs.manifest.needs == \"images\" and (.jobs.publish.needs | sort | join(\",\")) == \"binaries,manifest\"' .github/workflows/ci.yml && git diff --cached --quiet",
          "create": [
            ".github/workflows/ci.yml"
          ],
          "dependsOn": [
            "M6.T1"
          ],
          "files": [
            ".github/workflows/ci.yml"
          ],
          "forbidden": [
            ":latest for pre-release versions",
            "tokens on the command line (use --password-stdin)",
            "uppercase image names",
            "Cachix in images",
            "artifact-metadata permission"
          ],
          "goal": "ci.yml: add images (matrix amd64/ubuntu-24.04, arm64/ubuntu-24.04-arm; timeout 60; no Cachix; nix build .#container; skopeo from nix run --inputs-from . nixpkgs#skopeo, in a step whose env sets GH_TOKEN and REGISTRY_AUTH_FILE: ${{ runner.temp }}/containers-auth.json (step env, not job env: the runner context is unavailable there; no job-level GH_TOKEN): login ghcr.io --password-stdin, --insecure-policy copy docker-archive:result docker://$IMAGE:${TAG#v}-$ARCH), manifest (timeout 15, no artifact-metadata; docker login; buildx imagetools create -t :<ver> [+ :latest unless ver contains \"-\"]; digest = imagetools inspect --format '{{json .Manifest}}' | jq -r .digest; actions/attest subject-name/subject-digest/push-to-registry) and publish (timeout 10; gh release edit \"$TAG\" --draft=false).",
          "id": "M6.T2"
        }
      ]
    },
    {
      "acceptance": [
        "README documents flake input (neither inputs.nixpkgs.follows nor inputs.crane.follows, and why: either loses the Cachix hit; inputs.pin-toolkit.inputs.home-manager.follows is safe and only dedups the consumer's lock), nix profile install, cachix use pin-toolkit, HM example (settings without secrets), plugin override (uncached), overlay (needs rustc >= 1.88), release tarballs + attestation verify, macOS via Nix/Cachix or cargo install --locked, the GitLab CI image job with digest pin, scripts/nix-check.sh",
        "readme anchor check green",
        "spec rows D49-D56 present, plugins.<name>.settings removed from the S8.2 sketch, M6 row marked done with branches; O3 human steps incl. immutable releases + v* tag ruleset"
      ],
      "branch": "milestone",
      "dependsOn": [
        "M6"
      ],
      "goal": "Docs and close-out: README Install + CI image sections, spec S8 as built + D49-D56 + M6 row + S14.1 + O3 human steps, gate.sh header (~190 lines).",
      "id": "M7",
      "tasks": [
        {
          "check": "bash scripts/check-readme-anchors.sh && grep -q '^## Install' README.md && grep -q 'cachix use pin-toolkit' README.md && grep -q 'ghcr.io/grburst/pin-toolkit' README.md && grep -q 'nix-check.sh' README.md && grep -q '1\\.88' README.md && grep -q 'home-manager.follows' README.md",
          "create": [],
          "dependsOn": [],
          "files": [
            "README.md"
          ],
          "forbidden": [
            "secret values in examples",
            "claiming a release or image exists (none until the owner tags)",
            "inputs.nixpkgs.follows or inputs.crane.follows in any example (they lose the Cachix hit)"
          ],
          "goal": "README.md: add `## Install` (flake input, nix profile, Cachix, Home Manager example from design §6 (plus: do not follow nixpkgs or crane; home-manager.follows is safe), plugin override (uncached trimmed build), overlay (state next to it: needs rustc >= 1.88, let-chains), release tarballs + `gh attestation verify`, macOS) and `## CI image` (spec §8.4 example with ghcr.io/grburst/pin-toolkit:<ver>@sha256:<digest>, DOCKER_AUTH_CONFIG, CI_JOB_TOKEN/GITLAB_TOKEN); Tests section names scripts/nix-check.sh; link the new sections from the overview.",
          "id": "M7.T1"
        },
        {
          "check": "S=docs/superpowers/specs/2026-11-09-pin-toolkit-design.md && grep -q '^| D56 ' $S && ! grep -q 'plugins.<name>.settings = { }' $S && bash -n scripts/gate.sh && bash scripts/gate.sh",
          "create": [],
          "dependsOn": [
            "M7.T1"
          ],
          "files": [
            "docs/superpowers/specs/2026-11-09-pin-toolkit-design.md",
            "scripts/gate.sh"
          ],
          "forbidden": [
            "changing gate.sh behaviour (comment only)",
            "rewriting unrelated spec sections",
            "touching TODO.md (owner-local, untracked, outside git)"
          ],
          "goal": "Spec docs/superpowers/specs/2026-11-09-pin-toolkit-design.md: \"As built (M6)\" bullets in S8.1-S8.4 (design §2-§6), drop plugins.<name>.settings from the S8.2 sketch, D49-D56 rows (design §9), M6 row done with branch names, S14.1 DF1/M6 gate item resolved (D51), O3 human steps (Cachix cache + CACHIX_AUTH_TOKEN, GHCR package public after first push, GitHub immutable releases + a v* tag ruleset, first tag, owner drops `- ci builds + cachix` from their local untracked TODO.md); scripts/gate.sh header comment: CI runs these legs as flake checks (scripts/nix-check.sh).",
          "id": "M7.T2"
        }
      ]
    }
  ]
}
```

</details>
