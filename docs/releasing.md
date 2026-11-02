# Releasing punch

M6 packaging is integrated **after the TUI claim/sync change**. Do not tag the
packaging branch independently. Issue [#11](https://github.com/ContractorKeith/punch/issues/11)
tracks both pieces; this PR does not close it.

## Artifacts

The stable workspace version in `Cargo.toml` must match the tag exactly:
version `0.1.0` requires `v0.1.0`. Prereleases are not supported by this small
workflow. Keep `Cargo.lock` in sync with manifest version bumps; all compilation
and Rust tests use `--locked`.

For v0.1.0, the final `release-assets` Actions artifact contains:

| File | Platform |
| --- | --- |
| `punch-v0.1.0-x86_64-unknown-linux-musl.tar.gz` | Linux x86_64, static musl |
| `punch-v0.1.0-aarch64-apple-darwin.tar.gz` | macOS Apple Silicon |
| `punch-v0.1.0-x86_64-apple-darwin.tar.gz` | macOS Intel |
| `SHA256SUMS` | SHA-256 of the three archives |
| `punch.rb` | Homebrew formula with hashes calculated from those archives |

Each archive has `punch`, `LICENSE`, and `README.md` at its root. The bundled
SQLite builds into the executable; no database service is needed. macOS builds
are unsigned and not notarized. The workflow uses Rust 1.98.1, Ubuntu 24.04
with `musl-tools`, and native macOS 15 runners. macOS deployment support is the
runner/toolchain default; older macOS versions have not been certified.

Runner labels were checked against GitHub's official
[runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
on 2026-11-02: `ubuntu-24.04` is x86_64, `macos-15` is arm64, and
`macos-15-intel` is x86_64. Recheck before changing these labels.

## Validate without publishing

Every PR runs the release workflow. You can also run `release` with
`workflow_dispatch` from the Actions UI on the desired branch. Leave `tag`
empty to validate the workspace version, or enter `v0.1.0` to exercise exact
tag validation. **Manual and PR runs cannot enter the publish job**, even when
the selected ref is a tag. They upload only Actions artifacts.

The workflow checks formatting, Clippy, workspace tests, target tests, locked
release builds, and smoke tests of the extracted binary (`--help` and a fresh
`--json list`). Each command runs with a disposable `PUNCH_STATE_DIR`; fake
`gh` and `basecamp` executables are asserted first on `PATH`. Unexpected remote
CLI use fails. It then gathers all three packages, computes checksums, and
generates the formula. The read-only jobs have only `contents: read` and do
not retain checkout credentials. Only the tag-only publish job has
`contents: write`; no tap credential is used.

Local validation on Linux (keep build files off shared `/tmp`):

```bash
export CARGO_TARGET_DIR="$HOME/.cache/punch-release/target"
export TMPDIR="$HOME/.cache/punch-release/tmp"
mkdir -p "$CARGO_TARGET_DIR" "$TMPDIR"
bash packaging/check.sh python3 -m unittest discover -s packaging -v
bash packaging/check.sh cargo fmt --all --check
bash packaging/check.sh cargo clippy --locked --workspace --all-targets -- -D warnings
bash packaging/check.sh cargo test --locked --workspace
bash packaging/check.sh cargo build --locked --release
```

For the Linux release package, install your platform's musl C compiler and
`rustup target add x86_64-unknown-linux-musl`, then:

```bash
bash packaging/check.sh cargo build --locked --release --bin punch --target x86_64-unknown-linux-musl
python3 packaging/release.py package --target x86_64-unknown-linux-musl \
  --binary "$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/punch" --out "$HOME/.cache/punch-release/dist"
python3 packaging/release.py smoke \
  --archive "$HOME/.cache/punch-release/dist/punch-v0.1.0-x86_64-unknown-linux-musl.tar.gz"
```

The Python helpers require Python 3.11+. Native macOS build, execution and
Homebrew installation must be verified on each architecture in CI/on a Mac;
a Linux host cannot provide that evidence. Download `release-assets` from a
successful manual run to inspect the complete candidate without publishing it.

## Publish a reviewed version

The release owner first merges the TUI work, then packaging, and validates the
combined main commit with a manual run. Update this changelog with a dated
version heading, review the manifest/lock version, and check all three native
smoke tests. Only then should the owner push the matching `v<version>` tag on
the reviewed main commit. A tag push builds the packages again and publishes
them with `gh release create --verify-tag`. It does not create a tag or write
to the tap. A failure after publication needs inspection before retrying;
existing release assets are never overwritten automatically.

## Update Homebrew (owner action)

The existing [ContractorKeith/homebrew-tap](https://github.com/ContractorKeith/homebrew-tap)
uses `Formula/*.rb`. This repository does not modify it or publish to it.
After the GitHub release exists:

1. Download the three final release archives, `SHA256SUMS`, and `punch.rb` from
   that release into one directory. Verify with `sha256sum --check SHA256SUMS`
   (Linux) or `shasum -a 256 --check SHA256SUMS` (macOS).
2. If regenerating, run `python3 packaging/release.py assemble --dir <directory>`
   from the matching punch tag. It refuses missing archives and hashes the
   real files; do not fill in placeholder hashes or reuse candidate-run hashes.
3. Review URLs/version/hashes, copy `punch.rb` to `Formula/punch.rb` in the tap,
   and run `brew audit --strict --online ContractorKeith/tap/punch`,
   `brew install --formula ./Formula/punch.rb`, and `brew test punch` on
   supported hosts. Review and publish that tap change separately.
4. Once published, verify `brew install ContractorKeith/tap/punch` and
   `brew upgrade punch`. Linux arm64 is intentionally unsupported.
