# Shared crane arguments: source, name/version, and the dependency
# artifacts of the whole workspace with all features (one derivation shared
# by the package, every plugin override and every check).
{ lib, craneLib }:
let
  root = ../.;
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../pin-ci-images.sh # oracle.rs `oracle_repo_root_has_script` (not env-gated)
    ];
  };
  args = {
    inherit src version;
    pname = "pin-toolkit";
    strictDeps = true;
    cargoExtraArgs = "--locked --workspace --all-features";
  };
in
{
  inherit src version args;
  cargoArtifacts = craneLib.buildDepsOnly args;
}
