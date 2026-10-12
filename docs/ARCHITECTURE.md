# Architecture

This document describes how `trainer_core` works and why it is shaped the way
it is — in particular the Wine-specific handling, which is the main source of
complexity.

## Crates

- **`trainer_core`** — everything game-agnostic: memory access, scanning,
  process discovery, cheat model, config schema, TUI, module builder.
- **`trainer_cli`** — thin binary: parses CLI args, loads configs via
  `GameModuleBuilder`, runs either the interactive `TrainerApp` or the
  headless `apply` flow.

Games contribute **no code** — only TOML files (see
[ADDING_GAMES.md](ADDING_GAMES.md)).

## Memory access (`core/src/memory.rs`)

`ProcessMemory` wraps two OS facilities:

1. **ptrace** (`PTRACE_ATTACH`) — freezes the target so a write lands
   atomically from the game's perspective. Important subtlety we hit in
   production: `PTRACE_ATTACH` delivers **SIGSTOP, not SIGTRAP**. Waiting
   specifically for `Stopped(SIGTRAP)` hangs forever and leaves the game
   frozen; the code accepts any `Stopped` state.
2. **`/proc/pid/mem`** — the actual read/write channel (`seek` + `read` /
   `write`). With `yama/ptrace_scope = 0`, same-UID **reads work without
   attaching**, which the headless flow exploits: scan first (no freeze),
   attach only for the millisecond-scale write, then detach.

Two constructors reflect this:

- `ProcessMemory::open(pid)` — no attach, for scanning.
- `ProcessMemory::attach(pid)` — attach + wait-for-stop, for writing.
  `Drop` detaches, so a crashed caller still resumes the game (the kernel
  also auto-detaches a dead tracer).

`refresh_regions()` parses `/proc/pid/maps` into `MemoryRegion`s.

### Wine memory layout handling

A Windows exe under Wine does **not** look like a native Linux process:

- Only the PE header (`0x140000000–0x140001000`, `r--p`, file-backed) keeps
  its file path. All PE sections (code *and* data) appear as **anonymous**
  mappings (`r-xp` code, `rw-p`/`r--p` data, plus large `rwxp` areas).
- Therefore `find_module_base()` returns the **lowest mapping whose path
  contains the module name** (the header), without requiring `x`
  permission — the old `x`-only check found nothing under Wine.
- `get_game_regions()` (scan targets) includes anonymous executable mappings
  within a window above a known exe mapping (covers the canonical
  `0x140000000–0x150000000` PE range) in addition to file-backed `.exe`
  mappings. Without this, signature scans cover ~0 bytes of game code.

The main exe is typically non-ASLR (`DYNAMIC_BASE=False`), so
`base + RVA` addresses are stable across restarts for a given build — this
is what makes static-offset `write_value` cheats viable. Offsets still need
re-verification after game updates.

## Signature scanner (`core/src/scanner.rs`)

- `Signature::parse("F3 0F 11 4A D4 ... ?? ...")` — hex bytes, `??`/`?`
  wildcards.
- `SignatureScanner::scan_region` reads in 1 MiB chunks over `/proc/pid/mem`
  and matches in userspace, with overlap handling so matches spanning chunk
  boundaries are not missed.
- `scan_game_code()` scans `get_game_regions()`; callers decide uniqueness
  policy (headless `apply` fails on 0 or 2+ hits unless `--first`).

Performance: ~144 MiB of executable mappings scans in seconds; the TUI
reuses the same path for its `s` (scan) key.

## Process discovery (`core/src/process.rs`)

`find_process(&ProcessConfig)` walks `/proc`, matching `names` (default
case-insensitive) against `cmdline`, `maps`, and the `exe` symlink. Matching
`maps` as well as `cmdline` matters under Wine, where the interesting string
(`mgsvtpp.exe`) often appears only as the header mapping path.

## Cheat model (`core/src/cheats.rs`, `core/src/config.rs`)

- `config.rs` defines the **TOML schema** (`GameConfig`, `CheatConfig`,
  `HotkeyConfig`, `CheatTypeConfig`) plus `load_from_file` /
  `load_from_directory` and conversion to runtime types. See
  [CHEAT_TYPES.md](CHEAT_TYPES.md).
- `cheats.rs` defines runtime `Cheat` (with `enabled` flag + `original`
  bytes for restore) and `CheatManager` (toggle, hotkey lookup).

## Frontends

- **TUI** (`core/src/tui.rs`, `TrainerApp`): crossterm fullscreen loop.
  `↑/↓` + `Enter` toggles cheats through the same apply logic as headless
  (scan → backup originals → write → verify). Status bar reports results.
- **Headless `apply`** (`cli/src/main.rs`): resolve address (scan without
  freeze) → attach → read current → write patch → read-back verify →
  detach → confirm liveness. Backups live in `--backup-dir` (default
  `/mnt/scratch/trainer_backups/`) as `<game>_<cheat>_<addr>.bin`, which is
  also how `--disable` recovers the address when the patch destroyed its
  own signature.

## `GameModule` (`core/src/game.rs`)

`GameModuleBuilder` aggregates configs from `--config-dir`,
`~/.config/trainer/games/`, and `./games/`, then builds runnable
`GameModule`s by name. This is the seam where future plugin loading (`.so`
game modules) would attach — config files remain the default.
