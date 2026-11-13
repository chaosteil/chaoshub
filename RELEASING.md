# Releasing

A tag `vX.Y.Z` on main starts the release workflow
(`.github/workflows/release.yml`). The workflow runs the checks, builds
the binaries with GoReleaser and publishes them. You do the one-time
setup once, and the release steps for each version.

## One-time setup

### The Homebrew tap token: `HOMEBREW_TAP_TOKEN`

GoReleaser writes `Casks/chaoshub.rb` to `chaosteil/homebrew-tap`. The
default `GITHUB_TOKEN` of the workflow cannot write to another repo
([GoReleaser](https://goreleaser.com/customization/homebrew_casks/#github-actions)).

1. On GitHub, make a fine-grained personal access token
   ([GitHub docs](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#creating-a-fine-grained-personal-access-token)).
2. Set "Repository access" to "Only select repositories", and select
   `chaosteil/homebrew-tap`.
3. Set the repository permission "Contents" to "Read and write".
4. Write down the expiry date. Before this date, make a new token and
   set the secret again.
5. Save the token as a secret of chaoshub
   ([gh secret set](https://cli.github.com/manual/gh_secret_set)):

   ```sh
   gh secret set HOMEBREW_TAP_TOKEN --repo chaosteil/chaoshub
   ```

### The AUR key: `AUR_KEY`

GoReleaser pushes the `PKGBUILD` of `chaoshub-bin` to the AUR (Arch User
Repository) with SSH. The key must not have a passphrase
([GoReleaser](https://goreleaser.com/customization/aur/)).

1. If you do not have an AUR account, make one at
   <https://aur.archlinux.org/register>.
2. Make a new SSH key without a passphrase:

   ```sh
   ssh-keygen -t ed25519 -N '' -C 'chaoshub release' -f ~/.ssh/aur_chaoshub
   ```

3. Add the public key `~/.ssh/aur_chaoshub.pub` to "My Account" on the
   AUR ([AUR submission guidelines](https://wiki.archlinux.org/title/AUR_submission_guidelines#Authentication)).
4. Save the private key as a secret of chaoshub:

   ```sh
   gh secret set AUR_KEY --repo chaosteil/chaoshub < ~/.ssh/aur_chaoshub
   ```

The first push to `ssh://aur@aur.archlinux.org/chaoshub-bin.git` makes
the package `chaoshub-bin` on the AUR
([AUR submission guidelines](https://wiki.archlinux.org/title/AUR_submission_guidelines#Creating_package_repositories)).
Thus the first release makes the package. The package belongs to the
AUR account of the SSH key.

### crates.io

The release workflow publishes to crates.io with Trusted Publishing. With
Trusted Publishing, GitHub Actions gets a short-lived token from
crates.io, and the repo keeps no crates.io secret
([crates.io docs](https://crates.io/docs/trusted-publishing)). Trusted
Publishing needs a crate that exists. Thus you publish the first version
by hand.

1. Log in to <https://crates.io> with GitHub.
2. In "Account Settings", make an API token with the scope
   `publish-new`.
3. Log in with the token:

   ```sh
   cargo login
   ```

4. On main, at the commit of the first release, publish the crate:

   ```sh
   cargo publish
   ```

5. On crates.io, open the "Settings" tab of the crate `chaoshub`. Add a
   trusted publisher with these values:
   - Publisher: GitHub
   - Repository owner: `chaosteil`
   - Repository name: `chaoshub`
   - Workflow filename: `release.yml`
   - Environment: empty
6. Delete the API token of step 2.

After this setup, the `crates-io` job of the release workflow publishes
each new version. If crates.io already has the version of `Cargo.toml`,
the job does nothing. Thus the tag of the version that you published by
hand does not fail.

## Release steps

The steps use jj. The git commands are below the list.

1. Select the new version `X.Y.Z` ([SemVer](https://semver.org)).
2. Start a new change on main:

   ```sh
   jj git fetch
   jj new main -m "chore(release): vX.Y.Z"
   ```

3. Set `version = "X.Y.Z"` in `Cargo.toml`.
4. Run `cargo build`. Cargo writes the new version to `Cargo.lock`.
5. Run the four checks of [AGENTS.md](AGENTS.md).
6. Merge the change to main, with a pull request or a push.
7. Make sure that `main` points to the release commit:

   ```sh
   jj git fetch
   jj log -r main
   ```

8. Tag the release commit, and push the tag:

   ```sh
   jj tag set vX.Y.Z -r main
   jj git push --tag vX.Y.Z
   ```

In a git-only clone, do step 8 with git:

```sh
git fetch origin
git tag vX.Y.Z origin/main
git push origin vX.Y.Z
```

The repo does not use cargo-release, because cargo-release stops on the
detached HEAD of a colocated jj repo.

## What the workflow does

The tag starts three jobs:

- `checks` runs `ci.yml`: the four checks on Linux and on macOS.
- `goreleaser` starts after `checks`. It builds the binaries for
  `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`,
  `x86_64-apple-darwin` and `aarch64-apple-darwin`. Then it makes the
  GitHub release with the archives, the `.deb` and `.rpm` packages,
  `checksums.txt` and the changelog. Then it pushes
  `Casks/chaoshub.rb` to `chaosteil/homebrew-tap` and the `PKGBUILD` to
  the AUR package `chaoshub-bin`.
- `crates-io` starts after `checks`. It publishes the crate to crates.io,
  if crates.io does not have the version yet.

A tag with a prerelease part, for example `v0.2.0-rc.1`, makes a GitHub
prerelease. This tag does not change the Homebrew tap and the AUR. The
`crates-io` job publishes a prerelease version too.

## Make sure that the release works

1. Watch the workflow run:

   ```sh
   gh run watch --repo chaosteil/chaoshub
   ```

2. Make sure that the release has the files:

   ```sh
   gh release view vX.Y.Z --repo chaosteil/chaoshub
   ```

3. Make sure that Homebrew installs the new version:

   ```sh
   brew update
   brew upgrade chaosteil/tap/chaoshub
   chaoshub --version
   ```

4. Make sure that <https://aur.archlinux.org/packages/chaoshub-bin> shows
   the new version.
5. Make sure that <https://crates.io/crates/chaoshub> shows the new
   version.
6. Make sure that cargo-binstall finds the archive:

   ```sh
   cargo binstall --dry-run chaoshub
   ```

If a job fails, read its log. If GoReleaser made a part of the release,
delete the GitHub release before you run the job again:

```sh
gh release delete vX.Y.Z --repo chaosteil/chaoshub
gh run rerun <run-id> --failed --repo chaosteil/chaoshub
```

## Homebrew and the unsigned binary

The macOS binaries are not signed and not notarized. macOS sets a
quarantine flag on a downloaded file, and stops an unsigned binary that
has this flag. The cask removes the flag after the install, with
`postflight_steps` ([GoReleaser](https://goreleaser.com/customization/homebrew_casks/#signing-and-notarizing)).

Homebrew 5.0.0 deprecated casks without code signing. Homebrew also
said that it disables the casks of `Homebrew/homebrew-cask` that fail
Gatekeeper checks in September 2026
([Homebrew 5.0.0](https://brew.sh/2025/11/12/homebrew-5.0.0/)).
Gatekeeper is the macOS check of signed apps. This rule is for the
official cask repo, and `chaosteil/homebrew-tap` is a third-party tap.

Review this decision after the September 2026 enforcement. Make sure
that `brew install chaosteil/tap/chaoshub` still installs and starts
chaoshub on macOS. If it does not, sign and notarize the binaries
([GoReleaser](https://goreleaser.com/customization/sign/notarize/)).
