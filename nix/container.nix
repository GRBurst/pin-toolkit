# GitLab CI image (spec §8.4): static binary, static busybox (sh, grep),
# CA certificates; no Entrypoint, so GitLab can pipe its script into sh.
{ lib, dockerTools, busybox, pin-toolkit }:
dockerTools.buildLayeredImage {
  name = "pin-toolkit";
  tag = pin-toolkit.version;
  contents = [ pin-toolkit busybox dockerTools.caCertificates ];
  extraCommands = "mkdir -m 1777 tmp";
  config = {
    Env = [ "PATH=/bin" "SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt" ];
    Labels = {
      "org.opencontainers.image.source" = "https://github.com/GRBurst/pin-toolkit";
      "org.opencontainers.image.licenses" = "MIT OR Apache-2.0";
      "org.opencontainers.image.version" = pin-toolkit.version;
    };
  };
}
