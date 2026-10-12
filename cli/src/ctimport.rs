//! Import Cheat Engine (.CT) tables into game TOML configs.
//!
//! Supported subset (enough for typical community tables):
//! - `aobscanmodule(<sym>,<module>,<hex bytes with ??/? wildcards>)`
//! - `define(<name>,<hex bytes>)` resolution for symbolic `db <name>`
//! - `[DISABLE]` `<label>:\n db <hex>` original bytes -> patch length
//! - `<Hotkeys>` CE decimal VK codes -> key + modifiers
//!
//! Semantic gap (stated loudly, see report): CE `alloc`+`jmp newmem`
//! code-cave scripts are imported as plain `nop` approximations over the
//! original bytes. Every generated cheat must be validated live with
//! `apply --enable` before trusting it.

use anyhow::{Context, Result};
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use trainer_core::config::{
    CheatConfig, CheatTypeConfig, GameConfig, GameMetadata, GameSettings,
    HotkeyConfig, ProcessConfig,
};
use trainer_core::cheats::{ModifierKey, VirtualKey};

pub struct ImportReport {
    pub imported: Vec<ImportedCheat>,
    pub skipped: Vec<(String, String)>,
}

pub struct ImportedCheat {
    pub id: u32,
    pub name: String,
    pub signature: String,
    pub length: usize,
    pub notes: Vec<String>,
}

/// Import a .CT file into a `GameConfig`.
///
/// `exe_fallback` is used when no module name can be guessed from the
/// table's `aobscanmodule` calls.
pub fn import_ct(
    path: &Path,
    game_name: Option<String>,
    exe_fallback: Option<String>,
) -> Result<(GameConfig, ImportReport)> {
    let xml = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let doc = roxmltree::Document::parse(&xml).context("Failed to parse Cheat Table XML")?;

    let aob_re = Regex::new(r"(?i)aobscanmodule\(\s*([^,]+?)\s*,\s*([^,]+?)\s*,\s*([0-9A-Fa-f\?x][0-9A-Fa-f\?x\s]*)\)")?;
    let define_re = Regex::new(r"(?mi)^\s*define\(\s*(\w+)\s*,\s*([0-9A-Fa-f][0-9A-Fa-f\s]*)\)")?;
    let disable_db_re =
        Regex::new(r"(?m)^\s*(\w+)\s*:\s*\r?\n\s*db\s+([0-9A-Za-z_][0-9A-Za-z_\s]*)")?;

    // Guess exe name from aobscan module args (most common non-$process value).
    let mut module_votes: HashMap<String, usize> = HashMap::new();
    for caps in aob_re.captures_iter(&xml) {
        let module = caps[2].trim().trim_matches('"').to_string();
        if !module.eq_ignore_ascii_case("$process") && !module.is_empty() {
            *module_votes.entry(module).or_default() += 1;
        }
    }
    let guessed_exe = module_votes.into_iter().max_by_key(|(_, n)| *n).map(|(m, _)| m);
    let exe = exe_fallback.or(guessed_exe).unwrap_or_else(|| "game.exe".to_string());

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported Game");
    let name = game_name.unwrap_or_else(|| stem.to_string());

    let mut report = ImportReport { imported: Vec::new(), skipped: Vec::new() };
    let mut cheats: Vec<CheatConfig> = Vec::new();
    let mut next_id: u32 = 1;

    // All CheatEntry nodes in document order (parents before children).
    let entries: Vec<roxmltree::Node> = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "CheatEntry")
        .collect();

    for entry in entries {
        let desc = child_text(&entry, "Description")
            .map(|s| s.trim().trim_matches('"').to_string())
            .unwrap_or_default();
        if desc.is_empty() {
            continue;
        }
        let script = child_text(&entry, "AssemblerScript").unwrap_or_default();
        if script.trim().is_empty() {
            report.skipped.push((desc, "no AssemblerScript (group header / value entry)".into()));
            continue;
        }

        // defines for symbolic db resolution
        let mut defines: HashMap<String, Vec<u8>> = HashMap::new();
        for caps in define_re.captures_iter(&script) {
            if let Some(bytes) = parse_hex_bytes(&caps[2]) {
                defines.insert(caps[1].to_string(), bytes);
            }
        }

        // first aob pattern
        let aob = aob_re.captures(&script).map(|caps| {
            let sym = caps[1].trim().to_string();
            let module = caps[2].trim().to_string();
            let pattern = normalize_pattern(&caps[3]);
            (sym, module, pattern)
        });
        let Some((aob_sym, _module, pattern)) = aob else {
            report.skipped.push((desc, "no aobscan pattern (Lua-only or manual script)".into()));
            continue;
        };
        if pattern.split_whitespace().count() < 3 {
            report.skipped.push((desc, "aob pattern too short (<3 bytes)".into()));
            continue;
        }

        // original bytes from [DISABLE] section
        let disable_part = script.split("[DISABLE]").nth(1).unwrap_or("");
        let mut length: Option<usize> = None;
        for caps in disable_db_re.captures_iter(disable_part) {
            let val = caps[2].trim();
            if let Some(bytes) = defines.get(val).cloned().or_else(|| parse_hex_bytes(val)) {
                length = Some(bytes.len());
                break;
            }
        }
        let sig_len = pattern.split_whitespace().count();
        let length = length.unwrap_or(sig_len);

        // hotkeys: direct Hotkeys children only (not nested children's)
        let mut notes: Vec<String> = Vec::new();
        let mut hotkey_cfg = HotkeyConfig { key: "f1".to_string(), modifiers: vec![] };
        let mut hotkey_found = false;
        for hotkeys in entry.children().filter(|n| n.is_element() && n.tag_name().name() == "Hotkeys") {
            for hk in hotkeys.children().filter(|n| n.is_element() && n.tag_name().name() == "Hotkey") {
                let codes: Vec<u16> = hk
                    .descendants()
                    .filter(|n| n.is_element() && n.tag_name().name() == "Key")
                    .filter_map(|n| n.text().and_then(|t| t.trim().parse().ok()))
                    .collect();
                if codes.is_empty() {
                    continue;
                }
                let mut mods: Vec<String> = Vec::new();
                let mut main: Option<VirtualKey> = None;
                let mut dropped: Vec<u16> = Vec::new();
                for code in codes {
                    if let Some(m) = VirtualKey::modifier_for_vk_code(code) {
                        let s = match m {
                            ModifierKey::Control => "ctrl",
                            ModifierKey::Shift => "shift",
                            ModifierKey::Alt => "alt",
                        };
                        if !mods.contains(&s.to_string()) {
                            mods.push(s.to_string());
                        }
                    } else if main.is_none() {
                        main = VirtualKey::from_vk_code(code);
                        if main.is_none() {
                            notes.push(format!("unmapped VK code {}", code));
                        }
                    } else {
                        dropped.push(code);
                    }
                }
                if !dropped.is_empty() {
                    notes.push(format!(
                        "extra CE keys ignored: {} (chords unsupported, first key kept)",
                        dropped.iter().map(|c| c.to_string()).collect::<Vec<_>>().join("+")
                    ));
                }
                if let Some(k) = main {
                    hotkey_cfg = HotkeyConfig { key: k.config_name().to_string(), modifiers: mods };
                    hotkey_found = true;
                }
                let action = hk
                    .children()
                    .find(|n| n.is_element() && n.tag_name().name() == "Action")
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .to_string();
                let only_while_down = hk.attribute("OnlyWhileDown").unwrap_or("0") == "1";
                if only_while_down {
                    notes.push("CE hotkey is momentary (OnlyWhileDown); imported as toggle".into());
                }
                if action.eq_ignore_ascii_case("Set Value") {
                    notes.push("CE hotkey sets a value; imported as toggle (verify!)".into());
                }
                break; // first hotkey wins
            }
        }
        if !hotkey_found {
            notes.push("no CE hotkey; bound to F1 placeholder".into());
        }

        let mask = "x".repeat(sig_len);
        let mut full_desc = format!("{} (auto-imported from CE, aob '{}'; VERIFY live)", desc, aob_sym);
        if !notes.is_empty() {
            full_desc.push_str(&format!(" [{}]", notes.join("; ")));
        }

        cheats.push(CheatConfig {
            id: next_id,
            name: desc.clone(),
            description: full_desc,
            hotkey: hotkey_cfg,
            cheat_type: CheatTypeConfig::Nop {
                signature: pattern.clone(),
                mask: Some(mask),
                length,
            },
        });
        report.imported.push(ImportedCheat {
            id: next_id,
            name: desc,
            signature: pattern,
            length,
            notes,
        });
        next_id += 1;
    }

    let config = GameConfig {
        metadata: GameMetadata {
            name: name.clone(),
            version: "imported".to_string(),
            author: "CT import".to_string(),
            description: format!("Auto-imported from {}", path.display()),
            executable_names: vec![exe.clone()],
            steam_app_id: None,
        },
        process: ProcessConfig {
            names: vec![exe.clone()],
            case_insensitive: true,
            require_wine: true,
            module_name: Some(exe),
        },
        cheats,
        settings: GameSettings::default(),
    };
    Ok((config, report))
}

fn child_text(entry: &roxmltree::Node, tag: &str) -> Option<String> {
    entry
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == tag)
        .and_then(|n| n.text())
        .map(|s| s.to_string())
}

/// Parse "F3 0F 11 4A D4" into bytes; None if any token is not hex.
fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for tok in s.split_whitespace() {
        if tok.len() != 2 {
            return None;
        }
        out.push(u8::from_str_radix(tok, 16).ok()?);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Normalize a CE pattern: any token containing '?' or 'x' becomes "??",
/// everything else uppercased as-is.
fn normalize_pattern(s: &str) -> String {
    s.split_whitespace()
        .map(|tok| {
            if tok.contains('?') || tok.contains('x') || tok.contains('X') {
                "??".to_string()
            } else {
                tok.to_ascii_uppercase()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
