//! Trainer Core - Game-agnostic memory manipulation and TUI framework
//!
//! This crate provides the core functionality for building game trainers:
//! - Process attachment via ptrace
//! - Memory read/write via /proc/pid/mem
//! - Signature scanning (IDA-style patterns)
//! - Cheat management (enable/disable, hotkeys)
//! - Terminal UI (TUI)
//! - Game configuration loading (TOML)

pub mod memory;
pub mod scanner;
pub mod process;
pub mod cheats;
pub mod tui;
pub mod config;
pub mod game;
pub mod pe;
pub mod disasm;

pub use memory::ProcessMemory;
pub use scanner::{Signature, SignatureScanner, DriftCandidate};
pub use process::{find_process, ProcessInfo};
pub use cheats::{Cheat, CheatType, CheatManager, Hotkey, VirtualKey, ModifierKey};
pub use tui::{TrainerApp, TrainerEvent};
pub use config::{GameConfig, load_game_configs};
pub use game::{GameModule, GameModuleBuilder};
pub use pe::PeImage;
pub use disasm::{volatile_mask, portable_pattern};

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_signature_parse() {
        let sig = Signature::parse("48 89 5C 24 ?? 48 89 74 24 ?? 57 48 83 EC 20").unwrap();
        assert_eq!(sig.bytes.len(), 15);
        assert_eq!(sig.bytes[0], Some(0x48));
        assert_eq!(sig.bytes[4], None);
    }
    
    #[test]
    fn test_cheat_creation() {
        let cheat = Cheat::new(
            1,
            "Test Cheat",
            "Description",
            Hotkey { key: VirtualKey::Numpad1, modifiers: vec![] },
            CheatType::Nop {
                signature: "90 90".to_string(),
                mask: "xx".to_string(),
                length: 2,
                original: None,
            },
        );
        assert_eq!(cheat.id, 1);
        assert!(!cheat.enabled);
    }
}