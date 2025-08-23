{
  description = "Bandcamp Ingesting";

  inputs = {
    nixpkgs.url = "nixpkgs";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        # Automatically read rust-toolchain.toml
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustToolchain
            pkg-config
            openssl
            gcc
            chromedriver  # For browser automation
            dbus          # For KWallet/Secret Service
            dbus.dev      # DBus development headers
            # Add any other dependencies your project needs
          ];
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "bandcamp-ingest";
          version = "0.1.0";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          # Use the same toolchain for building
          nativeBuildInputs = [ rustToolchain ];
          buildInputs = with pkgs; [
            # Add runtime dependencies here
          ];
        };
      }
    );
}
