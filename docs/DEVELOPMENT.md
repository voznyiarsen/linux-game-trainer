# Development

## Prerequisites

- Linux x86_64, Rust 1.70+ (`rustup` recommended).
- A Wine/Proton game to test against; `CAP_SYS_PTRACE` for attach
  (`sudo setcap cap_sys_ptrace+ep target/release/trainer_cli`).

## Build & test

```bash
cargo build --release   # binaries: target/release/trainer_cli
cargo test              # unit tests (signatures, cheat model)
cargo clippy            # lints (warnings are pre-existing; keep new code clean)
```

`Cargo.lock` is committed (binary workspace).

## Layout conventions

- `core/` stays game-agnostic. Game-specific knowledge belongs in `games/*.toml`,
  never in Rust code. If you need a new cheat *mechanism*, add a `CheatType`
  variant plus handling in both `tui.rs` (`apply_cheat`) and `cli` (`apply`);
  document it in `docs/CHEAT_TYPES.md`.
- Keep the scan-then-attach discipline: bulk memory reads go through
  `ProcessMemory::open` (no freeze); `attach` is only for short writes.
- `Drop` on `ProcessMemory` detaches — never leave a code path where the
  game stays stopped after an error (return `Err` only *after* detach or via
  RAII).

## Commit style

Small, focused commits: `area: what changed` (e.g. `core: accept any stop signal on attach`,
`games: fix infinite-resources signature for current build`,
`docs: document x2 resource encoding`).

## Releasing

1. Bump versions in `core/Cargo.toml` + `cli/Cargo.toml`.
2. `cargo build --release`, smoke-test `list`/`validate`/`apply --enable` +
   `--disable` against a live game.
3. Tag `git tag vX.Y.Z && git push --tags`.
