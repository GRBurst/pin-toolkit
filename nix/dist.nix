# Release assets (spec §8.3 `binaries`): the static binary and its aliases,
# LICENSE and README.md as a reproducible tar.gz plus its .sha256.
{ lib, runCommand, pin-toolkit, target }:
let
  name = "pin-toolkit-v${pin-toolkit.version}-${target}";
in
runCommand name { } ''
  mkdir -p "$name" $out
  cp -P ${pin-toolkit}/bin/* "$name/"
  cp ${../LICENSE} "$name/LICENSE"
  cp ${../README.md} "$name/README.md"
  tar --sort=name --mtime=@1 --owner=0 --group=0 --numeric-owner -czf "$out/$name.tar.gz" "$name"
  (cd $out && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
''
