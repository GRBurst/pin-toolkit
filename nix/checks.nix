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
  allPlugins = import ./plugins.nix lib;
  plugins =
    { plugins-none = pkg.override { plugins = [ ]; }; }
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
  # Messages of the failed assertions of the bare module (HM's own evaluation
  # only throws them; stub the options the module sets so they can be read).
  failedAssertions = extra:
    let
      stub = { options = {
        assertions = lib.mkOption { type = lib.types.listOf lib.types.raw; default = [ ]; };
        home.packages = lib.mkOption { type = lib.types.listOf lib.types.package; default = [ ]; };
        xdg.configFile = lib.mkOption { type = lib.types.attrsOf lib.types.raw; default = { }; };
      }; };
      evaluated = lib.evalModules {
        modules = [ stub self.homeManagerModules.default { _module.args.pkgs = pkgs; programs.pin-toolkit.enable = true; } extra ];
      };
    in
    map (a: a.message) (lib.filter (a: !a.assertion) evaluated.config.assertions);
  layout = pkgs.callPackage ./layout.nix { };
in
lib.mapAttrs' (n: v: lib.nameValuePair n (layout v)) plugins
// {
  # the overlay builds the same store path (pkgs = the flake's nixpkgs);
  # an unknown plugin is an evaluation error
  pin-toolkit =
    assert lib.assertMsg ((pkgs.extend self.overlays.default).pin-toolkit.drvPath == pkg.drvPath) "overlays.default: different derivation";
    assert lib.assertMsg (!(builtins.tryEval (pkg.override { plugins = [ "no-such-plugin" ]; }).drvPath).success) "unknown plugin accepted";
    pkg;
  hm-module =
    assert lib.assertMsg (full.programs.pin-toolkit.finalPackage == pkg) "all plugins enabled: finalPackage must be the flake default";
    assert lib.assertMsg (trimmed.programs.pin-toolkit.finalPackage.passthru.plugins == [ ]) "gitlab-ci disabled: trimmed build";
    assert lib.assertMsg (failedAssertions { } == [ ]) "default module config: no failed assertion";
    assert lib.assertMsg (lib.any (lib.hasInfix "enabled but not in programs.pin-toolkit.package") (failedAssertions { programs.pin-toolkit.package = pkg.override { plugins = [ ]; }; })) "enabled plugin not in package: assertion message";
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
    nativeBuildInputs = [ pkgs.actionlint pkgs.shellcheck pkgs.yq-go ];
    src = lib.fileset.toSource { root = ../.; fileset = ../.github/workflows; };
  } ''
    cd $src
    actionlint -no-color .github/workflows/*.yml
    # every `uses:` pinned by a full commit SHA with a `# vX` comment (spec §8.3)
    if grep -nE '^\s*(-\s+)?uses:' .github/workflows/*.yml | grep -vE 'uses: [^@ ]+@[0-9a-f]{40} # v[0-9]'; then
      echo "unpinned uses: above"; exit 1
    fi
    # least privilege: no workflow-wide grant, every job states its own
    for f in .github/workflows/*.yml; do
      yq -e '(.permissions | tag) == "!!map" and (.permissions | length) == 0 and ([.jobs[] | select((.permissions | tag) != "!!map")] | length) == 0' "$f" > /dev/null \
        || { echo "$f: top-level permissions must be {} and every job must declare permissions"; exit 1; }
    done
    touch $out
  '';
  # leaf checks keep no cargo artifacts: their outputs would be pushed to Cachix
  # (nextest 89 MB, no-default 13 MB) and could evict the package the consumers pull
  clippy = craneLib.cargoClippy (ca // {
    cargoClippyExtraArgs = "--all-targets -- -D warnings";
    doInstallCargoArtifacts = false;
  });
  nextest = craneLib.cargoNextest (ca // {
    cargoNextestExtraArgs = "--no-tests=fail"; # a filter or source change that runs nothing is red
    env = testEnv;
    doInstallCargoArtifacts = false;
    __darwinAllowLocalNetworking = true;
  });
  nextest-no-default = craneLib.cargoNextest (ca // {
    pnameSuffix = "-nd";
    cargoExtraArgs = "--locked -p pin-toolkit --no-default-features";
    cargoNextestExtraArgs = "--no-tests=fail";
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
  doc = craneLib.cargoDoc (ca // { env.RUSTDOCFLAGS = "-D warnings"; doInstallCargoArtifacts = false; });
}
// lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux (
  let
    static = self.packages.${system}.pin-toolkit-static;
    image = self.packages.${system}.container;
    dist = self.packages.${system}.dist;
    arch = { x86_64-linux = "amd64"; aarch64-linux = "arm64"; }.${system};
  in
  {
    static = pkgs.runCommand "pin-toolkit-static-check" { nativeBuildInputs = [ pkgs.file ]; } ''
      file -L ${static}/bin/pin-toolkit | tee /dev/stderr | grep -q 'static-pie linked\|statically linked'
      echo ${layout static} > $out
    '';
    dist = pkgs.runCommand "pin-toolkit-dist-check" { } ''
      cd ${dist}
      name=pin-toolkit-v${static.version}-${pkgs.pkgsStatic.stdenv.hostPlatform.config}
      [ "$(ls | tr '\n' ' ')" = "$name.tar.gz $name.tar.gz.sha256 " ]
      sha256sum -c $name.tar.gz.sha256
      want=$( (echo LICENSE; echo README.md; ls ${static}/bin) | sed "s|^|$name/|" | sort)
      got=$(tar -tzf $name.tar.gz | grep -v '/$' | sort)
      [ "$got" = "$want" ] || { echo "tarball: $got"; echo "expected: $want"; exit 1; }
      touch $out
    '';
    container = pkgs.runCommand "pin-toolkit-container-check" { nativeBuildInputs = [ pkgs.jq pkgs.gnutar pkgs.gzip ]; } ''
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
      for f in /bin/sh /bin/grep /bin/pin-toolkit /etc/ssl/certs/ca-bundle.crt; do
        resolve $f > /dev/null || { echo "image lacks $f"; exit 1; }
      done
      [ -d root/tmp ]
      $(resolve /bin/sh) sh -c 'echo sh ok | grep -q ok' # busybox: the applet comes from argv
      $(resolve /bin/pin-toolkit) --version | grep -qx 'pin-toolkit ${static.version}'
      size=$(stat -c %s ${image}); echo "image ${image}: $size bytes"
      [ "$size" -lt 20000000 ]
      touch $out
    '';
  }
)
