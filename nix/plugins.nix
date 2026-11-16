# allPlugins = the [features] of crates/pin-toolkit minus `default` (spec §8.1):
# Cargo is the single source of truth for which plugins exist.
lib:
builtins.attrNames (
  builtins.removeAttrs (lib.importTOML ../crates/pin-toolkit/Cargo.toml).features [ "default" ]
)
