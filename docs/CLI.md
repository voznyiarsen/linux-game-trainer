# CLI reference (`trainer_cli`)

Global: every command that loads games accepts `--config-dir <dir>` (repeat
resolution: `--config-dir` → `~/.config/trainer/games/` → `./games/`).

## `list` — show available games

```bash
trainer_cli list --config-dir games/
```

```
Available games:
  Metal Gear Solid V: The Phantom Pain v1.15 - MGSV Trainer with 23 cheats
```

## `run` — interactive TUI trainer

```bash
trainer_cli run "Metal Gear Solid V: The Phantom Pain" --config-dir games/
```

Attempts auto-attach on start; `a` re-attaches. Keys: `↑/↓` navigate,
`Enter`/`Space` toggle, `s` scan selected cheat's signature, `r` refresh
regions, `q`/`Esc` quit. Toggling applies the same scan → backup →
write → verify pipeline as `apply`.

Requires `CAP_SYS_PTRACE` (run once):
`sudo setcap cap_sys_ptrace+ep ./target/release/trainer_cli`.

## `apply` — headless single-cheat apply/restore

```bash
# Enable (patch)
trainer_cli apply --game "<name>" --cheat <id> --enable --config-dir games/ [--first] [--backup-dir DIR]

# Disable (restore original bytes)
trainer_cli apply --game "<name>" --cheat <id> --disable --config-dir games/
```

Behavior:

1. Loads the game config, finds the running process (no freeze).
2. Opens `/proc/pid/mem` **without attaching** and scans for the signature
   (Nop/SignaturePatch) or computes `base + offset` (WriteValue).
3. Prints every hit as `0xADDR (module+0xRVA)`. Fails on 0 hits, or on 2+
   hits unless `--first` is given.
4. Attaches (brief freeze), reads current bytes, saves originals to
   `--backup-dir` (default `/mnt/scratch/trainer_backups/`) as
   `<game>_<cheat>_<addr>.bin`, writes the patch, reads back to verify,
   detaches, confirms the process is alive.
5. On `--disable`, restores from the backup. If the patch destroyed its own
   signature (rescan finds 0 hits), the address is recovered from the
   backup filename instead of failing.

Exit code is non-zero on any failure (scan miss, non-unique, verify
mismatch, missing backup); nothing is written in those cases.

## `import-ct` — import a Cheat Engine table

```bash
trainer_cli import-ct "Game.CT" -o games/mygame.toml [--game "My Game"] [--exe game.exe]
```

Parses `.CT` XML and emits a game TOML: first `aobscanmodule` pattern per
script entry → `nop` cheat (length from `[DISABLE]` original bytes, `define`
symbols resolved), CE hotkeys mapped to key + modifiers, exe guessed from
module args. Prints per-cheat signatures plus a skipped-entries report.
See ADDING_GAMES.md §2a. **Imports are nop approximations — validate each
with `apply` before trusting it.**

## `scan` — validate every signature (drift detection + repair)

```bash
# Live: scan the running game's memory
trainer_cli scan --game "<name>" --config-dir games/ [--suggest] [--repair]

# Offline: scan the exe on disk (no game running required)
trainer_cli scan --game "<name>" --config-dir games/ --offline [--exe /path/to/game.exe]
```

Checks every signature in a game config and reports per cheat:

| Status | Meaning |
|---|---|
| `OK` | exactly one match — the pattern is valid and unique |
| `AMBIGUOUS` | multiple matches — lengthen the pattern or use `patch_offset` |
| `MISSING` | no match — the pattern drifted between game builds |
| `NO-PATTERN` | cheat has no signature (e.g. `write_value`) |

For `MISSING` patterns it **hunts for the drift**: byte positions are
progressively relaxed to wildcards and rescanned; when a relaxed pattern
matches, the real bytes at that address are reported as the repaired
pattern:

```
  [  2] Drifted pattern (2 bytes changed)          MISSING
        ↳ drifted; relaxed bytes [0,4] match, suggested: '0F 11 01 EB 71 48 B9'
```

`--repair` writes the suggested patterns back into the game config (the
config is re-validated afterwards). `--suggest` additionally prints a
**portable pattern** for every OK cheat: the instruction is decoded
(iced-x86) and volatile bytes (RIP-relative displacements, relative
branch operands, address-like immediates) are masked with `??`, so the
pattern survives code movement.

Exit code is 1 when any pattern is ambiguous or missing — usable as a
CI/regression gate after a game update.

## `validate` — check a config file

```bash
trainer_cli validate games/mgsvtpp.toml
```

```
✓ Config valid: Metal Gear Solid V: The Phantom Pain v1.15
  Cheats: 26
  Process names: ["mgsvtpp.exe"]
```

## `init` — scaffold a new game config

```bash
trainer_cli init --output games/new_game.toml
```

Writes a commented template (metadata, process block, one example `nop`
cheat). See ADDING_GAMES.md for filling it in.
