{
  description = "chaoshub: a terminal app that tracks your GitHub pull requests";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;
      # chaoshub uses Unix-only APIs. nixpkgs 26.11 has no x86_64-darwin.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      cargoToml = lib.importTOML ./Cargo.toml;
    in
    {
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = cargoToml.package.name;
          inherit (cargoToml.package) version;

          # Only the files of the build and the tests. A change to the
          # README does not start a new build.
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./src
              ./tests
              ./.config
            ];
          };
          cargoLock.lockFile = ./Cargo.lock;

          # The cargo hook of nixpkgs links rusqlite to the system SQLite,
          # not to the bundled copy.
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.sqlite ];

          # nextest reads .config/nextest.toml: the timing test runs alone.
          useNextest = true;
          # Some tests use git. With jj, the tests also read a jj repo.
          nativeCheckInputs = [
            pkgs.git
            pkgs.jujutsu
          ];

          # At run time, chaoshub needs git and gh (logged in) on the PATH.
          # jj and nvim are optional. See the README.
          meta = {
            inherit (cargoToml.package) description homepage;
            license = with lib.licenses; [
              mit
              asl20
            ];
            mainProgram = "chaoshub";
            platforms = lib.platforms.unix;
          };
        };
      });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          meta.description = cargoToml.package.description;
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
          packages = [
            pkgs.cargo-nextest
            pkgs.clippy
            pkgs.rustfmt
            pkgs.gh
          ];
        };
      });
    };
}
