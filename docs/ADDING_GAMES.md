# Adding a new game

This guide walks through adding a game, using MGSV (`games/mgsvtpp.toml`)
as the worked example. The same steps apply to any Wine/Proton game.

## 0. Scaffold

```bash
./target/release/trainer_cli init --output games/mygame.toml
# then edit metadata/process/cheats
```

```toml
[metadata]
name = "My Game"
version = "1.0"
author = "You"
description = "Short description"
executable_names = ["game.exe"]

[process]
names = ["game.exe"]     # matched against cmdline, maps, and exe symlink
case_insensitive = true
require_wine = true
module_name = "game.exe" # header mapping; base = its lowest address
```

## 1. Confirm process discovery

Start the game, then check the trainer sees it. The headless `apply` prints
the matched PID, or scan `/proc` yourself:

```bash
ps aux | grep -i game.exe
grep -i "game.exe" /proc/<pid>/maps | head
```

Under Wine expect exactly one file-backed mapping (the PE header,
`r--p` at the ImageBase, e.g. `0x140000000`) plus large **anonymous**
`r-xp`/`rwxp` regions — that is normal (see ARCHITECTURE.md).

## 2. Code cheats: signatures

Sources, in order of preference:

1. **Community Cheat Engine tables** (`.CT`): `aobscanmodule(...)` patterns
   plus injection-point comments (`game.exe+OFFSET`) and `[DISABLE]`
   original bytes. This is how all 23 MGSV code cheats were sourced.
2. **Your own RE**: disassemble around interesting behavior, pick 8–20
   unique bytes with `??` for addresses/relocations.

Validate each signature against the **live** game before shipping. Quick
recipe with the repo's own tooling — scan all executable regions and count
hits (the headless `apply` does exactly this and fails unless unique):

- 1 hit → good.
- 0 hits → version drift (see §4) or wrong module.
- 2+ hits → lengthen the pattern or anchor with surrounding bytes and use
  `patch_offset` (for `signature_patch`) to target the sub-instruction.

## 3. Data cheats: value scanning

When no code hook is known (e.g. resource counters), find the live address
by value:

1. Read the exact displayed number in-game (e.g. Fuel `17824`).
2. Scan **all** writable regions (`/proc/pid/maps`, `w` flag) for the
   little-endian encoding — int32 first, then int64/float if needed. Do
   **not** pre-filter to large regions only: the live counter can sit in a
   small mapping.
3. Change the value in-game (spend/gain), rescan survivors for the new
   number. Repeat until 1 address tracks across 2+ changes.

### Gotchas we hit with MGSV (check these before concluding "not found")

- **Encoded storage.** The 27 exact-int32 matches for Fuel were all static
  data tables that never moved. The live counter was stored **doubled**
  (`35360` for displayed `17824`). Cheat Engine sell-code (`shr eax,1`)
  hinted at the codec. Always also scan for `value*2`, `value*2+1`, and
  consider XOR-obfuscation (MGSV xors health with constants).
- **Packed neighbors.** The confirmed Fuel slot sat in a block of
  all-even values — a strong tell for ×2 packing. Consecutive related
  counters (Bio/Common/Minor at +4/+8/+12) confirmed the layout.
- **Display caps.** The game clamps Fuel display at 500000 regardless of
  stored value. Write the cap (stored `1000000`), not more.
- **Static vs live tables.** Random-looking tables can contain your number
  by coincidence (27 did). Only a value that *tracks changes* is live.

## 4. Version drift

Game updates shift code (our CE table said health at `+11F849B`; the live
build had it at `+11F7A2B`) and even instruction encodings (`09 C2` vs
`0B D0` for the same `or edx,eax`). Defenses:

- Prefer **long signatures** over recorded offsets.
- Pin static `write_value` offsets to the tested build in the cheat
  description, and re-verify after updates (one value round-trip suffices
  when the layout is understood).
- Check the exe's ASLR posture: MGSV opts out (`DYNAMIC_BASE=False`), so
  `base + RVA` is restart-stable. If your exe opts in, prefer signatures.

## 5. Ship it

1. Add `[[cheats]]` entries with hotkeys (see CHEAT_TYPES.md).
2. `./target/release/trainer_cli validate games/mygame.toml`
3. Test headlessly first: `apply --cheat <id> --enable`, verify in-game,
   then `apply --cheat <id> --disable` and confirm restore.
4. Commit the TOML — no Rust changes needed.
