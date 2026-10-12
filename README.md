# Linux Game Trainer Framework

A modular, game-agnostic trainer framework for Linux. Attach to Windows games
running under Wine/Proton, scan their memory with IDA-style signatures, and
apply patches — all driven by per-game TOML configs, no recompilation needed.

> **Scope:** single-player / offline use. Memory manipulation can crash games
> and may violate online Terms of Service — never use this with multiplayer or
> online-connected features.

---

## Features

- **Game-agnostic core** (`trainer_core`): ptrace attach, `/proc/pid/mem`
  read/write, signature scanning, cheat state management, terminal UI.
- **Config-driven games**: each game is a TOML file (process names, module,
  23+ cheats with hotkeys, signatures, patch bytes). Drop a file in `games/`
  and it appears in the CLI.
- **Two frontends**:
  - `trainer_cli run` — interactive fullscreen TUI (navigate, toggle, scan).
  - `trainer_cli apply` — headless single-cheat apply/restore for scripting
    and testing. Scans *without* freezing the game, then attaches only for
    the brief write, with read-back verification and original-bytes backups.
- **Wine-aware**: handles Wine's anonymous executable mappings (only the PE
  header stays file-backed) and non-ASLR ImageBase layouts.
- **Safe by default**: every patch saves original bytes; `disable` restores
  them. Failed scans abort instead of writing blindly.

## Quick start

Requirements: Linux x86_64, Rust 1.70+, a game running under Wine/Proton
(Steam Proton, Lutris, plain Wine), and `CAP_SYS_PTRACE` (root or file
capability) for attaching.

```bash
# Build
cargo build --release

# Grant ptrace capability once (avoids sudo on every run)
sudo setcap cap_sys_ptrace+ep ./target/release/trainer_cli

# List available games
./target/release/trainer_cli list --config-dir games/

# Validate a config
./target/release/trainer_cli validate games/mgsvtpp.toml

# Interactive trainer
./target/release/trainer_cli run "Metal Gear Solid V: The Phantom Pain" --config-dir games/

# Headless: enable one cheat (Ctrl+Num3 = Infinite Fuel in the MGSV config)
./target/release/trainer_cli apply \
  --game "Metal Gear Solid V: The Phantom Pain" \
  --cheat 15 --enable --config-dir games/

# Headless: restore original bytes
./target/release/trainer_cli apply \
  --game "Metal Gear Solid V: The Phantom Pain" \
  --cheat 15 --disable --config-dir games/
```

TUI keys: `↑/↓` navigate · `Enter/Space` toggle cheat · `s` scan signature ·
`a` (re)attach · `r` refresh regions · `q`/`Esc` quit.

## Repository layout

```
.
├── Cargo.toml          # workspace (core + cli)
├── core/               # trainer_core library (game-agnostic)
│   └── src/
│       ├── lib.rs      # public API surface
│       ├── memory.rs   # ptrace attach, /proc/pid/mem, module-base discovery
│       ├── scanner.rs  # IDA-style signature scanner
│       ├── process.rs  # /proc-based process discovery
│       ├── cheats.rs   # Cheat / CheatType / CheatManager
│       ├── config.rs   # TOML game-config schema + loading
│       ├── tui.rs      # interactive terminal UI
│       └── game.rs     # GameModule builder (config -> runnable trainer)
├── cli/                # trainer_cli binary (run/list/validate/init/apply)
├── games/              # per-game TOML configs (e.g. mgsvtpp.toml, 26 cheats)
└── docs/               # detailed documentation
```

## Documentation

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — how the pieces fit together,
  including Wine memory-layout handling and the attach/scan/write lifecycle.
- [`docs/CHEAT_TYPES.md`](docs/CHEAT_TYPES.md) — the five cheat types with
  TOML examples (`write_value`, `nop`, `signature_patch`, `pointer_chain`,
  `set_symbol`).
- [`docs/ADDING_GAMES.md`](docs/ADDING_GAMES.md) — end-to-end guide for adding
  a new game: finding the process, writing signatures, value-scanning for
  data cheats, and the encoding gotchas we hit in practice (×2-packed values,
  static tables vs live counters).
- [`docs/CLI.md`](docs/CLI.md) — full command reference for `trainer_cli`.
- [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) — building, testing, and
  contributing notes.

## Included game: MGSV 1.15 (`games/mgsvtpp.toml`)

26 cheats for *Metal Gear Solid V: The Phantom Pain* (FLiNG hotkey layout
plus F1–F3 resource maxes). Code cheats use signatures sourced from the
community Cheat Engine table; resource cheats use **verified live offsets**
in `.data2`:

| Cheat | Mechanism | Status |
|---|---|---|
| Num 1–0, ± (12 cheats) | signature NOP/patch (health, ammo, suppressor, accuracy, recoil, …) | signatures confirmed present in live build |
| Ctrl+Num 1–9, ± (11 cheats) | signature NOP/patch (GMP, stealth, speed, …) | signatures confirmed present |
| Ctrl+Num 3 / F1–F3 | `write_value` to `.data2` offsets (Fuel `0x2C1267C`, Bio `0x2C12680`, Common `0x2C12684`, Minor `0x2C12688`) | **verified live**: values stored ×2, game caps display at 500000 |

> Offsets are pinned to the tested game build (non-ASLR exe, `.data2`
> globals). After a game update, re-verify offsets; signature cheats are
> resilient to small shifts, static offsets are not.

## Security & safety notes

- Requires `CAP_SYS_PTRACE`. The tool only attaches to processes matching the
  configured names.
- No code injection: only reads/writes existing memory. No network calls.
- Every `apply --enable` stores original bytes under
  `/mnt/scratch/trainer_backups/` (override with `--backup-dir`); `--disable`
  restores them, including the case where the patch destroyed its own
  signature (address recovered from the backup filename).
- Writes are read-back verified; mismatches abort with an error.

## License

MIT — see [LICENSE](LICENSE).
