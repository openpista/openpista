{
  description = "Nix flake for openpistacrab";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        config = {
          allowUnfree = true;
        };
      };
    in
      {
        packages.openpista = pkgs.rustPlatform.buildRustPackage {
          pname = "openpistacrab";
          version = "0.1.0";
          src = ./.;
          sourceRoot = "source/crates/cli";
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
          nativeBuildInputs = with pkgs; [
            pkg-config
            openssl
          ];
          buildInputs = with pkgs; [
            openssl
          ];
        };

        checks.default = self.packages.${system}.openpista;

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            rustc
            cargo
            rustfmt
            pkg-config
            openssl
            openssl.dev
          ];
        };
      });
}
