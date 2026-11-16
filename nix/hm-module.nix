# Home Manager module (spec §8.2, O1): enabling a plugin is a build
# decision; configuring is a runtime decision (repo .pin-toolkit.toml).
# `self` = the pin-toolkit flake (default package from its own nixpkgs, so
# the store path matches the Cachix-cached one).
self:
{ config, lib, pkgs, ... }:
let
  cfg = config.programs.pin-toolkit;
  toml = pkgs.formats.toml { };
  allPlugins = import ./plugins.nix lib;
  enabled = lib.filter (p: cfg.plugins.${p}.enable) allPlugins;
  available = cfg.package.passthru.plugins or [ ];
  configFile = toml.generate "pin-toolkit-config.toml" ({ version = 1; } // cfg.settings);
in
{
  options.programs.pin-toolkit = {
    enable = lib.mkEnableOption "pin-toolkit, the pinning toolkit for CI files";
    package = lib.mkOption {
      type = lib.types.package;
      default =
        self.packages.${pkgs.stdenv.hostPlatform.system}.default
          or (throw "pin-toolkit: no package for ${pkgs.stdenv.hostPlatform.system}; set programs.pin-toolkit.package");
      defaultText = lib.literalExpression "pin-toolkit.packages.\${system}.default";
      description = "The pin-toolkit package with every plugin (the Cachix-cached store path).";
    };
    plugins = lib.genAttrs allPlugins (p: {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Build the ${p} plugin into pin-toolkit. Disabling any plugin makes
          `finalPackage` a feature-trimmed build that is not in the
          pin-toolkit Cachix cache and compiles locally. To stop a built
          plugin from running in a repository, set `[plugins.${p}] run = false`
          in its `.pin-toolkit.toml` instead.
        '';
      };
    });
    settings = lib.mkOption {
      type = toml.type;
      default = { };
      example = lib.literalExpression ''
        {
          concurrency = 8;
          hosts."gitlab.example.com" = { kind = "gitlab"; token-env = "GITLAB_EXAMPLE_TOKEN"; };
        }
      '';
      description = ''
        The global configuration file `pin-toolkit/config.toml` (`version = 1`
        is added): `json`, `concurrency` and `[hosts]`. Environment-variable
        names only, never secret values (the Nix store is world-readable).
      '';
    };
    finalPackage = lib.mkOption {
      type = lib.types.package;
      readOnly = true;
      description = "The package installed: `package`, or a feature-trimmed override of it.";
    };
  };

  config = lib.mkIf cfg.enable {
    programs.pin-toolkit.finalPackage =
      if lib.naturalSort enabled == lib.naturalSort available then cfg.package
      else cfg.package.override { plugins = enabled; };
    home.packages = [ cfg.finalPackage ];
    xdg.configFile."pin-toolkit/config.toml".source = configFile;
  };
}
