{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

{
  # https://devenv.sh/packages/
  #
  # Tooling for the differential oracle (tests/oracle.rs): the old pipeline
  # (pin-ci-images.sh) needs renovate, python3 and crane; the new tool is
  # built with Rust (see PATH/CARGO_HOME in README, or add a rust language
  # module here if you want it in the devshell).
  packages = [
    pkgs.git
    pkgs.crane
    pkgs.renovate
    pkgs.python3
  ];

  # https://devenv.sh/tests/
  enterTest = ''
    crane --version | grep --color=auto "${pkgs.crane.version}"
    python3 --version
    renovate --version
  '';
}
