# Flake checks (spec §8.1): the cargo gate's legs (scripts/gate.sh) as
# sandboxed derivations sharing one dependency build, plus the plugin
# builds, the static binary, the container and the Home Manager module.
{ pkgs, crane, home-manager, self }:
let
  inherit (pkgs) lib;
  system = pkgs.stdenv.hostPlatform.system;
  craneLib = crane.mkLib pkgs;
  common = import ./common.nix { inherit lib craneLib; };
  ca = common.args // { inherit (common) cargoArtifacts; };
  pkg = self.packages.${system}.pin-toolkit;
  testEnv = {
    INSTA_UPDATE = "no"; # a snapshot mismatch fails; nothing is written
    CI = "true";
  };
  # gate.sh runs `cargo test` in the dev profile; the flake checks build in the
  # release profile. Keep debug assertions and overflow checks on for the
  # workspace crates (dependencies stay shared release artifacts), so the
  # debug_assert!s in pin-core run in CI too (review A-M1).
  workspaceCrates = map (d: (lib.importTOML ../crates/${d}/Cargo.toml).package.name)
    (lib.attrNames (lib.filterAttrs (_: t: t == "directory") (builtins.readDir ../crates)));
  devAssertions = lib.concatMapStrings
    (c: " --config profile.release.package.${c}.debug-assertions=true --config profile.release.package.${c}.overflow-checks=true")
    workspaceCrates;
  allPlugins = import ./plugins.nix lib;
  plugins =
    {
      plugins-none = pkg.override { plugins = [ ]; };
      # the all-plugins package (Cachix path) gets its own layout check, not only
      # through the per-plugin ones that coincide with it while there is one plugin
      plugins-all = pkg;
    }
    // lib.genAttrs' allPlugins (p: lib.nameValuePair "plugins-${p}" (pkg.override { plugins = [ p ]; }));
  hmConfig = extra: (home-manager.lib.homeManagerConfiguration {
    inherit pkgs;
    modules = [
      self.homeManagerModules.default
      {
        home = { username = "u"; homeDirectory = if pkgs.stdenv.hostPlatform.isDarwin then "/Users/u" else "/home/u"; stateVersion = "25.05"; };
        programs.pin-toolkit.enable = true;
      }
      extra
    ];
  }).config;
  full = hmConfig { programs.pin-toolkit.settings = { concurrency = 4; hosts."gitlab.example.com" = { kind = "gitlab"; token-env = "GITLAB_EXAMPLE_TOKEN"; }; }; };
  trimmed = hmConfig { programs.pin-toolkit.plugins.gitlab-ci.enable = false; };
  # The bare module on stub options (HM's own evaluation only throws failed
  # assertions; this can read their messages and the generated file's recipe).
  stubConfig = pkgs': extra:
    let
      stub = { options = {
        assertions = lib.mkOption { type = lib.types.listOf lib.types.raw; default = [ ]; };
        home.packages = lib.mkOption { type = lib.types.listOf lib.types.package; default = [ ]; };
        xdg.configFile = lib.mkOption { type = lib.types.attrsOf lib.types.raw; default = { }; };
      }; };
    in
    (lib.evalModules {
      modules = [ stub self.homeManagerModules.default { _module.args.pkgs = pkgs'; programs.pin-toolkit.enable = true; } extra ];
    }).config;
  failedAssertions = extra:
    map (a: a.message) (lib.filter (a: !a.assertion) (stubConfig pkgs extra).assertions);
  # Evaluation of `settings` fails (a type or unknown-option error). builtins.tryEval
  # cannot read the message, so every negative has a positive control that differs
  # from it in the one offending value.
  settingsFail = extra: !(builtins.tryEval (builtins.toJSON (stubConfig pkgs extra).programs.pin-toolkit.settings)).success;
  host = h: { programs.pin-toolkit.settings.hosts."gitlab.example.com" = h; };
  # a build platform that cannot run the host's binaries (cross-built generation)
  noExec = pkgs // { stdenv = pkgs.stdenv // { buildPlatform = pkgs.stdenv.buildPlatform // { canExecute = _: false; }; }; };
  validated = pkgs': lib.hasInfix "config show" (stubConfig pkgs' { }).xdg.configFile."pin-toolkit/config.toml".source.buildCommand;
  layout = pkgs.callPackage ./layout.nix { };
in
lib.mapAttrs' (n: v: lib.nameValuePair n (layout v)) plugins
// {
  # the overlay builds the same store path (pkgs = the flake's nixpkgs);
  # an unknown plugin is an evaluation error
  pin-toolkit =
    assert lib.assertMsg ((pkgs.extend self.overlays.default).pin-toolkit.drvPath == pkg.drvPath) "overlays.default: different derivation";
    assert lib.assertMsg (!(builtins.tryEval (pkg.override { plugins = [ "no-such-plugin" ]; }).drvPath).success) "unknown plugin accepted";
    assert lib.assertMsg ((pkg.override { plugins = [ "gitlab-ci" "gitlab-ci" ]; }).drvPath == pkg.drvPath) "duplicate plugins: must be the same derivation";
    pkg;
  hm-module =
    # derivation == compares outPath: the all-plugins finalPackage is the Cachix-cached store path
    assert lib.assertMsg (full.programs.pin-toolkit.finalPackage.outPath == pkg.outPath) "all plugins enabled: finalPackage must be the flake default";
    assert lib.assertMsg ((hmConfig { imports = [ self.homeModules.default ]; }).programs.pin-toolkit.finalPackage.outPath == pkg.outPath) "both module aliases imported: must deduplicate (key)";
    assert lib.assertMsg ((hmConfig { programs.pin-toolkit.enable = lib.mkForce false; }).programs.pin-toolkit.finalPackage.outPath == pkg.outPath) "disabled: finalPackage still readable";
    assert lib.assertMsg (trimmed.programs.pin-toolkit.finalPackage.passthru.plugins == [ ]) "gitlab-ci disabled: trimmed build";
    assert lib.assertMsg (failedAssertions { } == [ ]) "default module config: no failed assertion";
    assert lib.assertMsg (lib.any (lib.hasInfix "enabled but not in programs.pin-toolkit.package") (failedAssertions { programs.pin-toolkit.package = pkg.override { plugins = [ ]; }; })) "enabled plugin not in package: assertion message";
    assert lib.assertMsg (!settingsFail (host { kind = "gitlab"; token-env = "GITLAB_EXAMPLE_TOKEN"; })) "control: a host with a variable name evaluates";
    assert lib.assertMsg (settingsFail (host { kind = "gitlab"; token = "glpat-SECRET"; })) "hosts.*.token (a secret value) must fail evaluation";
    assert lib.assertMsg (settingsFail (host { kind = "gitlab"; token-env = "glpat-SECRET123"; })) "hosts.*.token-env that is not a variable name must fail evaluation";
    assert lib.assertMsg (settingsFail (host { kind = "ftp"; })) "unknown host kind must fail evaluation";
    assert lib.assertMsg (validated pkgs && !validated noExec) "config validated by the binary only when the build platform can run it";
    assert lib.assertMsg (lib.any (lib.hasInfix "has no `.override`") (failedAssertions { programs.pin-toolkit.package = pkgs.runCommand "w" { passthru.plugins = pkg.passthru.plugins; } "mkdir $out"; programs.pin-toolkit.plugins.gitlab-ci.enable = false; })) "package without .override and a disabled plugin: assertion message";
    assert lib.assertMsg (failedAssertions { programs.pin-toolkit.package = pkgs.runCommand "w" { passthru.plugins = pkg.passthru.plugins; } "mkdir $out"; } == [ ]) "package without .override with every plugin enabled: fine";
    assert lib.assertMsg (lib.any (lib.hasInfix "programs.pin-toolkit.settings: update not allowed") (failedAssertions { programs.pin-toolkit.settings.update.enabled = true; })) "repo-only key in settings: assertion message";
    pkgs.runCommand "pin-toolkit-hm-module-check" { } ''
      f=${full.xdg.configFile."pin-toolkit/config.toml".source}
      grep -qx 'version = 1' $f
      mkdir -p xdg/pin-toolkit && cp $f xdg/pin-toolkit/config.toml
      XDG_CONFIG_HOME=$PWD/xdg ${pkg}/bin/pin-toolkit config show > show
      grep -qx 'concurrency = 4' show
      grep -qx 'hosts.gitlab.example.com.token-env = GITLAB_EXAMPLE_TOKEN' show
      touch $out
    '';
  workflows = pkgs.runCommand "pin-toolkit-workflows-check" {
    nativeBuildInputs = [ pkgs.actionlint pkgs.shellcheck pkgs.yq-go pkgs.zizmor ];
    src = lib.fileset.toSource { root = ../.; fileset = ../.github/workflows; };
  } ''
    cd $src
    # GitHub runs *.yml and *.yaml alike; a new file must not escape the checks
    shopt -s nullglob
    wf=(.github/workflows/*.yml .github/workflows/*.yaml)
    [ ''${#wf[@]} -gt 0 ] || { echo "no workflow files found under .github/workflows"; exit 1; }
    actionlint -no-color "''${wf[@]}"
    # every `uses:` pinned by a full commit SHA with a `# vX` comment (spec §8.3)
    if grep -nE '^\s*(-\s+)?uses:' "''${wf[@]}" | grep -vE 'uses: [^@ ]+@[0-9a-f]{40} # v[0-9]'; then
      echo "unpinned uses: above"; exit 1
    fi
    # least privilege: no workflow-wide grant, every job states its own
    for f in "''${wf[@]}"; do
      yq -e '(.permissions | tag) == "!!map" and (.permissions | length) == 0 and ([.jobs[] | select((.permissions | tag) != "!!map")] | length) == 0' "$f" > /dev/null \
        || { echo "$f: top-level permissions must be {} and every job must declare permissions"; exit 1; }
    done
    # static security audit (template injection, dangerous triggers, flow-style or
    # unpinned uses, artipacked, ...): any finding fails; the audits that need the
    # GitHub API are skipped by --offline (review B-L1)
    zizmor --offline .github/workflows
    touch $out
  '';
  # leaf checks keep no cargo artifacts: their outputs would be pushed to Cachix
  # (nextest 89 MB, no-default 13 MB) and could evict the package the consumers pull
  clippy = craneLib.cargoClippy (ca // {
    cargoClippyExtraArgs = "--all-targets -- -D warnings";
    doInstallCargoArtifacts = false;
  });
  nextest = craneLib.cargoNextest (ca // {
    cargoNextestExtraArgs = "--no-tests=fail${devAssertions}"; # a filter or source change that runs nothing is red
    env = testEnv;
    doInstallCargoArtifacts = false;
    __darwinAllowLocalNetworking = true;
  });
  nextest-no-default = craneLib.cargoNextest (ca // {
    pnameSuffix = "-nd";
    cargoExtraArgs = "--locked -p pin-toolkit --no-default-features";
    cargoNextestExtraArgs = "--no-tests=fail${devAssertions}";
    env = testEnv;
    doInstallCargoArtifacts = false;
    __darwinAllowLocalNetworking = true;
  });
  feature-matrix = craneLib.mkCargoDerivation (ca // {
    pnameSuffix = "-feature-matrix";
    nativeBuildInputs = [ pkgs.clippy ];
    doInstallCargoArtifacts = false;
    buildPhaseCargoCommand = ''
      for f in "" oci gitlab yaml oci,gitlab,test-support; do
        cargo clippy --profile release --locked -p pin-core --no-default-features ''${f:+--features "$f"} --all-targets -- -D warnings
      done
      cargo clippy --profile release --locked -p pin-toolkit --no-default-features --all-targets -- -D warnings
    '';
    installPhaseCommand = "mkdir -p $out";
  });
  d38 = craneLib.mkCargoDerivation (ca // {
    pnameSuffix = "-d38";
    doInstallCargoArtifacts = false;
    buildPhaseCargoCommand = ''
      cargo check --profile release --locked -p pin-gitlab-ci -p pin-toolkit
      for p in pin-gitlab-ci pin-toolkit; do
        t=$(cargo tree --locked -p "$p" -e normal,build -f '{p} {f}')
        if grep -q 'pin-core.*test-support' <<< "$t"; then
          echo "D38: a normal dependency of $p enables pin-core/test-support"; exit 1
        fi
      done
    '';
    installPhaseCommand = "mkdir -p $out";
  });
  deny = craneLib.cargoDeny {
    inherit (common.args) pname version;
    src = lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions [ (lib.fileset.fromSource common.src) ../deny.toml ];
    };
  };
  fmt = craneLib.cargoFmt { inherit (common) src; inherit (common.args) pname version; };
  # mkCargoDerivation, not cargoDoc: cargoDoc hard-codes its install phase and
  # would push the 7.6 MB rendered HTML to Cachix on every main push (review A-L2)
  doc = craneLib.mkCargoDerivation (ca // {
    pnameSuffix = "-doc";
    env.RUSTDOCFLAGS = "-D warnings";
    doInstallCargoArtifacts = false;
    buildPhaseCargoCommand = "cargoWithProfile doc --locked --workspace --all-features --no-deps";
    installPhaseCommand = "mkdir -p $out";
  });
}
// lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux (
  let
    static = self.packages.${system}.pin-toolkit-static;
    image = self.packages.${system}.container;
    dist = self.packages.${system}.dist;
    arch = { x86_64-linux = "amd64"; aarch64-linux = "arm64"; }.${system};
    # written out, not derived: the check pins the published asset name
    triple = { x86_64-linux = "x86_64-unknown-linux-musl"; aarch64-linux = "aarch64-unknown-linux-musl"; }.${system};
  in
  {
    static = pkgs.runCommand "pin-toolkit-static-check" { nativeBuildInputs = [ pkgs.file ]; } ''
      file -L ${static}/bin/pin-toolkit | tee /dev/stderr | grep -q 'static-pie linked\|statically linked'
      echo ${layout static} > $out
    '';
    dist = pkgs.runCommand "pin-toolkit-dist-check" { } ''
      cd ${dist}
      name=pin-toolkit-v${static.version}-${triple}
      [ "$(ls | tr '\n' ' ')" = "$name.tar.gz $name.tar.gz.sha256 " ]
      sha256sum -c $name.tar.gz.sha256
      want=$( (echo LICENSE; echo README.md; ls ${static}/bin) | sed "s|^|$name/|" | sort)
      got=$(tar -tzf $name.tar.gz | grep -v '/$' | sort)
      [ "$got" = "$want" ] || { echo "tarball: $got"; echo "expected: $want"; exit 1; }
      # the shipped binary is the static one checks.static validated (review A-M2)
      x=$(mktemp -d) && tar -xzf $name.tar.gz -C $x
      cmp $x/$name/pin-toolkit ${static}/bin/pin-toolkit
      touch $out
    '';
    container = pkgs.runCommand "pin-toolkit-container-check" { nativeBuildInputs = [ pkgs.jq pkgs.gnutar pkgs.gzip pkgs.file ]; } ''
      mkdir img && tar -xzf ${image} -C img
      cfg=$(jq -r '.[0].Config' img/manifest.json)
      jq -e '(.config.Entrypoint // null) == null and (.config.Cmd // null) == null' img/$cfg
      jq -e '.config.Env | index("SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt") != null' img/$cfg
      jq -e '.architecture == "${arch}" and .os == "linux"' img/$cfg
      mkdir root
      for l in $(jq -r '.[0].Layers[]' img/manifest.json); do tar -xf img/$l -C root; done
      # resolve a path inside the image (absolute symlinks point into its /nix/store)
      resolve() {
        local p=$1 i t
        for i in 1 2 3 4 5 6 7 8; do
          [ -L "root$p" ] || break
          t=$(readlink "root$p")
          case $t in /*) p=$t ;; *) p=$(dirname "$p")/$t ;; esac
        done
        [ -f "root$p" ] && echo "root$p"
      }
      for f in /bin/busybox /bin/sh /bin/grep /bin/pin-toolkit /etc/ssl/certs/ca-bundle.crt; do
        resolve $f > /dev/null || { echo "image lacks $f"; exit 1; }
      done
      # /tmp is sticky and world-writable (the tar header: extracting as a user drops the bit)
      for l in $(jq -r '.[0].Layers[]' img/manifest.json); do tar -tvf img/$l; done \
        | grep -q '^drwxrwxrwt .* \./tmp/$' || { echo "image: /tmp is not drwxrwxrwt"; exit 1; }
      # sh and grep are the image's own busybox, and it is static: a dynamic one
      # runs here (the sandbox has a libc) but not in a bare GitLab job (review A-M2)
      bb=$(resolve /bin/busybox)
      [ "$(resolve /bin/sh)" = "$bb" ] && [ "$(resolve /bin/grep)" = "$bb" ]
      file -L "$bb" | tee /dev/stderr | grep -q 'statically linked\|static-pie linked'
      $bb sh -c 'echo sh ok' | $bb grep -qx 'sh ok' # busybox: the applet comes from argv
      # the shipped binary is the static one checks.static validated
      cmp "$(resolve /bin/pin-toolkit)" ${static}/bin/pin-toolkit
      $(resolve /bin/pin-toolkit) --version | grep -qx 'pin-toolkit ${static.version}'
      size=$(stat -c %s ${image}); echo "image ${image}: $size bytes"
      [ "$size" -lt 10000000 ]
      touch $out
    '';
  }
)
