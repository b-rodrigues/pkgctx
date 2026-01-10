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

        # R minimal environment
        rWithPackages = pkgs.rWrapper.override {
          packages = [ pkgs.rPackages.BiocManager ];
        };

      in {
        devShells.default = pkgs.mkShell {
          buildInputs = [
            # Rust toolchain
            rustToolchain
            pkgs.pkg-config

            # R for package introspection
            rWithPackages

            # Python with pip for PyPI downloads
            (pkgs.python312.withPackages (ps: [ ps.pip ]))

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
