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
    KeyA = 0x41, KeyB = 0x42, KeyC = 0x43, KeyD = 0x44, KeyE = 0x45,
    KeyF = 0x46, KeyG = 0x47, KeyH = 0x48, KeyI = 0x49, KeyJ = 0x4A,
    KeyK = 0x4B, KeyL = 0x4C, KeyM = 0x4D, KeyN = 0x4E, KeyO = 0x4F,
    KeyP = 0x50, KeyQ = 0x51, KeyR = 0x52, KeyS = 0x53, KeyT = 0x54,
    KeyU = 0x55, KeyV = 0x56, KeyW = 0x57, KeyX = 0x58, KeyY = 0x59,
    KeyZ = 0x5A,
    Digit0 = 0x30, Digit1 = 0x31, Digit2 = 0x32, Digit3 = 0x33,
    Digit4 = 0x34, Digit5 = 0x35, Digit6 = 0x36, Digit7 = 0x37,
    Digit8 = 0x38, Digit9 = 0x39,
}

impl VirtualKey {
    /// Build from a Windows VK decimal code (as stored in CE <Key> elements).
    /// Returns None for pure modifiers (Shift/Ctrl/Alt), which belong in
    /// `modifiers` instead — use `is_modifier_code` to split them.
    pub fn from_vk_code(code: u16) -> Option<Self> {
        Some(match code {
            0x60 => VirtualKey::Numpad0, 0x61 => VirtualKey::Numpad1,
            0x62 => VirtualKey::Numpad2, 0x63 => VirtualKey::Numpad3,
            0x64 => VirtualKey::Numpad4, 0x65 => VirtualKey::Numpad5,
            0x66 => VirtualKey::Numpad6, 0x67 => VirtualKey::Numpad7,
            0x68 => VirtualKey::Numpad8, 0x69 => VirtualKey::Numpad9,
            0x6A => VirtualKey::Multiply, 0x6B => VirtualKey::Add,
            0x6C => VirtualKey::Separator, 0x6D => VirtualKey::Subtract,
            0x6E => VirtualKey::Decimal, 0x6F => VirtualKey::Divide,
            0x70 => VirtualKey::F1, 0x71 => VirtualKey::F2,
            0x72 => VirtualKey::F3, 0x73 => VirtualKey::F4,
            0x74 => VirtualKey::F5, 0x75 => VirtualKey::F6,
            0x76 => VirtualKey::F7, 0x77 => VirtualKey::F8,
            0x78 => VirtualKey::F9, 0x79 => VirtualKey::F10,
            0x7A => VirtualKey::F11, 0x7B => VirtualKey::F12,
            0x41 => VirtualKey::KeyA, 0x42 => VirtualKey::KeyB,
            0x43 => VirtualKey::KeyC, 0x44 => VirtualKey::KeyD,
            0x45 => VirtualKey::KeyE, 0x46 => VirtualKey::KeyF,
            0x47 => VirtualKey::KeyG, 0x48 => VirtualKey::KeyH,
            0x49 => VirtualKey::KeyI, 0x4A => VirtualKey::KeyJ,
            0x4B => VirtualKey::KeyK, 0x4C => VirtualKey::KeyL,
            0x4D => VirtualKey::KeyM, 0x4E => VirtualKey::KeyN,
            0x4F => VirtualKey::KeyO, 0x50 => VirtualKey::KeyP,
            0x51 => VirtualKey::KeyQ, 0x52 => VirtualKey::KeyR,
            0x53 => VirtualKey::KeyS, 0x54 => VirtualKey::KeyT,
            0x55 => VirtualKey::KeyU, 0x56 => VirtualKey::KeyV,
            0x57 => VirtualKey::KeyW, 0x58 => VirtualKey::KeyX,
            0x59 => VirtualKey::KeyY, 0x5A => VirtualKey::KeyZ,
            0x30 => VirtualKey::Digit0, 0x31 => VirtualKey::Digit1,
            0x32 => VirtualKey::Digit2, 0x33 => VirtualKey::Digit3,
            0x34 => VirtualKey::Digit4, 0x35 => VirtualKey::Digit5,
            0x36 => VirtualKey::Digit6, 0x37 => VirtualKey::Digit7,
            0x38 => VirtualKey::Digit8, 0x39 => VirtualKey::Digit9,
            _ => return None,
        })
    }

    /// CE modifier key codes that map to `ModifierKey` instead of a main key.
    pub fn modifier_for_vk_code(code: u16) -> Option<ModifierKey> {
        match code {
            16 => Some(ModifierKey::Shift),
            17 => Some(ModifierKey::Control),
            18 => Some(ModifierKey::Alt),
            _ => None,
        }
    }

    /// Canonical lowercase config name (round-trips through from_str).
    pub fn config_name(self) -> &'static str {
        match self {
            VirtualKey::Numpad0 => "numpad0", VirtualKey::Numpad1 => "numpad1",
            VirtualKey::Numpad2 => "numpad2", VirtualKey::Numpad3 => "numpad3",
            VirtualKey::Numpad4 => "numpad4", VirtualKey::Numpad5 => "numpad5",
            VirtualKey::Numpad6 => "numpad6", VirtualKey::Numpad7 => "numpad7",
            VirtualKey::Numpad8 => "numpad8", VirtualKey::Numpad9 => "numpad9",
            VirtualKey::Multiply => "multiply", VirtualKey::Add => "add",
            VirtualKey::Separator => "separator", VirtualKey::Subtract => "subtract",
            VirtualKey::Decimal => "decimal", VirtualKey::Divide => "divide",
            VirtualKey::F1 => "f1", VirtualKey::F2 => "f2",
            VirtualKey::F3 => "f3", VirtualKey::F4 => "f4",
            VirtualKey::F5 => "f5", VirtualKey::F6 => "f6",
            VirtualKey::F7 => "f7", VirtualKey::F8 => "f8",
            VirtualKey::F9 => "f9", VirtualKey::F10 => "f10",
            VirtualKey::F11 => "f11", VirtualKey::F12 => "f12",
            VirtualKey::KeyA => "a", VirtualKey::KeyB => "b",
            VirtualKey::KeyC => "c", VirtualKey::KeyD => "d",
            VirtualKey::KeyE => "e", VirtualKey::KeyF => "f",
            VirtualKey::KeyG => "g", VirtualKey::KeyH => "h",
            VirtualKey::KeyI => "i", VirtualKey::KeyJ => "j",
            VirtualKey::KeyK => "k", VirtualKey::KeyL => "l",
            VirtualKey::KeyM => "m", VirtualKey::KeyN => "n",
            VirtualKey::KeyO => "o", VirtualKey::KeyP => "p",
            VirtualKey::KeyQ => "q", VirtualKey::KeyR => "r",
            VirtualKey::KeyS => "s", VirtualKey::KeyT => "t",
            VirtualKey::KeyU => "u", VirtualKey::KeyV => "v",
            VirtualKey::KeyW => "w", VirtualKey::KeyX => "x",
            VirtualKey::KeyY => "y", VirtualKey::KeyZ => "z",
            VirtualKey::Digit0 => "digit0", VirtualKey::Digit1 => "digit1",
            VirtualKey::Digit2 => "digit2", VirtualKey::Digit3 => "digit3",
            VirtualKey::Digit4 => "digit4", VirtualKey::Digit5 => "digit5",
            VirtualKey::Digit6 => "digit6", VirtualKey::Digit7 => "digit7",
            VirtualKey::Digit8 => "digit8", VirtualKey::Digit9 => "digit9",
        }
    }

    /// Short display name for the TUI.
    pub fn display_name(self) -> &'static str {
        match self {
            VirtualKey::Numpad0 => "Num 0", VirtualKey::Numpad1 => "Num 1",
            VirtualKey::Numpad2 => "Num 2", VirtualKey::Numpad3 => "Num 3",
            VirtualKey::Numpad4 => "Num 4", VirtualKey::Numpad5 => "Num 5",
            VirtualKey::Numpad6 => "Num 6", VirtualKey::Numpad7 => "Num 7",
            VirtualKey::Numpad8 => "Num 8", VirtualKey::Numpad9 => "Num 9",
            VirtualKey::Add => "Num +", VirtualKey::Subtract => "Num -",
            VirtualKey::Multiply => "Num *", VirtualKey::Divide => "Num /",
            VirtualKey::Separator => "Num Sep", VirtualKey::Decimal => "Num .",
            VirtualKey::F1 => "F1", VirtualKey::F2 => "F2",
            VirtualKey::F3 => "F3", VirtualKey::F4 => "F4",
            VirtualKey::F5 => "F5", VirtualKey::F6 => "F6",
            VirtualKey::F7 => "F7", VirtualKey::F8 => "F8",
            VirtualKey::F9 => "F9", VirtualKey::F10 => "F10",
            VirtualKey::F11 => "F11", VirtualKey::F12 => "F12",
            VirtualKey::KeyA => "A", VirtualKey::KeyB => "B",
            VirtualKey::KeyC => "C", VirtualKey::KeyD => "D",
            VirtualKey::KeyE => "E", VirtualKey::KeyF => "F",
            VirtualKey::KeyG => "G", VirtualKey::KeyH => "H",
            VirtualKey::KeyI => "I", VirtualKey::KeyJ => "J",
            VirtualKey::KeyK => "K", VirtualKey::KeyL => "L",
            VirtualKey::KeyM => "M", VirtualKey::KeyN => "N",
            VirtualKey::KeyO => "O", VirtualKey::KeyP => "P",
            VirtualKey::KeyQ => "Q", VirtualKey::KeyR => "R",
            VirtualKey::KeyS => "S", VirtualKey::KeyT => "T",
            VirtualKey::KeyU => "U", VirtualKey::KeyV => "V",
            VirtualKey::KeyW => "W", VirtualKey::KeyX => "X",
            VirtualKey::KeyY => "Y", VirtualKey::KeyZ => "Z",
            VirtualKey::Digit0 => "0", VirtualKey::Digit1 => "1",
            VirtualKey::Digit2 => "2", VirtualKey::Digit3 => "3",
            VirtualKey::Digit4 => "4", VirtualKey::Digit5 => "5",
            VirtualKey::Digit6 => "6", VirtualKey::Digit7 => "7",
            VirtualKey::Digit8 => "8", VirtualKey::Digit9 => "9",
        }
    }
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