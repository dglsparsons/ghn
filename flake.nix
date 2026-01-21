{
  description = "A fast, keyboard-driven TUI for GitHub notifications.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, crane, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        craneLib = crane.mkLib pkgs;

        rustSrc = craneLib.cleanCargoSource ./.;

        commonArgs = {
          src = rustSrc;
          pname = "ghn";
          version = "0.1.0";

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; lib.optionals stdenv.isDarwin [
            apple-sdk_15
          ] ++ lib.optionals stdenv.isLinux [
            wayland
            libxkbcommon
          ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        ghn = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
        });
      in
      {
        packages = {
          inherit ghn;
          default = ghn;
        };

        checks = {
          inherit ghn;

          clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets --all-features -- -D warnings";
          });

          rust-tests = craneLib.cargoTest (commonArgs // {
            inherit cargoArtifacts;
            cargoTestExtraArgs = "--all-targets --all-features";
          });

          fmt = craneLib.cargoFmt {
            src = rustSrc;
            pname = "ghn";
            version = "0.1.0";
          };
        };
      }
    ) // {
      overlays.default = final: prev: {
        ghn = self.packages.${prev.system}.ghn;
      };
    };
}
