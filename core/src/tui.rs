use crate::cheats::{CheatManager, CheatType, Hotkey, ModifierKey};
use crate::memory::ProcessMemory;
use crate::process::find_process;
use crate::scanner::{Signature, SignatureScanner};
use crate::config::GameConfig;
use anyhow::{Result, Context};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    cursor::{Hide, Show},
    style::{Color, ResetColor, SetForegroundColor},
};
use std::io::{stdout, Write};
use std::time::{Duration, Instant};

/// Trainer application events
#[derive(Debug, Clone)]
pub enum TrainerEvent {
    Quit,
    ToggleCheat(u32),
    ScanCheat(u32),
    AttachProcess,
    RefreshProcess,
    MoveUp,
    MoveDown,
}

/// Main trainer application state
pub struct TrainerApp {
    game_config: GameConfig,
    cheat_manager: CheatManager,
    process: Option<ProcessMemory>,
    pid: Option<i32>,
    running: bool,
    status_message: String,
    selected_index: usize,
    last_scan_results: Vec<usize>,
}

impl TrainerApp {
    pub fn new(game_config: GameConfig) -> Result<Self> {
        let cheats = game_config.to_runtime_cheats()?;
        let welcome_msg = format!("Welcome to {} Trainer", game_config.metadata.name);
        Ok(Self {
            game_config,
            cheat_manager: CheatManager::new(cheats),
            process: None,
            pid: None,
            running: true,
            status_message: welcome_msg,
            selected_index: 0,
            last_scan_results: Vec::new(),
        })
    }
    
    /// Initialize and attach to game process
    pub fn initialize(&mut self) -> Result<()> {
        self.set_status("Searching for game process...");
        
        let pids = find_process(&self.game_config.process)?;
        
        if pids.is_empty() {
            self.set_status(&format!("No {} process found. Make sure the game is running.", self.game_config.metadata.name));
            return Ok(());
        }
        
        let pid = pids[0].pid;
        self.set_status(&format!("Attaching to PID {}...", pid));
        
        let mut process = ProcessMemory::attach(pid)?;
        self.set_status("Finding game module base...");
        
        let module_name = self.game_config.process.module_name
            .as_deref()
            .unwrap_or(&self.game_config.process.names[0]);
        let base = process.find_module_base(module_name)
            .context("Failed to find game module base address")?;
        
        self.set_status(&format!("Found module at 0x{:x}", base));
        
        self.process = Some(process);
        self.pid = Some(pid);
        
        Ok(())
    }
    
    /// Run the main TUI loop
    pub fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen, Hide)?;
        
        let _ = self.initialize();
        
        let mut last_tick = Instant::now();
        
        while self.running {
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key)?;
                }
            }
            
            if last_tick.elapsed() > Duration::from_secs(self.game_config.settings.refresh_interval_secs) {
                self.refresh_process();
                last_tick = Instant::now();
            }
            
            self.render()?;
            std::thread::sleep(Duration::from_millis(16));
        }
        
        execute!(stdout(), LeaveAlternateScreen, Show)?;
        disable_raw_mode()?;
        Ok(())
    }
    
    fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            KeyCode::Char('r') => self.refresh_process(),
            KeyCode::Char('a') => self.attach_to_process()?,
            KeyCode::Up => { if self.selected_index > 0 { self.selected_index -= 1; } },
            KeyCode::Down => {
                let max = self.cheat_manager.all_cheats().len().saturating_sub(1);
                if self.selected_index < max { self.selected_index += 1; }
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.toggle_selected_cheat()?,
            KeyCode::Char('s') => self.scan_selected_cheat()?,
            _ => {}
        }
        Ok(())
    }
    
    fn refresh_process(&mut self) {
        if let Some(ref mut process) = self.process {
            if let Err(e) = process.refresh_regions() {
                self.set_status(&format!("Failed to refresh regions: {}", e));
            }
        }
    }
    
    fn attach_to_process(&mut self) -> Result<()> {
        self.set_status("Searching for game process...");
        let pids = find_process(&self.game_config.process)?;
        
        if pids.is_empty() {
            self.set_status("No game process found");
            return Ok(());
        }
        
        if let Some(ref mut process) = self.process { let _ = process.detach(); }
        
        let pid = pids[0].pid;
        let mut process = ProcessMemory::attach(pid)?;
        let module_name = self.game_config.process.module_name
            .as_deref()
            .unwrap_or(&self.game_config.process.names[0]);
        let base = process.find_module_base(module_name)?;
        
        self.process = Some(process);
        self.pid = Some(pid);
        self.set_status(&format!("Attached to PID {} (base: 0x{:x})", pid, base));
        
        Ok(())
    }
    
    fn toggle_selected_cheat(&mut self) -> Result<()> {
        let cheat_id = self.cheat_manager.all_cheats()
            .get(self.selected_index)
            .map(|c| c.id);
        
        if let Some(id) = cheat_id {
            let cheat_name = self.cheat_manager.get_cheat(id).map(|c| c.name.clone());
            let new_state = self.cheat_manager.toggle_cheat(id);
            
            if new_state {
                self.apply_cheat(id, true)?;
                if let Some(name) = cheat_name { self.set_status(&format!("Enabled: {}", name)); }
            } else {
                self.apply_cheat(id, false)?;
                if let Some(name) = cheat_name { self.set_status(&format!("Disabled: {}", name)); }
            }
        }
        Ok(())
    }
    
    fn scan_selected_cheat(&mut self) -> Result<()> {
        let (cheat_name, cheat_type) = {
            let cheats = self.cheat_manager.all_cheats();
            if let Some(cheat) = cheats.get(self.selected_index) {
                (cheat.name.clone(), cheat.cheat_type.clone())
            } else { return Ok(()); }
        };
        
        self.set_status(&format!("Scanning for {}...", cheat_name));
        
        if let Some(ref mut process) = self.process {
            let mut scanner = SignatureScanner::new(process);
            
            match &cheat_type {
                CheatType::SignaturePatch { signature, .. } |
                CheatType::Nop { signature, .. } => {
                    if !signature.is_empty() {
                        let sig = Signature::parse(signature)?;
                        let results = scanner.scan_game_code(&sig)?;
                        self.last_scan_results = results.clone();
                        
                        if results.is_empty() {
                            self.set_status("Signature not found in memory");
                        } else if results.len() == 1 {
                            self.set_status(&format!("Found unique match at 0x{:x}", results[0]));
                        } else {
                            self.set_status(&format!("Found {} matches (need unique)", results.len()));
                        }
                    } else {
                        self.set_status("No signature defined for this cheat");
                    }
                }
                _ => self.set_status("Scan not supported for this cheat type"),
            }
        } else {
            self.set_status("Not attached to any process");
        }
        Ok(())
    }
    
    fn apply_cheat(&mut self, cheat_id: u32, enable: bool) -> Result<()> {
        let Some(ref mut process) = self.process else {
            self.set_status("Not attached to process");
            return Ok(());
        };
        
        let Some(cheat_type) = self.cheat_manager.get_cheat(cheat_id).map(|c| c.cheat_type.clone()) else {
            return Ok(());
        };
        
        let mut status_msg = None;
        
        let result = match cheat_type {
            CheatType::WriteValue { offset, value, original } => {
                let base = process.find_module_base(
                    self.game_config.process.module_name.as_deref()
                    .unwrap_or(&self.game_config.process.names[0])
                )?;
                let addr = base + offset;
                
                if enable {
                    if original.is_none() {
                        let mut orig = vec![0u8; value.len()];
                        process.read_memory(addr, &mut orig)?;
                        process.write_memory(addr, &value)?;
                        Some((CheatUpdate::WriteValue { offset, value, original: Some(orig) }, "Enabled"))
                    } else {
                        process.write_memory(addr, &value)?;
                        None
                    }
                } else if let Some(orig) = original {
                    process.write_memory(addr, &orig)?;
                    None
                } else { None }
            }
            CheatType::Nop { signature, mask, length, original } => {
                if signature.is_empty() {
                    status_msg = Some("Nop cheat: no signature defined".to_string());
                    None
                } else {
                    let sig = Signature::parse(&signature)?;
                    let mut scanner = SignatureScanner::new(process);
                    let matches = scanner.scan_game_code(&sig)?;
                    
                    if matches.is_empty() {
                        status_msg = Some("Nop signature not found".to_string());
                        None
                    } else {
                        if matches.len() > 1 {
                            status_msg = Some(format!("Nop: multiple matches ({}), using first", matches.len()));
                        }
                        
                        let addr = matches[0];
                        
                        if enable {
                            if original.is_none() {
                                let mut orig = vec![0u8; length];
                                process.read_memory(addr, &mut orig)?;
                                let nops = vec![0x90; length];
                                process.write_memory(addr, &nops)?;
                                Some((CheatUpdate::Nop { signature: signature.clone(), mask: mask.clone(), length, original: Some(orig) }, "Enabled"))
                            } else {
                                let nops = vec![0x90; length];
                                process.write_memory(addr, &nops)?;
                                None
                            }
                        } else if let Some(orig) = original {
                            process.write_memory(addr, &orig)?;
                            None
                        } else { None }
                    }
                }
            }
            CheatType::SignaturePatch { signature, mask: _, patch_offset, patch_bytes, original } => {
                if signature.is_empty() { return Ok(()); }
                
                let sig = Signature::parse(&signature)?;
                let mut scanner = SignatureScanner::new(process);
                let matches = scanner.scan_game_code(&sig)?;
                
                if matches.is_empty() { status_msg = Some("Signature not found".to_string()); None }
                else {
                    if matches.len() > 1 { status_msg = Some(format!("Multiple matches ({}), using first", matches.len())); }
                    
                    let base_addr = (matches[0] as isize + patch_offset) as usize;
                    
                    if enable {
                        if original.is_none() {
                            let mut orig = vec![0u8; patch_bytes.len()];
                            process.read_memory(base_addr, &mut orig)?;
                            process.write_memory(base_addr, &patch_bytes)?;
                            Some((CheatUpdate::SignaturePatch { signature, patch_offset, patch_bytes, original: Some(orig) }, "Enabled"))
                        } else {
                            process.write_memory(base_addr, &patch_bytes)?;
                            None
                        }
                    } else if let Some(orig) = original {
                        process.write_memory(base_addr, &orig)?;
                        None
                    } else { None }
                }
            }
            CheatType::PointerChain { .. } => { status_msg = Some("Pointer chain cheats not yet implemented".to_string()); None }
            CheatType::SetSymbol { .. } => { status_msg = Some("SetSymbol not fully implemented".to_string()); None }
        };
        
        if let Some((update, action)) = result {
            self.apply_cheat_update(cheat_id, update);
            let name = self.cheat_manager.get_cheat(cheat_id).map(|c| c.name.clone()).unwrap_or_default();
            self.set_status(&format!("{}: {}", action, name));
        }
        
        if let Some(msg) = status_msg { self.set_status(&msg); }
        Ok(())
    }
    
    fn apply_cheat_update(&mut self, cheat_id: u32, update: CheatUpdate) {
        if let Some(cheat) = self.cheat_manager.get_cheat_mut(cheat_id) {
            cheat.cheat_type = match update {
                CheatUpdate::WriteValue { offset, value, original } => 
                    CheatType::WriteValue { offset, value, original },
                CheatUpdate::Nop { signature, mask, length, original } => 
                    CheatType::Nop { signature, mask, length, original },
                CheatUpdate::SignaturePatch { signature, patch_offset, patch_bytes, original } => 
                    CheatType::SignaturePatch { 
                        signature, 
                        mask: String::new(), 
                        patch_offset, 
                        patch_bytes, 
                        original 
                    },
            };
        }
    }
    
    fn set_status(&mut self, msg: &str) { self.status_message = msg.to_string(); }
    
    fn render(&self) -> Result<()> {
        let mut out = stdout();
        execute!(out, crossterm::cursor::MoveTo(0, 0))?;
        
        execute!(out, SetForegroundColor(Color::Cyan))?;
        println!("╔═══════════════════════════════════════════════════════════════════════════════╗");
        println!("║                    {} - Linux Trainer", self.game_config.metadata.name);
        println!("╠═══════════════════════════════════════════════════════════════════════════════╣");
        execute!(out, ResetColor)?;
        
        if let Some(pid) = self.pid {
            execute!(out, SetForegroundColor(Color::Green))?;
            println!("║ Process: PID {}  │  Status: Attached", pid);
        } else {
            execute!(out, SetForegroundColor(Color::Red))?;
            println!("║ Process: Not attached  │  Press 'a' to attach");
        }
        execute!(out, ResetColor)?;
        
        println!("╠══════════════════════════════════════════════════════════════════════════════╣");
        
        let cheats = self.cheat_manager.all_cheats();
        for (i, cheat) in cheats.iter().enumerate() {
            let is_selected = i == self.selected_index;
            if is_selected { execute!(out, SetForegroundColor(Color::Yellow))?; print!("► "); } else { print!("  "); }
            let hk = format!("{}", cheat.hotkey);
            let marker = if cheat.enabled { "✓" } else { " " };
            println!("  [{}] {:<40} │ Hotkey: {:<20} │ {}", marker, cheat.name, hk, cheat.description);
            if is_selected { execute!(out, ResetColor)?; }
        }
        for _ in cheats.len()..23 { println!(); }
        
        println!("╠══════════════════════════════════════════════════════════════════════════════╣");
        execute!(out, SetForegroundColor(Color::Cyan))?; println!("║ {}", self.status_message); execute!(out, ResetColor)?;
        println!("╠══════════════════════════════════════════════════════════════════════════════╣");
        println!("║ Controls: ↑/↓ Navigate  │  Enter/Space Toggle  │  s Scan  │  a Attach  │  r Refresh  │  q Quit  ║");
        println!("╚══════════════════════════════════════════════════════════════════════════════╝");
        out.flush()?;
        Ok(())
    }
}

#[derive(Debug)]
enum CheatUpdate {
    WriteValue { offset: usize, value: Vec<u8>, original: Option<Vec<u8>> },
    Nop { signature: String, mask: String, length: usize, original: Option<Vec<u8>> },
    SignaturePatch { signature: String, patch_offset: isize, patch_bytes: Vec<u8>, original: Option<Vec<u8>> },
}

impl std::fmt::Display for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mods: Vec<&str> = self.modifiers.iter().map(|m| match m {
            ModifierKey::Control => "Ctrl", ModifierKey::Shift => "Shift", ModifierKey::Alt => "Alt",
        }).collect();
        let key_name = self.key.display_name();
        if mods.is_empty() { write!(f, "{}", key_name) } else { write!(f, "+{}+{}", mods.join("+"), key_name) }
    }
}