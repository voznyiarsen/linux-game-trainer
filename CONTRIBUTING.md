# Contributing

Thanks for helping out. Single-player/offline scope only — no multiplayer,
anti-cheat circumvention, or online-economy tooling.

## How to contribute

- **New game configs** (`games/*.toml`) are the most welcome contribution.
  Follow [`docs/ADDING_GAMES.md`](docs/ADDING_GAMES.md): every signature or
  offset must be verified against a live game build, and the tested build
  noted in the cheat description.
- **Core changes**: keep `core/` game-agnostic; add tests for scanners and
  config parsing; update `docs/` alongside behavior changes.
- Run `cargo test` and `cargo build --release` before opening a PR.

## Reporting issues

Include: game name + build/version, Wine/Proton version, the cheat id and
config used, and full `apply` output (scan hits, addresses). For wrong-value
writes, the original-bytes backup filename and a hexdump of the read-back
help a lot.
