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

## 2. Code cheats: finding signatures

A signature is a byte pattern that locates the instruction(s) to patch,
without depending on hardcoded addresses. The framework scans executable
regions for it at runtime (`Signature::parse` in `core/src/scanner.rs`;
IDA-style hex with `??`/`?` wildcards).

### 2a. From a Cheat Engine table (fastest — now automated)

Use the importer instead of hand-converting:

```bash
trainer_cli import-ct "Game.CT" -o games/mygame.toml --game "My Game"
```

It extracts, per script entry: the first `aobscanmodule(...)` pattern
(`??`/`?` partial-nibble wildcards like `5?` are widened to `??`),
original bytes from the first `label:` + `db` in `[DISABLE]` (resolving
`define(name, bytes)` symbols) for the patch length, and CE hotkeys
(decimal VK codes → key + ctrl/shift/alt, with notes for momentary
`OnlyWhileDown` and multi-key chords, which import as toggles on the first
key). The exe name is guessed from the most common non-`$process` module
argument. Entries without a usable pattern (Lua-only scripts, value/group
rows) are reported as skipped, not silently dropped.

On the MGSV table (432 KiB) this yields **146 cheats** in one run, e.g.
`Infinite Hp` → `F3 0F 11 4A D4`, len 5, plus `Stun Soldiers [Alt+E]` with
its Alt+E binding intact.

Caveat (printed by every import): CE `alloc`+`jmp newmem` code-cave scripts
become plain **`nop` approximations** over the original bytes — the custom
injected logic is *not* replicated. Treat every import as a candidate:
validate each with `apply --cheat <id> --enable` (uniqueness-checked scan,
read-back-verified write) before trusting it, and hand-tune semantics as
§2b–§2c describe. The manual conversion below explains what the importer
does field by field, for when you need to fix one up by hand.

```
// from Metal Gear Solid V.CT — "Infinite Hp" entry
aobscanmodule(HealthZaa,$process,F3 0F 11 4A D4) // should be unique
...
// ---------- INJECTING HERE ----------
mgsvtpp.exe+11F849B: F3 0F 11 4A D4        - movss [rdx-2C],xmm1
```

Convert to TOML in three steps:

1. **Signature** ← the byte list: `"F3 0F 11 4A D4"`.
2. **Length / patch** ← the `[DISABLE]` block, which restores original
   bytes and therefore tells you both the instruction length and the
   exact original encoding:
   ```
   HealthZaa:
     db F3 0F 11 4A D4     # 5 bytes -> length = 5 for a "nop" cheat
   ```
3. **Semantics** ← the `// ... - <disassembly>` comment: `movss [rdx-2C],xmm1`
   *writes* health, so NOPing it freezes health. If the comment shows a
   *read/compare* (e.g. `cmp`, `test`), NOPing usually disables a check —
   good for stealth/no-detection style cheats.

Worked example (MGSV Infinite Health → `games/mgsvtpp.toml` cheat 1).

### 2b. From your own reverse engineering

1. Attach a disassembler/debugger to the game (x64dbg, IDA, Ghidra, or
   Cheat Engine's debugger + "Find out what writes to this address" on a
   value you located via §3).
2. Identify the target instruction, then select **8–20 bytes** around it
   following these rules:
   - **Keep opcodes, wildcard addresses.** `E8 ?? ?? ?? ??` (call rel32),
     `0F 10 05 ?? ?? ?? ??` (RIP-relative `movss`), and any absolute
     address/immediate that ASLR or versions change → `??` each byte.
     Example: `F3 0F 10 87 6C 02 00 00` keeps the full `movss` + displacement
     (stable), while a neighboring `call` becomes `E8 ?? ?? ?? ??`.
   - **Prefer longer over shorter.** 5-byte patterns (e.g. `09 C2 41 89 13`)
     routinely collide or vanish across builds; 10+ bytes with 1–3 wildcards
     is the sweet spot. Our MGSV scan found 22/23 short patterns but the
     5-byte resource one had **0 hits** in the live build.
   - **Include context, patch precisely.** If only part of the match is the
     patch site, extend the signature over neighboring instructions and use
     `patch_offset` (for `signature_patch`) so the write lands exactly.
3. Note the instruction length (from the disassembler) — that is your
   `length` for `nop`, or the `patch_bytes` span for `signature_patch`.

### 2c. Validate against the live game

Never ship an unscanned pattern. The headless `apply` is the validator —
it scans all executable regions and reports hit counts:

- **1 hit** → good. Record the `module+0xRVA` it prints.
- **0 hits** → version drift (see §4): the bytes changed. Diff the
  disassembly at the old injection offset against the table's commented
  disassembly; look for same-mnemonic-different-encoding swaps (we hit
  `09 C2` → `0B D0`, both `or edx,eax`).
- **2+ hits** → lengthen the pattern, or keep it and pass `--first` only
  after confirming (via surrounding disassembly) that the first hit is the
  right site.

Then finish the loop: `apply --cheat <id> --enable`, verify the effect
in-game, `apply --cheat <id> --disable`, and confirm the restore (read-back
is verified automatically; the original bytes live in the `--backup-dir`
`.bin` file).

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

### Automated drift detection & repair

After a game update, run the scanner — it finds broken patterns and
proposes the fix instead of leaving you to guess:

```bash
# Which patterns broke?
trainer_cli scan --game "My Game" --config-dir games/ --offline

# Apply the repairs to the config
trainer_cli scan --game "My Game" --config-dir games/ --offline --repair
```

Output for a pattern whose bytes changed:

```
  [ 12] Infinite Resources   MISSING
        ↳ drifted; relaxed bytes [0,1] match, suggested: '0B D0 41 89 13'
```

Mechanics: byte positions are progressively relaxed to wildcards and
rescanned (up to `--max-relax`, default 2); the first level that matches
yields the *real* bytes at that address, which is the corrected pattern.
This is exactly how the MGSV `09 C2 41 89 13` → `0B D0 41 89 13` drift was
recovered.

For patterns that still match, `--suggest` prints a **portable** version
with volatile bytes masked:

```
  [  1] Infinite Health      OK    game.exe+0x11F7A2B
        ↳ portable pattern: 'F3 0F 11 4A D4'
```

Masking is done by decoding the instruction (iced-x86): RIP-relative
displacements, relative branch operands and address-like immediates become
`??`, so the pattern keeps matching when surrounding code moves.

## 5. Ship it

1. Add `[[cheats]]` entries with hotkeys (see CHEAT_TYPES.md).
2. `./target/release/trainer_cli validate games/mygame.toml`
3. Test headlessly first: `apply --cheat <id> --enable`, verify in-game,
   then `apply --cheat <id> --disable` and confirm restore.
4. Commit the TOML — no Rust changes needed.
