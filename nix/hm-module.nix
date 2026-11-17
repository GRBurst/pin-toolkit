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
  missing = lib.subtractLists available enabled;
  repoOnly = lib.intersectLists (lib.attrNames cfg.settings) [ "version" "plugins" "update" "release_age" ];
  # An environment variable NAME (spec §7.3), never a value: a pasted value with
  # characters a name cannot contain (e.g. `glpat-…`) is refused at evaluation,
  # before anything reaches the world-readable store. Values that happen to be
  # valid names (e.g. `ghp_…`) cannot be told apart from names and pass.
  envName = lib.types.strMatching "[A-Za-z_][A-Za-z0-9_]*";
  envOption = description: lib.mkOption {
    type = lib.types.nullOr envName;
    default = null;
    inherit description;
  };
  hostType = lib.types.submodule {
    options = {
      kind = lib.mkOption {
        type = lib.types.enum [ "gitlab" "helm-repo" "oci" ];
        description = "What the host is; decides which of the other keys are required.";
      };
      token-env = envOption "gitlab: the NAME of the environment variable holding the PRIVATE-TOKEN.";
      username-env = envOption "helm-repo: the NAME of the environment variable holding the basic-auth user.";
      password-env = envOption "helm-repo: the NAME of the environment variable holding the basic-auth password.";
      ca-file = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "oci: path of a PEM file of extra CA certificates (a runtime path, not copied into the store).";
      };
    };
  };
  # unset (null) host keys and an empty `hosts` are not written
  settingsOut =
    lib.filterAttrs (n: _: n != "hosts") cfg.settings
    // lib.optionalAttrs (cfg.settings.hosts != { }) {
      hosts = lib.mapAttrs (_: lib.filterAttrs (_: v: v != null)) cfg.settings.hosts;
    };
  configFile = toml.generate "pin-toolkit-config.toml" ({ version = 1; } // settingsOut);
  # The binary that reads the file validates it: a bad key fails the build
  # (home-manager switch), not every later run (exit 2). A cross-built
  # generation cannot run the host binary: the file is copied unvalidated.
  checkedConfig =
    if pkgs.stdenv.buildPlatform.canExecute pkgs.stdenv.hostPlatform then
      pkgs.runCommand "pin-toolkit-config.toml" { } ''
        mkdir -p xdg/pin-toolkit
        cp ${configFile} xdg/pin-toolkit/config.toml
        XDG_CONFIG_HOME=$PWD/xdg ${lib.getExe cfg.finalPackage} config show > /dev/null
        cp ${configFile} $out
      ''
    else
      configFile;
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
      type = lib.types.submodule {
        # json, concurrency, ...: any TOML value, validated by the binary at build
        # time; `hosts` is strict (unknown keys and non-name values fail evaluation)
        freeformType = toml.type;
        options.hosts = lib.mkOption {
          type = lib.types.attrsOf hostType;
          default = { };
          description = "Per-host bindings `[hosts.\"<host>\"]` (spec §7.3): environment-variable names only.";
        };
      };
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

  config = lib.mkMerge [
    {
      # defined even when disabled (a lazily-failing read), so other modules can
      # refer to it unconditionally like the other `finalPackage` options
      programs.pin-toolkit.finalPackage =
        if lib.naturalSort enabled == lib.naturalSort available then cfg.package
        else cfg.package.override { plugins = enabled; };
    }
    (lib.mkIf cfg.enable {
      assertions = [
        {
          assertion = lib.naturalSort enabled == lib.naturalSort available || cfg.package ? override;
          message = "programs.pin-toolkit.package has no `.override`, so it cannot be trimmed to the enabled plugins (${lib.concatStringsSep ", " enabled}); keep all plugins enabled or use a package built with pin-toolkit's package.nix";
        }
        {
          assertion = missing == [ ];
          message = "programs.pin-toolkit: plugin(s) ${lib.concatStringsSep ", " missing} enabled but not in programs.pin-toolkit.package (its passthru.plugins: ${lib.concatStringsSep ", " available})";
        }
        {
          assertion = repoOnly == [ ];
          message = "programs.pin-toolkit.settings: ${lib.concatStringsSep ", " repoOnly} not allowed (version is set by the module; plugins, update and release_age belong in a repository's .pin-toolkit.toml)";
        }
      ];
      home.packages = [ cfg.finalPackage ];
      xdg.configFile."pin-toolkit/config.toml".source = checkedConfig;
    })
  ];
}
