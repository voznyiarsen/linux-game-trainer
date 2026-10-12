use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use anyhow::{Result, Context};
use crate::cheats::{Cheat, CheatType, Hotkey, VirtualKey, ModifierKey};

/// Complete game configuration loaded from TOML
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GameConfig {
    pub metadata: GameMetadata,
    pub process: ProcessConfig,
    pub cheats: Vec<CheatConfig>,
    #[serde(default)]
    pub settings: GameSettings,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GameMetadata {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub executable_names: Vec<String>,
    #[serde(default)]
    pub steam_app_id: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessConfig {
    pub names: Vec<String>,
    #[serde(default = "default_true")]
    pub case_insensitive: bool,
    #[serde(default)]
    pub require_wine: bool,
    #[serde(default)]
    pub module_name: Option<String>,
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GameSettings {
    #[serde(default = "default_scan_chunk")]
    pub scan_chunk_size: usize,
    #[serde(default = "default_refresh_secs")]
    pub refresh_interval_secs: u64,
    #[serde(default)]
    pub auto_attach: bool,
}

fn default_scan_chunk() -> usize { 65536 }
fn default_refresh_secs() -> u64 { 2 }

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            scan_chunk_size: default_scan_chunk(),
            refresh_interval_secs: default_refresh_secs(),
            auto_attach: false,
        }
    }
}

/// Cheat definition in config (simpler than runtime Cheat)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CheatConfig {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub hotkey: HotkeyConfig,
    #[serde(flatten)]
    pub cheat_type: CheatTypeConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HotkeyConfig {
    pub key: String,
    #[serde(default)]
    pub modifiers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum CheatTypeConfig {
    #[serde(rename = "write_value")]
    WriteValue {
        offset: String,
        value: Vec<u8>,
    },
    #[serde(rename = "nop")]
    Nop {
        signature: String,
        mask: Option<String>,
        length: usize,
    },
    #[serde(rename = "signature_patch")]
    SignaturePatch {
        signature: String,
        mask: Option<String>,
        patch_offset: isize,
        patch_bytes: Vec<u8>,
    },
    #[serde(rename = "pointer_chain")]
    PointerChain {
        base_offset: String,
        offsets: Vec<String>,
        value: Vec<u8>,
    },
    #[serde(rename = "set_symbol")]
    SetSymbol {
        symbol_name: String,
        value: Vec<u8>,
    },
}

impl GameConfig {
    /// Load a single game config from TOML file
    pub fn load_from_file<P: AsRef<Path> + std::fmt::Debug>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read game config: {:?}", path))?;
        let config: GameConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse TOML: {:?}", path))?;
        Ok(config)
    }
    
    /// Load all game configs from a directory
    pub fn load_from_directory<P: AsRef<Path>>(dir: P) -> Result<HashMap<String, Self>> {
        let mut games = HashMap::new();
        let dir = dir.as_ref();
        
        if !dir.exists() {
            return Ok(games);
        }
        
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                let config = Self::load_from_file(&path)?;
                let key = config.metadata.name.clone();
                games.insert(key, config);
            }
        }
        
        Ok(games)
    }
    
    /// Convert to runtime cheats
    pub fn to_runtime_cheats(&self) -> Result<Vec<Cheat>> {
        self.cheats.iter().map(|c| c.to_runtime()).collect()
    }
    
    /// Get default config directory
    pub fn default_config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("trainer")
            .join("games")
    }
}

impl CheatConfig {
    pub fn to_runtime(&self) -> Result<Cheat> {
        let hotkey = self.hotkey.to_runtime()?;
        let cheat_type = self.cheat_type.to_runtime()?;
        Ok(Cheat::new(self.id, &self.name, &self.description, hotkey, cheat_type))
    }
}

impl HotkeyConfig {
    pub fn to_runtime(&self) -> Result<Hotkey> {
        let key = VirtualKey::from_str(&self.key)?;
        let modifiers = self.modifiers.iter()
            .map(|m| ModifierKey::from_str(m))
            .collect::<Result<Vec<_>>>()?;
        Ok(Hotkey { key, modifiers })
    }
}

impl CheatTypeConfig {
    pub fn to_runtime(&self) -> Result<CheatType> {
        match self {
            CheatTypeConfig::WriteValue { offset, value } => {
                let offset = parse_offset(offset)?;
                Ok(CheatType::WriteValue { offset, value: value.clone(), original: None })
            }
            CheatTypeConfig::Nop { signature, mask, length } => {
                let mask = mask.as_ref().map(|m| m.clone()).unwrap_or_else(|| "x".repeat(signature.split_whitespace().count()));
                Ok(CheatType::Nop { signature: signature.clone(), mask, length: *length, original: None })
            }
            CheatTypeConfig::SignaturePatch { signature, mask, patch_offset, patch_bytes } => {
                let mask = mask.as_ref().map(|m| m.clone()).unwrap_or_else(|| "x".repeat(signature.split_whitespace().count()));
                Ok(CheatType::SignaturePatch { 
                    signature: signature.clone(), 
                    mask, 
                    patch_offset: *patch_offset, 
                    patch_bytes: patch_bytes.clone(), 
                    original: None 
                })
            }
            CheatTypeConfig::PointerChain { base_offset, offsets, value } => {
                let base_offset = parse_offset(base_offset)?;
                let offsets = offsets.iter().map(|o| parse_offset(o)).collect::<Result<Vec<_>>>()?;
                Ok(CheatType::PointerChain { base_offset, offsets, value: value.clone() })
            }
            CheatTypeConfig::SetSymbol { symbol_name, value } => {
                Ok(CheatType::SetSymbol { symbol_name: symbol_name.clone(), value: value.clone() })
            }
        }
    }
}

fn parse_offset(s: &str) -> Result<usize> {
    if s.starts_with("0x") || s.starts_with("0X") {
        Ok(usize::from_str_radix(&s[2..], 16)?)
    } else {
        Ok(s.parse()?)
    }
}

impl VirtualKey {
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "numpad0" | "np0" => Ok(VirtualKey::Numpad0),
            "numpad1" | "np1" => Ok(VirtualKey::Numpad1),
            "numpad2" | "np2" => Ok(VirtualKey::Numpad2),
            "numpad3" | "np3" => Ok(VirtualKey::Numpad3),
            "numpad4" | "np4" => Ok(VirtualKey::Numpad4),
            "numpad5" | "np5" => Ok(VirtualKey::Numpad5),
            "numpad6" | "np6" => Ok(VirtualKey::Numpad6),
            "numpad7" | "np7" => Ok(VirtualKey::Numpad7),
            "numpad8" | "np8" => Ok(VirtualKey::Numpad8),
            "numpad9" | "np9" => Ok(VirtualKey::Numpad9),
            "multiply" | "np_multiply" => Ok(VirtualKey::Multiply),
            "add" | "np_add" | "plus" => Ok(VirtualKey::Add),
            "subtract" | "np_subtract" | "minus" => Ok(VirtualKey::Subtract),
            "divide" | "np_divide" => Ok(VirtualKey::Divide),
            "f1" => Ok(VirtualKey::F1),
            "f2" => Ok(VirtualKey::F2),
            "f3" => Ok(VirtualKey::F3),
            "f4" => Ok(VirtualKey::F4),
            "f5" => Ok(VirtualKey::F5),
            "f6" => Ok(VirtualKey::F6),
            "f7" => Ok(VirtualKey::F7),
            "f8" => Ok(VirtualKey::F8),
            "f9" => Ok(VirtualKey::F9),
            "f10" => Ok(VirtualKey::F10),
            "f11" => Ok(VirtualKey::F11),
            "f12" => Ok(VirtualKey::F12),
            "a" => Ok(VirtualKey::KeyA), "b" => Ok(VirtualKey::KeyB),
            "c" => Ok(VirtualKey::KeyC), "d" => Ok(VirtualKey::KeyD),
            "e" => Ok(VirtualKey::KeyE), "f" => Ok(VirtualKey::KeyF),
            "g" => Ok(VirtualKey::KeyG), "h" => Ok(VirtualKey::KeyH),
            "i" => Ok(VirtualKey::KeyI), "j" => Ok(VirtualKey::KeyJ),
            "k" => Ok(VirtualKey::KeyK), "l" => Ok(VirtualKey::KeyL),
            "m" => Ok(VirtualKey::KeyM), "n" => Ok(VirtualKey::KeyN),
            "o" => Ok(VirtualKey::KeyO), "p" => Ok(VirtualKey::KeyP),
            "q" => Ok(VirtualKey::KeyQ), "r" => Ok(VirtualKey::KeyR),
            "s" => Ok(VirtualKey::KeyS), "t" => Ok(VirtualKey::KeyT),
            "u" => Ok(VirtualKey::KeyU), "v" => Ok(VirtualKey::KeyV),
            "w" => Ok(VirtualKey::KeyW), "x" => Ok(VirtualKey::KeyX),
            "y" => Ok(VirtualKey::KeyY), "z" => Ok(VirtualKey::KeyZ),
            "digit0" | "0" => Ok(VirtualKey::Digit0),
            "digit1" | "1" => Ok(VirtualKey::Digit1),
            "digit2" | "2" => Ok(VirtualKey::Digit2),
            "digit3" | "3" => Ok(VirtualKey::Digit3),
            "digit4" | "4" => Ok(VirtualKey::Digit4),
            "digit5" | "5" => Ok(VirtualKey::Digit5),
            "digit6" | "6" => Ok(VirtualKey::Digit6),
            "digit7" | "7" => Ok(VirtualKey::Digit7),
            "digit8" | "8" => Ok(VirtualKey::Digit8),
            "digit9" | "9" => Ok(VirtualKey::Digit9),
            "separator" | "np_separator" => Ok(VirtualKey::Separator),
            "decimal" | "np_decimal" | "np_dot" => Ok(VirtualKey::Decimal),
            _ => anyhow::bail!("Unknown virtual key: {}", s),
        }
    }
}

impl ModifierKey {
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "ctrl" | "control" => Ok(ModifierKey::Control),
            "shift" => Ok(ModifierKey::Shift),
            "alt" => Ok(ModifierKey::Alt),
            _ => anyhow::bail!("Unknown modifier: {}", s),
        }
    }
}

impl GameConfig {
    /// Create a template config for a new game
    pub fn template() -> Self {
        Self {
            metadata: GameMetadata {
                name: "New Game".to_string(),
                version: "1.0.0".to_string(),
                author: "Your Name".to_string(),
                description: "Game trainer".to_string(),
                executable_names: vec!["game.exe".to_string()],
                steam_app_id: None,
            },
            process: ProcessConfig {
                names: vec!["game.exe".to_string()],
                case_insensitive: true,
                require_wine: true,
                module_name: None,
            },
            cheats: vec![
                CheatConfig {
                    id: 1,
                    name: "Example Cheat".to_string(),
                    description: "Enable infinite health".to_string(),
                    hotkey: HotkeyConfig { key: "numpad1".to_string(), modifiers: vec![] },
                    cheat_type: CheatTypeConfig::Nop {
                        signature: "48 89 5C 24 ??".to_string(),
                        mask: None,
                        length: 5,
                    },
                },
            ],
            settings: GameSettings::default(),
        }
    }
}

/// Load all game configs from default and custom directories
pub fn load_game_configs(custom_dir: Option<PathBuf>) -> Result<HashMap<String, GameConfig>> {
    let mut all_games = HashMap::new();
    
    // Default config directory
    if let Some(dir) = dirs::config_dir() {
        let default_dir = dir.join("trainer").join("games");
        if default_dir.exists() {
            let games = GameConfig::load_from_directory(&default_dir)?;
            all_games.extend(games);
        }
    }
    
    // Custom directory (higher priority)
    if let Some(dir) = custom_dir {
        if dir.exists() {
            let games = GameConfig::load_from_directory(&dir)?;
            all_games.extend(games);
        }
    }
    
    // Current directory ./games
    let local_dir = PathBuf::from("games");
    if local_dir.exists() {
        let games = GameConfig::load_from_directory(&local_dir)?;
        all_games.extend(games);
    }
    
    Ok(all_games)
}