# Property over a pin-toolkit build v (spec §8.1, §6.6):
#   bin(v) = {pin-toolkit} ∪ {pin-p | p ∈ v.plugins} ∪ {pin-ci | gitlab-ci ∈ v.plugins}
#   enabled(`pin-toolkit plugins`) = v.plugins,  `--version` = "pin-toolkit <version>"
{ lib, runCommand }:
v:
let
  expectedBin = lib.naturalSort ([ "pin-toolkit" ] ++ map (p: "pin-${p}") v.plugins
    ++ lib.optional (lib.elem "gitlab-ci" v.plugins) "pin-ci");
in
runCommand "${v.name}-layout" { } ''
  bin=$(ls ${v}/bin | sort -V | tr '\n' ' ')
  [ "$bin" = "${lib.concatMapStrings (b: b + " ") expectedBin}" ] || { echo "bin: [$bin] expected [${toString expectedBin}]"; exit 1; }
  for a in ${v}/bin/pin-*; do [ "$a" = ${v}/bin/pin-toolkit ] || [ "$(readlink "$a")" = pin-toolkit ] || { echo "$a is not a symlink to pin-toolkit"; exit 1; }; done
  enabled=$(${v}/bin/pin-toolkit plugins | sed -n 's/^\([a-z0-9-]*\): enabled$/\1/p' | sort -V | tr '\n' ' ')
  [ "$enabled" = "${lib.concatMapStrings (p: p + " ") (lib.naturalSort v.plugins)}" ] || { echo "enabled: [$enabled] expected [${toString v.plugins}]"; exit 1; }
  ${v}/bin/pin-toolkit --version | grep -qx 'pin-toolkit ${v.version}'
  touch $out
''
