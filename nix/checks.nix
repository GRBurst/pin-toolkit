# Flake checks (spec §8.1): the cargo gate's legs (scripts/gate.sh) as
# sandboxed derivations sharing one dependency build, plus the plugin
# builds, the static binary, the container and the Home Manager module.
{ pkgs, crane, home-manager, self }:
let
  inherit (pkgs) lib;
  system = pkgs.stdenv.hostPlatform.system;
  pkg = self.packages.${system}.pin-toolkit;
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
}
