{
  description = "tla-connect: TLA+/Apalache integration for model-based testing";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Check if Cargo.lock exists to determine if we can build
        cargoLockExists = builtins.pathExists ./Cargo.lock;

        src = if cargoLockExists then craneLib.cleanCargoSource (craneLib.path ./.) else ./.;

        commonArgs = {
          inherit src;
          pname = "tla-connect";
          version = "0.1.0";
          strictDeps = true;

          buildInputs = [ ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
          ];
        };

        # Only build if Cargo.lock exists
        cargoArtifacts = if cargoLockExists then craneLib.buildDepsOnly commonArgs else null;

        tla-connect = if cargoLockExists then craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
        }) else null;
      in
      {
        checks = pkgs.lib.optionalAttrs cargoLockExists {
          inherit tla-connect;

          tla-connect-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });

          tla-connect-doc = craneLib.cargoDoc (commonArgs // {
            inherit cargoArtifacts;
          });

          tla-connect-fmt = craneLib.cargoFmt {
            inherit src;
            pname = "tla-connect";
            version = "0.1.0";
          };

          tla-connect-nextest = craneLib.cargoNextest (commonArgs // {
            inherit cargoArtifacts;
            partitions = 1;
            partitionType = "count";
          });
        };

        packages = pkgs.lib.optionalAttrs cargoLockExists {
          default = tla-connect;
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            # Rust toolchain (from rust-overlay)
            rustToolchain
            rust-analyzer

            # Cargo tools
            cargo-watch
            cargo-edit
            cargo-outdated
            cargo-audit
            cargo-nextest

            # Development tools
            just
          ];
        };
      });
}
