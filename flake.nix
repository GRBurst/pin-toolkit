{
  description = "pin-toolkit: pin CI references to immutable digests";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane/v0.24.0";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, nixpkgs, crane, home-manager }:
    let
      inherit (nixpkgs) lib;
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      eachSystem = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = eachSystem (pkgs:
        let pin-toolkit = pkgs.callPackage ./nix/package.nix { craneLib = crane.mkLib pkgs; };
        in { default = pin-toolkit; inherit pin-toolkit; });
      checks = eachSystem (pkgs: import ./nix/checks.nix { inherit pkgs crane home-manager self; });
      overlays.default = final: _prev: {
        pin-toolkit = final.callPackage ./nix/package.nix {
          craneLib = crane.mkLib final;
          plugins = import ./nix/plugins.nix final.lib; # not a consumer overlay's `plugins` attribute
        };
      };
      devShells = eachSystem (pkgs: {
        default = (crane.mkLib pkgs).devShell {
          checks = self.checks.${pkgs.stdenv.hostPlatform.system};
          packages = with pkgs; [ cargo-nextest cargo-deny actionlint ];
        };
      });
    };
}
