use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Runtime cheat definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cheat {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub hotkey: Hotkey,
    pub cheat_type: CheatType,
    pub enabled: bool,
}

/// Hotkey definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hotkey {
    pub key: VirtualKey,
    pub modifiers: Vec<ModifierKey>,
}

/// Virtual key codes (Windows VK_ constants)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
pub enum VirtualKey {
    Numpad0 = 0x60, Numpad1 = 0x61, Numpad2 = 0x62, Numpad3 = 0x63,
    Numpad4 = 0x64, Numpad5 = 0x65, Numpad6 = 0x66, Numpad7 = 0x67,
    Numpad8 = 0x68, Numpad9 = 0x69,
    Multiply = 0x6A, Add = 0x6B, Separator = 0x6C, Subtract = 0x6D,
    Decimal = 0x6E, Divide = 0x6F,
    F1 = 0x70, F2 = 0x71, F3 = 0x72, F4 = 0x73, F5 = 0x74, F6 = 0x75,
    F7 = 0x76, F8 = 0x77, F9 = 0x78, F10 = 0x79, F11 = 0x7A, F12 = 0x7B,
}

/// Modifier keys
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModifierKey { Control = 0x0001, Shift = 0x0002, Alt = 0x0004 }

/// Type of cheat implementation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CheatType {
    WriteValue { offset: usize, value: Vec<u8>, original: Option<Vec<u8>> },
    Nop { signature: String, mask: String, length: usize, original: Option<Vec<u8>> },
    SignaturePatch { signature: String, mask: String, patch_offset: isize, patch_bytes: Vec<u8>, original: Option<Vec<u8>> },
    PointerChain { base_offset: usize, offsets: Vec<usize>, value: Vec<u8> },
    SetSymbol { symbol_name: String, value: Vec<u8> },
}

impl Cheat {
    pub fn new(id: u32, name: &str, description: &str, hotkey: Hotkey, cheat_type: CheatType) -> Self {
        Self { id, name: name.to_string(), description: description.to_string(), hotkey, cheat_type, enabled: false }
    }
    
    pub fn toggle(&mut self) { self.enabled = !self.enabled; }
}

/// Cheat manager for handling activation/deactivation
pub struct CheatManager {
    cheats: HashMap<u32, Cheat>,
}

impl CheatManager {
    pub fn new(cheats: Vec<Cheat>) -> Self {
        let cheats = cheats.into_iter().map(|c| (c.id, c)).collect();
        Self { cheats }
    }
    
    pub fn get_cheat(&self, id: u32) -> Option<&Cheat> { self.cheats.get(&id) }
    pub fn get_cheat_mut(&mut self, id: u32) -> Option<&mut Cheat> { self.cheats.get_mut(&id) }
    pub fn all_cheats(&self) -> Vec<&Cheat> { self.cheats.values().collect() }
    pub fn toggle_cheat(&mut self, id: u32) -> bool {
        if let Some(cheat) = self.cheats.get_mut(&id) { cheat.toggle(); cheat.enabled } else { false }
    }
    pub fn find_by_hotkey(&self, key: VirtualKey, modifiers: &[ModifierKey]) -> Option<u32> {
        self.cheats.iter().find(|(_, c)| c.hotkey.key == key && c.hotkey.modifiers == modifiers).map(|(id, _)| *id)
    }
    pub fn enabled_cheats(&self) -> Vec<&Cheat> { self.cheats.values().filter(|c| c.enabled).collect() }
}

impl Default for CheatManager {
    fn default() -> Self { Self::new(Vec::new()) }
}