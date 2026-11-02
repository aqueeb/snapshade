# Contributing to Snapshade

Thanks for your interest. Snapshade is small on purpose: a hotkey, an overlay, a handful of
annotation tools, copy / save / print, and nothing that talks to a network. Changes that keep it
that way are welcome.

## Ground rules

- **No networking.** The binary must not gain an HTTP client, an update check or telemetry.
  Links are handed to the operating system's browser.
- **Both operating systems.** Behaviour lives in [docs/SPEC.md](docs/SPEC.md) and is the same
  on macOS and Windows. Platform differences belong in `crates/platform-macos` and
  `crates/platform-windows` behind the traits in `crates/platform-api`; the core crate and the
  TypeScript core stay pure and headless-testable.
- **Latency is a feature.** Anything on the hotkey → overlay path needs a before/after number.

## Setting up

Rust 1.90 or later (`rustup`), [bun](https://bun.sh), and the platform toolchain: Xcode
command-line tools on macOS; Visual Studio Build Tools with the Windows 10/11 SDK on Windows.

```sh
bun install
bun run build
cargo build -p snapshade
```

On macOS, run development builds through `scripts/sign-and-run.sh` with
`SNAPSHADE_SIGN_IDENTITY` and `SNAPSHADE_TEAM_ID` set to your Apple Development identity, so
the Screen Recording grant survives rebuilds.

## Before you open a pull request

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bun run --cwd ui test
```

CI runs the same set on macOS and Windows. Please:

- keep pull requests focused and describe the behaviour change, with the spec section it touches;
- add or update tests next to the code (`*.test.ts` in `ui/src`, `#[cfg(test)]` modules in Rust);
- update `docs/SPEC.md` when behaviour changes and `CHANGELOG.md` under *Unreleased*;
- note which operating systems you tested on.

## Reporting bugs

Open an issue with the operating system and version, the display setup (count, resolution,
scaling), what you did, what happened, and what you expected. For capture or overlay problems a
screenshot of the result helps more than a description.

## Code of conduct

Participation is covered by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
