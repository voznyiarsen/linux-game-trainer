use crate::config::GameConfig;
use crate::tui::TrainerApp;
use anyhow::Result;

/// A loaded game module ready to run
pub struct GameModule {
    pub config: GameConfig,
    pub app: TrainerApp,
}

impl GameModule {
    pub fn new(config: GameConfig) -> Result<Self> {
        let app = TrainerApp::new(config.clone())?;
        Ok(Self { config, app })
    }
    
    pub fn run(&mut self) -> Result<()> {
        self.app.run()
    }
}

/// Builder for creating game modules from various sources
pub struct GameModuleBuilder {
    config_dirs: Vec<std::path::PathBuf>,
}

impl GameModuleBuilder {
    pub fn new() -> Self { Self { config_dirs: Vec::new() } }
    
    pub fn add_config_dir<P: Into<std::path::PathBuf>>(mut self, dir: P) -> Self {
        self.config_dirs.push(dir.into());
        self
    }
    
    pub fn build_all(self) -> Result<Vec<GameModule>> {
        let mut all_configs = std::collections::HashMap::new();
        
        for dir in &self.config_dirs {
            if dir.exists() {
                let configs = crate::config::GameConfig::load_from_directory(dir)?;
                all_configs.extend(configs);
            }
        }
        
        // Also check default locations
        if let Some(config_dir) = dirs::config_dir() {
            let default = config_dir.join("trainer").join("games");
            if default.exists() {
                let configs = crate::config::GameConfig::load_from_directory(&default)?;
                all_configs.extend(configs);
            }
        }
        
        let local = std::path::PathBuf::from("games");
        if local.exists() {
            let configs = crate::config::GameConfig::load_from_directory(&local)?;
            all_configs.extend(configs);
        }
        
        all_configs.into_values()
            .map(GameModule::new)
            .collect()
    }
    
    pub fn build_by_name(self, name: &str) -> Result<GameModule> {
        let modules = self.build_all()?;
        modules.into_iter()
            .find(|m| m.config.metadata.name == name)
            .ok_or_else(|| anyhow::anyhow!("Game not found: {}", name))
    }
}

impl Default for GameModuleBuilder {
    fn default() -> Self { Self::new() }
}