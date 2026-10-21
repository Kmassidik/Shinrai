{
  description = "shinrai — Cloudflare Zero Trust from the CLI (no dashboard)";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in {
        # Enter with: nix develop
        # Or: direnv allow  (uses .envrc → use flake)
        # `nix build` package comes after Cargo.toml exists (see PLAN.md).
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            # --- Rust toolchain (from nixpkgs, not rustup) ---
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer

            # --- Native deps (reqwest / openssl, etc.) ---
            pkg-config
            openssl

            # --- Tunnel connector (also auto-installed by `shinrai install`) ---
            cloudflared

            # --- General tooling ---
            git
            jq
            curl
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
            pkgs.libiconv
          ];

          shellHook = ''
            export OPENSSL_DIR="${pkgs.openssl.dev}"
            export OPENSSL_LIB_DIR="${pkgs.openssl.out}/lib"
            export PKG_CONFIG_PATH="${pkgs.openssl.dev}/lib/pkgconfig''${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

            echo ""
            echo "shinrai Nix shell ready."
            echo "  rustc:  $(rustc --version)"
            echo "  cargo:  $(cargo --version)"
            echo ""
            echo "Quickstart:"
            echo "  cargo test"
            echo "  cargo run -- install|init|start|status|stop|up"
            echo ""
          '';
        };
      });
}
