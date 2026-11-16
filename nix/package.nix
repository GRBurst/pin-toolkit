# pin-toolkit (spec §8.1). `plugins` = the cargo features built in (O1);
# the default (all plugins) is the Cachix-cached store path.
{
  lib,
  craneLib,
  plugins ? import ./plugins.nix lib,
}:
let
  allPlugins = import ./plugins.nix lib;
  unknown = lib.subtractLists allPlugins plugins;
  common = import ./common.nix { inherit lib craneLib; };
in
assert lib.assertMsg (unknown == [ ])
  "pin-toolkit: unknown plugin(s) ${lib.concatStringsSep ", " unknown}; known: ${lib.concatStringsSep ", " allPlugins}";
craneLib.buildPackage (
  common.args
  // {
    inherit (common) cargoArtifacts;
    cargoExtraArgs =
      "--locked -p pin-toolkit --no-default-features"
      + lib.optionalString (plugins != [ ]) " --features ${lib.concatStringsSep "," plugins}";
    doCheck = false; # the tests run once, in checks.nextest
    postInstall =
      lib.concatMapStrings (p: "ln -s pin-toolkit $out/bin/pin-${p}\n") plugins
      + lib.optionalString (lib.elem "gitlab-ci" plugins) "ln -s pin-toolkit $out/bin/pin-ci\n";
    passthru = { inherit plugins allPlugins; };
    meta = {
      description = "Pin CI image, component and include references to immutable digests";
      homepage = "https://github.com/GRBurst/pin-toolkit";
      license = with lib.licenses; [ mit asl20 ];
      mainProgram = "pin-toolkit";
    };
  }
)
