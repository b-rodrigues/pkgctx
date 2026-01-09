{
  description = "pkgctx — compile software packages into LLM-ready context";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        # Pin Rust version for determinism
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "clippy" "rustfmt" ];
        };

        # Build rix from GitHub for testing
        rix = pkgs.rPackages.buildRPackage {
          name = "rix";
          src = pkgs.fetchFromGitHub {
            owner = "ropensci";
            repo = "rix";
            rev = "main";
            sha256 = "sha256-HrZMwJfbUOdmwXWN8CmzDOLgJwqRc3e3s5qS96oSUC0=";
          };
          propagatedBuildInputs = with pkgs.rPackages; [
            codetools curl jsonlite sys
          ];
        };

        # R with packages needed for introspection and rix testing
        rWithPackages = pkgs.rWrapper.override {
          packages = with pkgs.rPackages; [
            # Core packages for introspection
            jsonlite
            # rix for testing
            rix
          ];
        };

      in {
        devShells.default = pkgs.mkShell {
          buildInputs = [
            # Rust toolchain
            rustToolchain
            pkgs.pkg-config

            # R for package introspection
            rWithPackages

            # Python for future Python support
            pkgs.python312

            # Development tools
            pkgs.cargo-watch
          ];

          shellHook = ''
            echo "pkgctx dev shell: Rust $(rustc --version | cut -d' ' -f2), R $(R --version | head -1 | cut -d' ' -f3)"
          '';

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "pkgctx";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.pkg-config ];

          meta = with pkgs.lib; {
            description = "Compile software packages into LLM-ready context";
            homepage = "https://github.com/b-rodrigues/pkgctx";
            license = licenses.gpl3Plus;
          };
        };
      }
    );
}
