# Cheat types

Each `[[cheats]]` entry in a game TOML picks one `type`. All types support
enable (patch + back up originals) and disable (restore).

## `nop` — overwrite instructions with `0x90`

Best for: "prevent the game from doing X" (decrement ammo, degrade
suppressor, apply recoil/heat/damage, advance a timer).

```toml
[[cheats]]
id = 2
name = "Infinite Ammo & Grenades"
description = "Weapons never run out of ammo or grenades"
hotkey = { key = "numpad2", modifiers = [] }
type = "nop"
signature = "66 41 89 14 00"
mask = "xxxxx"
length = 5
```

- `signature`: IDA-style hex with `??` wildcards, scanned in executable
  regions. Must be unique (or pass `--first`).
- `length`: bytes to NOP — almost always `==` signature length. Original
  bytes are saved for restore.

## `signature_patch` — overwrite with arbitrary bytes

Best for: writing a tuned value at a code site (e.g. rapid-fire rate,
one-hit-kill zeroing).

```toml
[[cheats]]
id = 10
name = "One Hit Kill"
description = "Enemies die in one hit"
hotkey = { key = "numpad0", modifiers = [] }
type = "signature_patch"
signature = "44 0F B7 42 0C"
mask = "xxxxx"
patch_offset = 0
patch_bytes = [102, 199, 66, 12, 0, 0]
```

- `patch_offset`: signed offset from match start (use with a long,
  unique signature to anchor, then patch a sub-instruction).
- `patch_bytes`: TOML integer array (`[102, …]` = `66 C7 …`). Empty
  `patch_bytes` is rejected by headless `apply` — complex code-cave
  injections (`alloc` + `jmp newmem` in Cheat Engine terms) are **not**
  supported; the framework only overwrites in place.

## `write_value` — write bytes to `base + offset`

Best for: data cheats at verified static addresses (resource counters,
toggles). Offsets are RVAs from the detected module base.

```toml
[[cheats]]
id = 15
name = "Infinite Fuel"
description = "Sets Fuel to the 500000 cap (value stored x2)"
hotkey = { key = "numpad3", modifiers = ["ctrl"] }
type = "write_value"
offset = "0x2C1267C"     # hex or decimal string
value = [64, 36, 15, 0]  # 1000000 LE = 500000 display (stored x2)
```

Caveats (learned the hard way — see ADDING_GAMES.md):

- Static offsets are **build-specific**. Re-verify after game updates.
- Stored encoding may differ from display (MGSV resources are stored
  **doubled**; some games XOR-obfuscate). Verify live before shipping.
- Game-side clamps may cap your value (MGSV caps Fuel display at 500000).

## `pointer_chain` — multi-level pointer write

```toml
type = "pointer_chain"
base_offset = "0x1A2B3C"
offsets = ["0x10", "0x28"]
value = [1, 0, 0, 0]
```

Follows `*(*(*(base+base_offset)+offsets[0])+offsets[1]) = value`. Schema and
runtime support exist; interactive use is still being hardened — prefer
`write_value` with a verified static address where possible.

## `set_symbol` — write a script-exposed value

```toml
type = "set_symbol"
symbol_name = "setSprintSpeed"
value = [0, 0, 128, 63]
```

Mirrors Cheat Engine `registersymbol` values. Headless support is minimal;
use `write_value` after resolving the symbol address once.

## Hotkeys

```toml
hotkey = { key = "numpad1", modifiers = [] }        # Num 1
hotkey = { key = "numpad3", modifiers = ["ctrl"] }  # Ctrl+Num 3
hotkey = { key = "f1", modifiers = [] }             # F1
```

Keys: `numpad0`–`numpad9` (`np0`–`np9` aliases), `add`/`plus`,
`subtract`/`minus`, `multiply`, `divide`, `f1`–`f12`.
Modifiers: `ctrl`/`control`, `shift`, `alt`.
