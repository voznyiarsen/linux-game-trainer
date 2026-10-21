use trainer_core::{GameModuleBuilder, GameConfig, ProcessMemory, Signature, SignatureScanner, find_process};
use trainer_core::pe::PeImage;
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

    /// Validate every signature in a game config against the live game
    /// (or the exe on disk with --offline). Reports OK / DRIFTED / AMBIGUOUS
    /// per cheat and suggests a repaired pattern for drifted ones.
    Scan {
        /// Game name (as defined in game config)
        #[arg(short, long)]
        game: String,

        /// Additional config directory
        #[arg(short, long)]
        config_dir: Option<PathBuf>,

        /// Scan the executable on disk instead of live process memory
        #[arg(long)]
        offline: bool,

        /// Path to the executable (offline mode). Default: auto-detect.
        #[arg(long)]
        exe: Option<PathBuf>,

        /// Max byte positions to relax when hunting a drifted pattern
        #[arg(long, default_value_t = 2)]
        max_relax: usize,

        /// Also suggest a relocation-proof pattern (masks volatile bytes)
        #[arg(long)]
        suggest: bool,

        /// Write suggested repairs into the game config (in place)
        #[arg(long)]
        repair: bool,
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
        Commands::Scan { game, config_dir, offline, exe, max_relax, suggest, repair } => {
            let code = scan_game_signatures(&game, config_dir, offline, exe, max_relax, suggest, repair)?;
            if code != 0 {
                std::process::exit(code);
            }
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

/// Status of a single signature check.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SigStatus {
    Ok,
    Ambiguous,
    Missing,
    NoPattern,
}

impl SigStatus {
    fn label(self) -> &'static str {
        match self {
            SigStatus::Ok => "OK",
            SigStatus::Ambiguous => "AMBIGUOUS",
            SigStatus::Missing => "MISSING",
            SigStatus::NoPattern => "NO-PATTERN",
        }
    }
}

/// What we are scanning: live process memory or the exe on disk.
enum ScanTarget {
    Live { pid: i32, base: usize, pm: ProcessMemory },
    File { pe: PeImage, path: std::path::PathBuf },
}

impl ScanTarget {
    fn describe(&self) -> String {
        match self {
            ScanTarget::Live { pid, base, .. } => {
                format!("live (PID {}) — base 0x{:x}", pid, base)
            }
            ScanTarget::File { path, pe } => {
                format!("offline (file scan) — {} (ImageBase 0x{:x}, {} sections)",
                    path.display(), pe.image_base, pe.sections.len())
            }
        }
    }
}

/// Abstraction over live memory / file scanning for byte reads.
trait TargetScan {
    fn read_at(&mut self, addr: usize, len: usize) -> Option<Vec<u8>>;
    fn ip_for(&mut self, addr: usize) -> u64;
}

impl TargetScan for ScanTarget {
    fn read_at(&mut self, addr: usize, len: usize) -> Option<Vec<u8>> {
        match self {
            ScanTarget::Live { pm, .. } => {
                let mut buf = vec![0u8; len];
                pm.read_memory(addr, &mut buf).ok()?;
                Some(buf)
            }
            ScanTarget::File { pe, .. } => {
                if addr + len > pe.data.len() {
                    return None;
                }
                Some(pe.data[addr..addr + len].to_vec())
            }
        }
    }

    fn ip_for(&mut self, addr: usize) -> u64 {
        match self {
            ScanTarget::Live { .. } => addr as u64,
            ScanTarget::File { pe, .. } => pe
                .file_offset_to_rva(addr)
                .map(|rva| pe.image_base + rva as u64)
                .unwrap_or(addr as u64),
        }
    }
}

/// Validate every signature of a game config; optionally suggest repairs.
///
/// Exit code: 0 = all good, 1 = problems found (CI friendly).
fn scan_game_signatures(
    game: &str,
    config_dir: Option<PathBuf>,
    offline: bool,
    exe_path: Option<PathBuf>,
    max_relax: usize,
    suggest: bool,
    repair: bool,
) -> Result<i32> {
    use trainer_core::config::CheatTypeConfig;
    use trainer_core::pe::PeImage;
    use trainer_core::scanner::{find_drifted_in_file, scan_pe_file};

    // 1. Load config
    let mut builder = GameModuleBuilder::new();
    if let Some(dir) = config_dir.clone() { builder = builder.add_config_dir(dir); }
    let module = builder.build_by_name(game)?;
    let config = module.config.clone();
    let module_name = config.process.module_name.clone()
        .unwrap_or_else(|| config.process.names[0].clone());

    println!("Game: {}", config.metadata.name);
    println!("Module: {}", module_name);

    // 2. Set up the scan target
    let mut target = if offline {
        let exe = match exe_path {
            Some(p) => p,
            None => find_game_exe(&config)?,
        };
        ScanTarget::File { pe: PeImage::load(&exe)?, path: exe }
    } else {
        let procs = find_process(&config.process)?;
        if procs.is_empty() {
            bail!("No running process for {:?}. Use --offline to scan the exe on disk.", config.process.names);
        }
        let pid = procs[0].pid;
        let mut pm = ProcessMemory::open(pid)?;
        let base = pm.find_module_base(&module_name)?;
        ScanTarget::Live { pid, base, pm }
    };
    println!("Mode: {}", target.describe());

    // 3. Scan each signature-based cheat
    let mut ok = 0usize;
    let mut ambiguous = 0usize;
    let mut missing = 0usize;
    let mut nopattern = 0usize;
    let mut repairs: Vec<(u32, String, String)> = Vec::new(); // (id, new pattern, reason)

    for cheat_cfg in &config.cheats {
        let signature = match &cheat_cfg.cheat_type {
            CheatTypeConfig::Nop { signature, .. } => signature.clone(),
            CheatTypeConfig::SignaturePatch { signature, .. } => signature.clone(),
            _ => continue,
        };
        if signature.trim().is_empty() {
            nopattern += 1;
            println!("  [{:>3}] {:<42} {}", cheat_cfg.id, truncate(&cheat_cfg.name, 42), SigStatus::NoPattern.label());
            continue;
        }
        let sig = Signature::parse(&signature)?;

        let (hits, drift_candidates) = match &mut target {
            ScanTarget::Live { pm, .. } => {
                let mut scanner = SignatureScanner::new(pm);
                let hits = scanner.scan_game_code(&sig)?;
                let drift = if hits.is_empty() {
                    scanner.find_drifted(&sig, max_relax)?
                } else {
                    Vec::new()
                };
                (hits, drift)
            }
            ScanTarget::File { pe, .. } => {
                let hits = scan_pe_file(pe, &sig, true)?;
                let drift = if hits.is_empty() {
                    find_drifted_in_file(pe, &sig, max_relax, true)?
                } else {
                    Vec::new()
                };
                (hits, drift)
            }
        };

        let status = if hits.is_empty() {
            SigStatus::Missing
        } else if hits.len() > 1 {
            SigStatus::Ambiguous
        } else {
            SigStatus::Ok
        };

        match status {
            SigStatus::Ok => ok += 1,
            SigStatus::Ambiguous => ambiguous += 1,
            SigStatus::Missing => missing += 1,
            SigStatus::NoPattern => nopattern += 1,
        }

        // Address rendering
        let loc = match &target {
            ScanTarget::Live { base, .. } => {
                hits.first().map(|h| format!("{}+0x{:X}", module_name, h - base))
            }
            ScanTarget::File { pe, .. } => hits
                .first()
                .and_then(|h| pe.file_offset_to_rva(*h))
                .map(|rva| format!("{}+0x{:X}", module_name, rva)),
        };

        let mut line = format!("  [{:>3}] {:<42} {:<10}", cheat_cfg.id, truncate(&cheat_cfg.name, 42), status.label());
        if let Some(l) = loc {
            line.push_str(&format!(" {}", l));
        }
        if hits.len() > 1 {
            line.push_str(&format!(" ({} hits)", hits.len()));
        }
        println!("{}", line);

        // Drift suggestion
        if let Some(cand) = drift_candidates.first() {
            let suggested = cand.suggested_pattern();
            let relaxed_at = cand.relaxed_positions.iter()
                .map(|p| p.to_string()).collect::<Vec<_>>().join(",");
            println!("        ↳ drifted; relaxed bytes [{}] match, suggested: '{}'",
                relaxed_at, suggested);
            repairs.push((cheat_cfg.id, suggested, format!("drift repair (relaxed bytes {})", relaxed_at)));
        } else if suggest && status == SigStatus::Ok {
            if let Some(portable) = portable_suggestion(&mut target, &sig, hits.first().copied()) {
                println!("        ↳ portable pattern: '{}'", portable);
            }
        }
    }

    println!();
    println!("Summary: {} OK, {} ambiguous, {} missing, {} without pattern", ok, ambiguous, missing, nopattern);

    // 4. Optionally write repairs back into the config
    if repair && !repairs.is_empty() {
        let path = find_config_path(&config_dir, game)
            .context("Could not locate the game config file to repair")?;
        let mut text = std::fs::read_to_string(&path)?;
        let mut applied = 0usize;
        for (id, pattern, _reason) in &repairs {
            // Replace the signature line inside the block whose id matches.
            if let Some(pos) = text.find(&format!("id = {}", id)) {
                if let Some(sig_pos) = text[pos..].find("signature = \"") {
                    let start = pos + sig_pos + "signature = \"".len();
                    if let Some(end_rel) = text[start..].find('"') {
                        let end = start + end_rel;
                        text.replace_range(start..end, pattern);
                        applied += 1;
                    }
                }
            }
        }
        std::fs::write(&path, &text)?;
        println!("Repaired {} signature(s) in {}", applied, path.display());
        // Re-validate
        let revalidated = GameConfig::load_from_file(&path)?;
        println!("Config still valid: {} cheats", revalidated.cheats.len());
    }

    if ambiguous > 0 || missing > 0 {
        Ok(1)
    } else {
        Ok(0)
    }
}

/// Suggest a relocation-proof pattern by masking volatile bytes.
fn portable_suggestion(
    target: &mut ScanTarget,
    sig: &Signature,
    hit: Option<usize>,
) -> Option<String> {
    let hit = hit?;
    let code = target.read_at(hit, sig.len())?;
    let ip = target.ip_for(hit);
    let mask = trainer_core::disasm::volatile_mask(&code, ip);
    if mask.is_empty() {
        return None;
    }
    Some(trainer_core::disasm::portable_pattern(&code, &mask))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

/// Locate the game's executable on disk (for offline scanning).
fn find_game_exe(config: &trainer_core::config::GameConfig) -> Result<std::path::PathBuf> {
    let name = config.process.module_name.clone()
        .unwrap_or_else(|| config.process.names[0].clone());
    let name = name.to_lowercase();

    // Look through the user's game directories and Steam/Lutris libraries.
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    for base in [
        std::env::var("HOME").unwrap_or_default(),
    ] {
        let base = std::path::PathBuf::from(base);
        for sub in ["games", "Games", "GOG Games", ".steam/steamapps/common", ".local/share/Steam/steamapps/common", ".local/share/lutris/games"] {
            let p = base.join(sub);
            if p.exists() { roots.push(p); }
        }
    }

    for root in roots {
        // Bounded search: two levels deep is enough for common layouts.
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let direct = path.join(&name);
            if direct.is_file() {
                return Ok(direct);
            }
            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                for sub in sub_entries.flatten() {
                    let candidate = sub.path().join(&name);
                    if candidate.is_file() {
                        return Ok(candidate);
                    }
                }
            }
        }
    }

    bail!("Could not find '{}' on disk. Pass --exe <path>.", name)
}

/// Locate a game config file on disk (for --repair).
fn find_config_path(config_dir: &Option<PathBuf>, game: &str) -> Option<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(d) = config_dir { dirs.push(d.clone()); }
    if let Some(d) = dirs::config_dir() {
        dirs.push(d.join("trainer").join("games"));
    }
    dirs.push(std::path::PathBuf::from("games"));

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") { continue; }
            if let Ok(cfg) = GameConfig::load_from_file(&path) {
                if cfg.metadata.name == game {
                    return Some(path);
                }
            }
        }
    }
    None
}
