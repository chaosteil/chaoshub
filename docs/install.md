# Install

chaoshub runs on macOS and Linux. It does not run on Windows, because it
uses Unix-only APIs. Each method on this page installs the `chaoshub`
binary. You must also install the programs of
[Requirements](#requirements).

The releases are on the
[release page](https://github.com/chaosteil/chaoshub/releases).
[RELEASING.md](../RELEASING.md) tells how a release is made.

- [Requirements](#requirements)
- [jj and git](#jj-and-git)
- [Homebrew](#homebrew)
- [Cargo](#cargo)
- [cargo-binstall](#cargo-binstall)
- [Nix](#nix)
- [Debian, Ubuntu and Fedora](#debian-ubuntu-and-fedora)
- [Prebuilt archives](#prebuilt-archives)
- [Build from source](#build-from-source)

## Requirements

`git` and `gh` are necessary. jj, nvim and Rex are optional. The
Homebrew cask and the `.deb` and `.rpm` packages also install `git` and
`gh`.

- **git** on your `PATH`. chaoshub finds the repo of the current
  directory with `git remote get-url origin`, and prefetches pull
  request (PR) refs with `git fetch`. In a repo without jj, chaoshub
  also reads your local branches and worktrees with `git`.
- **gh**, the GitHub command line tool, logged in. Run `gh auth login`
  once. chaoshub gets its token from `gh auth token`.
- **jj** (Jujutsu, recommended, not required) on your `PATH`. In a jj
  repo, chaoshub reads your local changes and workspaces with `jj`. jj
  adds these features:
  - The change column shows the jj change id, not the branch name.
  - A PR also matches a local change when the last `/` part of its
    branch name is a prefix of the change id.
  - The scan reads the stack of each workspace, not only the branch
    tips.
  - `i` copies the change id.
- **nvim** with the **codeview** plugin, for the review action (`v`).
- **Rex** (optional). Inside Rex, the review and the PR view open in
  Rex panes beside chaoshub. Without Rex, the review runs in the same
  terminal, and chaoshub comes back when nvim quits. See
  [Rex, Ghostty and the terminal](rex.md).
- **ocr** or **pr-agent** (optional), for the AI review. See
  [Review agents](review-agents.md).

## jj and git

chaoshub selects the tool for each repo. It uses no configuration for
this:

- A repo with a `.jj` directory uses jj when `jj` is on your `PATH`. A
  colocated repo (a jj repo that also has `.git`) also uses jj.
- Every other repo uses git. A colocated repo without `jj` on your
  `PATH` also uses git.

In a git repo, the Mine tab shows the local branch name in the change
column, the short commit in the commit column, and the worktree in the
workspace column. The main worktree is `default`, the same as the
default jj workspace. Another worktree has the name of its directory. A
PR matches a local branch when the branch tip is the head commit of the
PR, or when the branch name is its head branch. After a merge, chaoshub
runs `jj git fetch` in a jj repo and `git fetch origin` in a git repo.

chaoshub does not support a jj repo without `.git`. Use a colocated repo
(`jj git init --colocate`). For a jj repo without `.git`, chaoshub shows
an error in the status line.

## Homebrew

```sh
brew install chaosteil/tap/chaoshub
```

The cask is in
[chaosteil/homebrew-tap](https://github.com/chaosteil/homebrew-tap). The
macOS binary is not signed. The cask removes the quarantine flag of
macOS after the install, so that macOS starts the binary
([GoReleaser](https://goreleaser.com/customization/homebrew_casks/#signing-and-notarizing)).

## Cargo

From crates.io:

```sh
cargo install --locked chaoshub
```

From the main branch on GitHub:

```sh
cargo install --locked --git https://github.com/chaosteil/chaoshub
```

Cargo compiles chaoshub. You need Rust 1.91 or later. `--locked` uses
the dependency versions of `Cargo.lock`
([cargo install](https://doc.rust-lang.org/cargo/commands/cargo-install.html)).

## cargo-binstall

```sh
cargo binstall chaoshub
```

[cargo-binstall](https://github.com/cargo-bins/cargo-binstall) downloads
the prebuilt archive of the GitHub release, and does not compile.

## Nix

Run chaoshub without an install:

```sh
nix run github:chaosteil/chaoshub
```

Install it in your profile:

```sh
nix profile install github:chaosteil/chaoshub
```

Or add the flake as an input of your own flake, for example in a NixOS
configuration:

```nix
{
  inputs.chaoshub.url = "github:chaosteil/chaoshub";

  outputs = { nixpkgs, chaoshub, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        {
          environment.systemPackages = [
            chaoshub.packages.x86_64-linux.default
          ];
        }
      ];
    };
  };
}
```

The flake has packages for `x86_64-linux`, `aarch64-linux` and
`aarch64-darwin`. It does not install `git` and `gh`. Add them to your
packages too.

## Debian, Ubuntu and Fedora

Download the `.deb` or `.rpm` file for your CPU from the
[release page](https://github.com/chaosteil/chaoshub/releases/latest).
Then install it with your package manager:

```sh
sudo apt install ./chaoshub_X.Y.Z_amd64.deb         # Debian, Ubuntu
sudo dnf install ./chaoshub-X.Y.Z-1.x86_64.rpm      # Fedora
```

The packages depend on `git` and `gh`, and recommend `jujutsu` and
`neovim`. If your distribution has no `gh` package, install `gh` first
([GitHub CLI install](https://github.com/cli/cli#installation)).

## Prebuilt archives

The [release page](https://github.com/chaosteil/chaoshub/releases/latest)
has an archive `chaoshub_X.Y.Z_<target>.tar.gz` for each target:

| Target | System |
|---|---|
| `aarch64-apple-darwin` | macOS on Apple silicon |
| `x86_64-apple-darwin` | macOS on Intel |
| `x86_64-unknown-linux-musl` | Linux on x86_64 (static binary) |
| `aarch64-unknown-linux-musl` | Linux on ARM64 (static binary) |

Each archive has the binary, the README and the license files.
`checksums.txt` has the SHA-256 sum of each file.

1. Download the archive and `checksums.txt`.
2. Make sure that the sum is correct:

   ```sh
   sha256sum --check --ignore-missing checksums.txt         # Linux
   shasum -a 256 --check --ignore-missing checksums.txt     # macOS
   ```

3. Extract the binary to a directory on your `PATH`:

   ```sh
   tar -xzf chaoshub_X.Y.Z_aarch64-apple-darwin.tar.gz -C ~/.local/bin chaoshub
   ```

4. If you downloaded the archive with a browser on macOS, remove the
   quarantine flag:

   ```sh
   xattr -d com.apple.quarantine ~/.local/bin/chaoshub
   ```

## Build from source

```sh
git clone https://github.com/chaosteil/chaoshub
cd chaoshub
cargo build --release
```

The binary is `target/release/chaoshub`. To install it in
`~/.cargo/bin`, run `cargo install --locked --path .`.

The repo pins the exact Rust version in `rust-toolchain.toml`. rustup
installs this toolchain when you build in the repo. The devShell of
`flake.nix` (`nix develop`) gives the same version. The minimum Rust
version is 1.91 (`rust-version` in `Cargo.toml`).
