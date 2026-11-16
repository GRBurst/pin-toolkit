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
  fmt = craneLib.cargoFmt { inherit (common) src; inherit (common.args) pname version; };
  doc = craneLib.cargoDoc (ca // { env.RUSTDOCFLAGS = "-D warnings"; doInstallCargoArtifacts = false; });
}
