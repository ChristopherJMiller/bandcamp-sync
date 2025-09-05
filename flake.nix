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
          nativeBuildInputs = with pkgs; [
            rustToolchain
            pkg-config
          ];
          
          buildInputs = with pkgs; [
            openssl
            geckodriver
            dbus.dev
            
            # CD ripping and audio processing
            cdparanoia
            ffmpeg_7-full
          ];
        };

        packages = rec {
          # Main binary
          bandcamp-sync = pkgs.rustPlatform.buildRustPackage {
            pname = "bandcamp-sync";
            version = "0.3.0";
            src = ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            # Build dependencies
            nativeBuildInputs = with pkgs; [
              rustToolchain
              pkg-config
              installShellFiles
            ];

            # Runtime dependencies
            buildInputs = with pkgs; [
              openssl
              dbus
              cdparanoia
              ffmpeg_7-full
            ];

            # Build configuration
            env = {
              # Link against system OpenSSL instead of vendoring
              OPENSSL_NO_VENDOR = "1";
            };

            # Post-install setup
            postInstall = ''
              # Generate and install shell completions
              installShellCompletion --cmd bandcamp-sync \
                --bash <($out/bin/bandcamp-sync completion bash) \
                --fish <($out/bin/bandcamp-sync completion fish) \
                --zsh <($out/bin/bandcamp-sync completion zsh)
            '';

            meta = with pkgs.lib; {
              description = "Sync Bandcamp purchases to WebDAV music library";
              homepage = "https://github.com/chris-miller/bandcamp-sync";
              license = licenses.gpl3;
              maintainers = [ ];
              platforms = platforms.unix;

              longDescription = ''
                A CLI tool to sync your Bandcamp music collection to WebDAV storage or local folders.
                Features parallel downloads, smart incremental sync, flexible filtering, and support
                for multiple audio formats. Requires a WebDriver (geckodriver, chromedriver, or safaridriver)
                for authentication.
              '';
            };
          };

          # Default package
          default = bandcamp-sync;

          # WebDriver bundles for different browsers
          with-firefox = pkgs.symlinkJoin {
            name = "bandcamp-sync-with-firefox";
            paths = [
              bandcamp-sync
              pkgs.geckodriver
            ];
            meta = bandcamp-sync.meta // {
              description = "Bandcamp Sync with Firefox WebDriver (geckodriver)";
            };
          };

          with-chrome = pkgs.symlinkJoin {
            name = "bandcamp-sync-with-chrome";
            paths = [
              bandcamp-sync
              pkgs.chromedriver
            ];
            meta = bandcamp-sync.meta // {
              description = "Bandcamp Sync with Chrome WebDriver (chromedriver)";
            };
          };

          with-all-drivers = pkgs.symlinkJoin {
            name = "bandcamp-sync-with-all-drivers";
            paths = [
              bandcamp-sync
              pkgs.geckodriver
              pkgs.chromedriver
            ];
            meta = bandcamp-sync.meta // {
              description = "Bandcamp Sync with all supported WebDrivers";
            };
          };
        };
      }
    );
}
