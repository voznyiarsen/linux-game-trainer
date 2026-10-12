use trainer_core::{GameModuleBuilder, GameConfig, ProcessMemory, Signature, SignatureScanner, find_process};
use anyhow::{Result, Context, bail};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod ctimport;

#[derive(Parser)]
#[command(name = "trainer", version, about = "Modular game trainer framework")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a game trainer by name
    Run {
        /// Game name (as defined in game config)
        game: String,

        /// Additional config directory
        #[arg(short, long)]
        config_dir: Option<PathBuf>,
    },

    /// List all available game configs
    List {
        /// Additional config directory
        #[arg(short, long)]
        config_dir: Option<PathBuf>,
    },

    /// Validate a game config file
    Validate {
        config_file: PathBuf,
    },

    /// Create a new game config template
    Init {
        /// Output file path
        #[arg(short, long, default_value = "games/new_game.toml")]
        output: PathBuf,
    },

    /// Import a Cheat Engine (.CT) table into a game TOML config.
    /// Extracts aobscan patterns, original bytes and hotkeys; every cheat
    /// is emitted as a `nop` approximation that MUST be validated live.
    ImportCt {
        /// Path to the .CT file
        table: PathBuf,

        /// Output TOML path
        #[arg(short, long)]
        output: PathBuf,

        /// Game name (default: table file stem)
        #[arg(short, long)]
        game: Option<String>,

        /// Executable name fallback (default: guessed from aobscan modules)
        #[arg(short, long)]
        exe: Option<String>,
    },

    /// Apply a single cheat headlessly (for testing/scripting, no TUI).
    /// Scans without freezing, then attaches only for the brief write.
    Apply {
        /// Game name (as defined in game config)
        #[arg(short, long)]
        game: String,

        /// Cheat id from the game config
        #[arg(short, long)]
        cheat: u32,

        /// Enable (patch) or disable (restore)
        #[arg(long, conflicts_with = "disable")]
        enable: bool,

        /// Disable (restore original bytes)
        #[arg(long, conflicts_with = "enable")]
        disable: bool,

        /// Additional config directory
        #[arg(short, long)]
        config_dir: Option<PathBuf>,

        /// Directory to store original-bytes backups (default: /mnt/scratch/trainer_backups)
        #[arg(long)]
        backup_dir: Option<PathBuf>,

        /// If multiple matches found, use the first instead of failing
        #[arg(long)]
        first: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { game, config_dir } => {
            let mut builder = GameModuleBuilder::new();
            if let Some(dir) = config_dir { builder = builder.add_config_dir(dir); }

            let mut module = builder.build_by_name(&game)?;
            println!("Starting {} trainer...", game);
            module.run()?;
        }
        Commands::List { config_dir } => {
            let mut builder = GameModuleBuilder::new();
            if let Some(dir) = config_dir { builder = builder.add_config_dir(dir); }

            let modules = builder.build_all()?;
            if modules.is_empty() {
                println!("No game configs found.");
            } else {
                println!("Available games:");
                for m in modules {
                    println!("  {} v{} - {}", m.config.metadata.name, m.config.metadata.version, m.config.metadata.description);
                }
            }
        }
        Commands::Validate { config_file } => {
            let config = GameConfig::load_from_file(&config_file)?;
            println!("✓ Config valid: {} v{}", config.metadata.name, config.metadata.version);
            println!("  Cheats: {}", config.cheats.len());
            println!("  Process names: {:?}", config.process.names);
        }
        Commands::Init { output } => {
            let template = GameConfig::template();
            std::fs::write(&output, toml::to_string_pretty(&template)?)?;
            println!("✓ Created template at {}", output.display());
        }
        Commands::ImportCt { table, output, game, exe } => {
            let (config, report) = ctimport::import_ct(&table, game, exe)?;
            std::fs::write(&output, toml::to_string_pretty(&config)?)?;
            println!("✓ Wrote {} cheat(s) to {}", report.imported.len(), output.display());
            for c in &report.imported {
                let note = if c.notes.is_empty() { String::new() } else { format!(" [{}]", c.notes.join("; ")) };
                println!("  [{:>3}] {:45} sig='{}' len={}{}", c.id, c.name, c.signature, c.length, note);
            }
            if !report.skipped.is_empty() {
                println!("Skipped {} entr{} (no usable pattern):", report.skipped.len(), if report.skipped.len() == 1 { "y" } else { "ies" });
                for (name, reason) in &report.skipped {
                    println!("  - {}: {}", name, reason);
                }
            }
            println!("NOTE: imports are nop approximations of code-cave scripts.");
            println!("Validate each with `apply --cheat <id> --enable` before trusting it.");
        }
        Commands::Apply { game, cheat, enable, disable, config_dir, backup_dir, first } => {
            let do_enable = if disable { false } else { true };
            let _ = enable; // --enable is the default action
            apply_cheat_headless(&game, cheat, do_enable, config_dir, backup_dir, first)?;
        }
    }

    Ok(())
}

fn apply_cheat_headless(
    game: &str,
    cheat_id: u32,
    do_enable: bool,
    config_dir: Option<PathBuf>,
    backup_dir: Option<PathBuf>,
    use_first: bool,
) -> Result<()> {
    // 1. Load game config
    let mut builder = GameModuleBuilder::new();
    if let Some(dir) = config_dir.clone() { builder = builder.add_config_dir(dir); }
    let module = builder.build_by_name(game)?;
    let config = &module.config;

    let cheat_cfg = config.cheats.iter()
        .find(|c| c.id == cheat_id)
        .with_context(|| format!("Cheat id {} not found in game '{}'", cheat_id, game))?;
    let runtime = cheat_cfg.to_runtime()?;
    println!("Game: {} | Cheat [{}]: {} ({})", game, cheat_id, runtime.name, runtime.description);
    println!("Type: {:?}", cheat_cfg.cheat_type);

    // 2. Find process (no attach yet)
    let procs = find_process(&config.process)?;
    if procs.is_empty() {
        bail!("No running process found for {:?}", config.process.names);
    }
    let pid = procs[0].pid;
    println!("Target PID: {} ({})", pid, procs[0].cmdline);

    // 3. Open without attach for scanning (no freeze)
    let mut pm = ProcessMemory::open(pid)?;
    let module_name = config.process.module_name.as_deref()
        .unwrap_or(&config.process.names[0]);
    let base = pm.find_module_base(module_name)?;
    println!("Module '{}' base: 0x{:x}", module_name, base);

    // 4. Resolve cheat address.
    // On disable, prefer the address stored in an existing backup file: the
    // patch itself destroys the signature, so rescanning would find 0 hits.
    use trainer_core::cheats::CheatType;
    let cheat_type = runtime.cheat_type.clone();
    let backup_dir = backup_dir.unwrap_or_else(|| PathBuf::from("/mnt/scratch/trainer_backups"));
    std::fs::create_dir_all(&backup_dir)?;
    let safe_game: String = game.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
    let backup_prefix = format!("{}_{}_", safe_game, cheat_id);
    let existing_backups: Vec<PathBuf> = std::fs::read_dir(&backup_dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|x| x.path())).filter(|p| {
            p.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with(&backup_prefix) && n.ends_with(".bin")).unwrap_or(false)
        }).collect())
        .unwrap_or_default();
    let (addr, len, backup_path) = match &cheat_type {
        CheatType::Nop { signature, length, .. } => {
            if signature.is_empty() { bail!("Nop cheat has empty signature"); }
            let sig = Signature::parse(signature)?;
            let mut scanner = SignatureScanner::new(&mut pm);
            let mut hits = scanner.scan_game_code(&sig)?;
            println!("Scan '{}': {} hit(s)", signature, hits.len());
            for h in &hits { println!("  0x{:x} ({}+0x{:X})", h, module_name, h - base); }
            if hits.is_empty() {
                // Likely already patched (signature destroyed). Fall back to backup address.
                if !do_enable && existing_backups.len() == 1 {
                    let p = &existing_backups[0];
                    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                    let addr_hex = stem.rsplit('_').next().unwrap_or("");
                    let addr = usize::from_str_radix(addr_hex, 16)
                        .with_context(|| format!("Cannot parse address from backup {:?}", p))?;
                    println!("Signature not found (probably already patched); using backup address 0x{:x} from {}", addr, p.display());
                    (addr, *length, p.clone())
                } else {
                    bail!("Signature not found (0 hits){}", if !do_enable && !existing_backups.is_empty() { "; multiple backups exist, specify address" } else { "" });
                }
            } else {
                if hits.len() > 1 && !use_first {
                    bail!("Signature not unique ({} hits); re-run with --first to use first", hits.len());
                }
                let addr = hits[0];
                (addr, *length, backup_dir.join(format!("{}_{}_{:x}.bin", safe_game, cheat_id, addr)))
            }
        }
        CheatType::SignaturePatch { signature, patch_offset, patch_bytes, .. } => {
            if signature.is_empty() { bail!("SignaturePatch has empty signature"); }
            if patch_bytes.is_empty() { bail!("SignaturePatch has empty patch_bytes (needs code-cave injection, not supported headlessly)"); }
            let sig = Signature::parse(signature)?;
            let mut scanner = SignatureScanner::new(&mut pm);
            let mut hits = scanner.scan_game_code(&sig)?;
            println!("Scan '{}': {} hit(s)", signature, hits.len());
            for h in &hits { println!("  0x{:x} ({}+0x{:X})", h, module_name, h - base); }
            if hits.is_empty() { bail!("Signature not found"); }
            if hits.len() > 1 && !use_first {
                bail!("Signature not unique ({} hits); re-run with --first to use first", hits.len());
            }
            let addr = ((hits[0] as isize + *patch_offset) as usize);
            (addr, patch_bytes.len(), backup_dir.join(format!("{}_{}_{:x}.bin", safe_game, cheat_id, addr)))
        }
        CheatType::WriteValue { offset, value, .. } => {
            let addr = base + *offset;
            (addr, value.len(), backup_dir.join(format!("{}_{}_{:x}.bin", safe_game, cheat_id, addr)))
        }
        other => bail!("Headless apply not supported for {:?}", std::mem::discriminant(other)),
    };
    println!("Resolved address: 0x{:x} (len {})", addr, len);
    drop(pm); // close mem handle before attach

    // 5. Backup path (resolved above; ensure dir exists)
    std::fs::create_dir_all(&backup_dir)?;
    println!("Backup file: {}", backup_path.display());

    // 6. Attach briefly for read/write, then detach
    println!("Attaching (freezing PID {})...", pid);
    let mut pm = ProcessMemory::attach(pid)?;
    // Re-resolve base after attach (regions may have changed, but base should be same)
    let base2 = pm.find_module_base(module_name).unwrap_or(base);
    if base2 != base {
        println!("Note: base changed after attach 0x{:x} -> 0x{:x}", base, base2);
    }

    let mut cur = vec![0u8; len];
    pm.read_memory(addr, &mut cur)?;
    println!("Current bytes @ 0x{:x}: {}", addr, hex(&cur));

    if do_enable {
        // Save original if no backup yet
        if !backup_path.exists() {
            std::fs::write(&backup_path, &cur)?;
            println!("Saved original bytes to backup");
        } else {
            let saved = std::fs::read(&backup_path)?;
            if saved != cur && cur.iter().all(|&b| b == 0x90) {
                println!("Already patched (current bytes are NOPs, backup exists). Verifying...");
            } else if saved != cur {
                println!("Warning: current bytes differ from backup; overwriting backup with current");
                std::fs::write(&backup_path, &cur)?;
            }
        }
        let patch: Vec<u8> = match &cheat_type {
            CheatType::Nop { .. } => vec![0x90; len],
            CheatType::SignaturePatch { patch_bytes, .. } => patch_bytes.clone(),
            CheatType::WriteValue { value, .. } => value.clone(),
            _ => unreachable!(),
        };
        println!("Writing patch bytes: {}", hex(&patch));
        pm.write_memory(addr, &patch)?;
        let mut verify = vec![0u8; len];
        pm.read_memory(addr, &mut verify)?;
        println!("Verify read-back: {}", hex(&verify));
        if verify != patch {
            bail!("Verification FAILED: read-back does not match patch");
        }
        println!("✓ ENABLED: {} is now active ({}+0x{:X} NOPed/patched)", runtime.name, module_name, addr - base2);
    } else {
        if !backup_path.exists() {
            bail!("No backup found at {}; cannot restore", backup_path.display());
        }
        let orig = std::fs::read(&backup_path)?;
        if orig.len() != len {
            bail!("Backup length {} != expected {}", orig.len(), len);
        }
        println!("Restoring original bytes: {}", hex(&orig));
        pm.write_memory(addr, &orig)?;
        let mut verify = vec![0u8; len];
        pm.read_memory(addr, &mut verify)?;
        println!("Verify read-back: {}", hex(&verify));
        if verify != orig {
            bail!("Verification FAILED: read-back does not match original");
        }
        println!("✓ DISABLED: {} restored", runtime.name);
    }

    // Detach (resume game)
    pm.detach()?;
    println!("Detached (game resumed).");

    // Confirm game still alive
    if std::path::Path::new(&format!("/proc/{}", pid)).exists() {
        println!("Game process {} still alive ✓", pid);
    } else {
        println!("Warning: game process {} gone!", pid);
    }
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect::<Vec<_>>().join(" ")
}
