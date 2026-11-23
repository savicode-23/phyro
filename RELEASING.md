# Release Phyro

## Prepare a draft on GitHub

1. Set a new `version` under `[workspace.package]` in `Cargo.toml`. For example, use `0.1.0-alpha.2` for the next alpha or `0.1.0` for the stable version. Run `cargo check --workspace` to refresh `Cargo.lock`. To include highlights in the draft, write them in `release/notes/<version>.md`. Commit the version, lockfile and notes to `main`.
2. Open [Actions → Release](https://github.com/savicode-23/phyro/actions/workflows/release.yml), click **Run workflow**, select the default branch, and enable **Create a draft GitHub Release after both packages pass**. Leave it unchecked to build and verify the archives as Actions artifacts.
3. When the run finishes, open [Releases](https://github.com/savicode-23/phyro/releases) while signed into an account with write access. The draft uses the tested commit and its version tag. Review the notes, download the archives under **Assets**, and try a sample project. Prerelease versions such as `0.1.0-alpha.1` are marked as pre-releases.
4. Choose **Publish release** when the build is ready for testers.

Maintainers using Git can also push a tag matching the committed workspace version, such as `v0.1.0-alpha.2`. This triggers the same draft workflow. If a tagged version needs a code change, bump the version and prepare a new tag.

## Downloads

| File | Contents |
| --- | --- |
| `Phyro-<version>-windows-x64.zip` | Windows x64 editor, CLI, examples, guides and third-party notices |
| `Phyro-<version>-linux-x64.tar.gz` | Ubuntu 24.04 x64 build with the same tools, examples and guides |
| `SHA256SUMS` | SHA-256 checksum of each downloadable archive |

For versions with build metadata, filenames use an underscore for the `+` in the version. The tag and the version reported by Phyro keep the original `+`.

On Linux, download all three files into one folder and run `sha256sum -c SHA256SUMS`. On Windows, compare `(Get-FileHash .\Phyro-<version>-windows-x64.zip -Algorithm SHA256).Hash` in PowerShell with the ZIP entry in `SHA256SUMS`, substituting the version you downloaded.

Save the matching Windows editor PDB from the Actions run alongside a copy of the ZIP before the workflow artifacts expire. The editor records its Git revision in crash reports so maintainers can match a report to the correct symbols.
