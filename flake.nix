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
      # The devShell uses the Rust of nixpkgs. CI uses rustup and the exact
      # version of rust-toolchain.toml. When a nixpkgs update gives another
      # version, the devShell does not start. Then set the new version in
      # rust-toolchain.toml too, so that the two stay equal.
      toolchain = (lib.importTOML ./rust-toolchain.toml).toolchain.channel;
      sameRust =
        pkgs:
        lib.assertMsg (pkgs.rustc.version == toolchain && pkgs.clippy.version == toolchain)
          "nixpkgs has rustc ${pkgs.rustc.version} and clippy ${pkgs.clippy.version}, but rust-toolchain.toml has ${toolchain}";
    in
    {
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = cargoToml.package.name;
          inherit (cargoToml.package) version;

          # Only the files of the build and the tests. A change to the
          # README or to docs/ does not start a new build.
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
          # jj and nvim are optional. See docs/install.md.
          meta = {
            inherit (cargoToml.package) description;
            # Cargo.toml has no homepage field: the repository is the homepage.
            homepage = cargoToml.package.repository;
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
        default =
          assert sameRust pkgs;
          pkgs.mkShell {
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
