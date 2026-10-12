use std::fs::File;
use std::io::{Read, Write, Seek, SeekFrom};
use nix::sys::ptrace;
use nix::sys::signal;
use nix::sys::wait;
use nix::unistd::Pid;
use anyhow::{Result, Context, bail};

/// Represents a memory region in the target process
#[derive(Debug, Clone)]
pub struct MemoryRegion {
    pub start: usize,
    pub end: usize,
    pub permissions: String,
    pub path: String,
}

/// Wrapper for accessing target process memory via /proc/pid/mem
pub struct ProcessMemory {
    pid: Pid,
    mem_file: File,
    regions: Vec<MemoryRegion>,
}

impl ProcessMemory {
    /// Attach to a process and open its memory file
    /// NOTE: PTRACE_ATTACH delivers SIGSTOP (not SIGTRAP), so accept any stop.
    pub fn attach(pid: i32) -> Result<Self> {
        let pid = Pid::from_raw(pid);

        ptrace::attach(pid).context("Failed to attach to process (ptrace)")?;

        // Wait for the group-stop. Must accept SIGSTOP; requiring SIGTRAP hangs forever.
        let status = wait::waitpid(pid, None).context("Failed to wait for process")?;
        if !matches!(status, wait::WaitStatus::Stopped(_, _)) {
            // Not stopped as expected; detach to avoid leaving the game frozen.
            let _ = ptrace::detach(pid, None);
            anyhow::bail!("Unexpected wait status after attach: {:?}", status);
        }

        let mut pm = Self::open(pid.as_raw())?;
        // Mark as attached by re-attaching state: open() doesn't attach, so we need
        // to keep track. Simplest: store attached flag via pid only; Drop always
        // tries detach which is harmless if not attached (will error, ignored).
        // To avoid double-detach issues, we just return pm (attached).
        pm.refresh_regions()?;
        return Ok(pm);
    }

    /// Open process memory WITHOUT ptrace attach (read-only scanning, no freeze).
    /// Writes may still work for same-UID processes, but attach() is recommended
    /// before writing.
    pub fn open(pid: i32) -> Result<Self> {
        let pid_raw = pid;
        let pid = Pid::from_raw(pid);

        let mem_path = format!("/proc/{}/mem", pid_raw);
        let mem_file = File::options()
            .read(true)
            .write(true)
            .open(&mem_path)
            .context("Failed to open /proc/pid/mem")?;

        let mut pm = Self {
            pid,
            mem_file,
            regions: Vec::new(),
        };

        pm.refresh_regions()?;
        Ok(pm)
    }
    
    /// Detach from the process
    pub fn detach(&mut self) -> Result<()> {
        ptrace::detach(self.pid, None).context("Failed to detach from process")?;
        Ok(())
    }
    
    /// Refresh memory regions from /proc/pid/maps
    pub fn refresh_regions(&mut self) -> Result<()> {
        let maps_path = format!("/proc/{}/maps", self.pid);
        let maps_content = std::fs::read_to_string(&maps_path)
            .context("Failed to read /proc/pid/maps")?;
        
        self.regions = maps_content
            .lines()
            .filter_map(|line| parse_maps_line(line))
            .collect();
        
        Ok(())
    }
    
    /// Find the module base address
    /// Handles both native (file-backed executable) and Wine (header is r--p file-backed,
    /// code is anonymous) layouts.
    pub fn find_module_base(&mut self, module_name: &str) -> Result<usize> {
        // First pass: lowest mapping whose path contains the module name (header mapping).
        // In Wine this is r--p at e.g. 0x140000000; native may be r-xp.
        let mut best: Option<usize> = None;
        for region in &self.regions {
            if region.path.contains(module_name) {
                match best {
                    None => best = Some(region.start),
                    Some(b) if region.start < b => best = Some(region.start),
                    _ => {}
                }
            }
        }
        if let Some(base) = best {
            return Ok(base);
        }

        // Fallback: search executable file-backed regions for PE header
        let regions: Vec<_> = self.regions.clone();
        for region in regions {
            if region.permissions.contains('x') && (region.path.contains(".exe") || region.path.contains(module_name)) {
                if let Ok(Some(base)) = self.find_pe_header(region.start, region.end) {
                    return Ok(base);
                }
            }
        }

        bail!("Could not find module base for: {}", module_name)
    }
    
    fn find_pe_header(&mut self, start: usize, end: usize) -> Result<Option<usize>> {
        const CHUNK_SIZE: usize = 4096;
        let mut buf = vec![0u8; CHUNK_SIZE];
        
        let mut addr = start;
        while addr < end {
            let read_size = CHUNK_SIZE.min(end - addr);
            self.mem_file.seek(SeekFrom::Start(addr as u64))?;
            self.mem_file.read_exact(&mut buf[..read_size])?;
            
            for i in 0..read_size.saturating_sub(1) {
                if buf[i] == b'M' && buf[i + 1] == b'Z' {
                    return Ok(Some(addr + i));
                }
            }
            addr += read_size;
        }
        Ok(None)
    }
    
    /// Read memory from the target process
    pub fn read_memory(&mut self, addr: usize, buf: &mut [u8]) -> Result<()> {
        self.mem_file.seek(SeekFrom::Start(addr as u64))?;
        self.mem_file.read_exact(buf)
            .context(format!("Failed to read memory at 0x{:x}", addr))?;
        Ok(())
    }
    
    /// Write memory to the target process
    pub fn write_memory(&mut self, addr: usize, buf: &[u8]) -> Result<()> {
        self.mem_file.seek(SeekFrom::Start(addr as u64))?;
        self.mem_file.write_all(buf)
            .context(format!("Failed to write memory at 0x{:x}", addr))?;
        Ok(())
    }
    
    /// Read a typed value from memory
    pub fn read_u8(&mut self, addr: usize) -> Result<u8> {
        let mut buf = [0u8; 1];
        self.read_memory(addr, &mut buf)?;
        Ok(buf[0])
    }
    
    pub fn read_u16(&mut self, addr: usize) -> Result<u16> {
        let mut buf = [0u8; 2];
        self.read_memory(addr, &mut buf)?;
        Ok(u16::from_le_bytes(buf))
    }
    
    pub fn read_u32(&mut self, addr: usize) -> Result<u32> {
        let mut buf = [0u8; 4];
        self.read_memory(addr, &mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }
    
    pub fn read_u64(&mut self, addr: usize) -> Result<u64> {
        let mut buf = [0u8; 8];
        self.read_memory(addr, &mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }
    
    /// Write a typed value to memory
    pub fn write_u8(&mut self, addr: usize, value: u8) -> Result<()> {
        self.write_memory(addr, &[value])
    }
    
    pub fn write_u16(&mut self, addr: usize, value: u16) -> Result<()> {
        self.write_memory(addr, &value.to_le_bytes())
    }
    
    pub fn write_u32(&mut self, addr: usize, value: u32) -> Result<()> {
        self.write_memory(addr, &value.to_le_bytes())
    }
    
    pub fn write_u64(&mut self, addr: usize, value: u64) -> Result<()> {
        self.write_memory(addr, &value.to_le_bytes())
    }
    
    /// Search for a byte pattern in a memory region
    pub fn find_pattern(&mut self, region: &MemoryRegion, pattern: &[u8], mask: Option<&[bool]>) -> Result<Vec<usize>> {
        const CHUNK_SIZE: usize = 65536;
        let mut results = Vec::new();
        let mut buf = vec![0u8; CHUNK_SIZE];
        
        let default_mask: Vec<bool> = vec![true; pattern.len()];
        let mask_ref: &[bool] = mask.unwrap_or(&default_mask);
        
        let mut addr = region.start;
        while addr < region.end {
            let read_size = CHUNK_SIZE.min(region.end - addr);
            self.mem_file.seek(SeekFrom::Start(addr as u64))?;
            self.mem_file.read_exact(&mut buf[..read_size])?;
            
            for i in 0..read_size.saturating_sub(pattern.len()) {
                let mut matches = true;
                for (j, (&pat_byte, &mask_bit)) in pattern.iter().zip(mask_ref.iter()).enumerate() {
                    if mask_bit && buf[i + j] != pat_byte {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    results.push(addr + i);
                }
            }
            addr += read_size;
        }
        
        Ok(results)
    }
    
    /// Get all executable regions belonging to the game
    /// Includes Wine-style anonymous executable mappings near the main module
    /// (Wine maps PE sections as anonymous memory, only the header stays file-backed).
    pub fn get_game_regions(&self) -> Vec<MemoryRegion> {
        // Find the main module base to define a window for anonymous game mappings.
        // 0x140000000 is the standard Wine/Windows PE ImageBase; use a generous window.
        let game_bases: Vec<usize> = self.regions.iter()
            .filter(|r| r.path.contains(".exe"))
            .map(|r| r.start & !0xFFFFFF)
            .collect();

        self.regions.iter()
            .filter(|r| {
                if !r.permissions.contains('x') {
                    return false;
                }
                // File-backed exe/dll mappings
                if r.path.contains(".exe") {
                    return true;
                }
                // Wine anonymous game code: executable anonymous mapping within ~256MB
                // above a known exe mapping (covers 0x140000000-0x150000000 range).
                if r.path.is_empty() {
                    for base in &game_bases {
                        if r.start >= *base && r.start < base.saturating_add(0x10000000) {
                            return true;
                        }
                    }
                    // Fallback: anonymous executable in the canonical Wine PE range
                    if (0x130000000..0x150000000).contains(&r.start) {
                        return true;
                    }
                }
                false
            })
            .cloned()
            .collect()
    }
    
    /// Get all writable regions belonging to the game
    pub fn get_writable_regions(&self) -> Vec<MemoryRegion> {
        self.regions.iter()
            .filter(|r| r.permissions.contains('w') && (r.path.contains(".exe") || r.path.contains("game") || r.path == "[heap]"))
            .cloned()
            .collect()
    }
    
    pub fn mem_file_mut(&mut self) -> &mut File {
        &mut self.mem_file
    }
    
    pub fn pid(&self) -> Pid {
        self.pid
    }
}

impl Drop for ProcessMemory {
    fn drop(&mut self) {
        let _ = self.detach();
    }
}

fn parse_maps_line(line: &str) -> Option<MemoryRegion> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 5 { return None; }
    
    let addr_range = parts[0];
    let perms = parts[1];
    let path = if parts.len() >= 6 { parts[5..].join(" ") } else { String::new() };
    
    let addrs: Vec<&str> = addr_range.split('-').collect();
    if addrs.len() != 2 { return None; }
    
    let start = usize::from_str_radix(addrs[0], 16).ok()?;
    let end = usize::from_str_radix(addrs[1], 16).ok()?;
    
    Some(MemoryRegion { start, end, permissions: perms.to_string(), path })
}