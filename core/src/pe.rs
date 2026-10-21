//! Minimal PE (Portable Executable) parser.
//!
//! Used for offline signature scanning: scan a game's `.exe` on disk (no
//! running process required) and report matches as module-relative RVAs,
//! exactly like the live-memory path does.

use anyhow::{Result, bail};
use std::path::Path;

/// A PE section with file- and memory-relative extents.
#[derive(Debug, Clone)]
pub struct PeSection {
    pub name: String,
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_pointer: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}

impl PeSection {
    /// True when the section holds executable code.
    pub fn is_executable(&self) -> bool {
        self.characteristics & 0x2000_0000 != 0 // IMAGE_SCN_MEM_EXECUTE
    }
    /// True when the section holds initialized data.
    pub fn is_initialized_data(&self) -> bool {
        self.characteristics & 0x0000_0040 != 0 // IMAGE_SCN_CNT_INITIALIZED_DATA
    }
}

/// Parsed PE image header plus the raw file bytes.
pub struct PeImage {
    pub image_base: u64,
    pub sections: Vec<PeSection>,
    pub data: Vec<u8>,
}

impl PeImage {
    /// Parse a PE file from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path.display(), e))?;
        Self::from_bytes(data)
    }

    /// Parse a PE image from an in-memory buffer.
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        if data.len() < 0x40 || &data[0..2] != b"MZ" {
            bail!("Not a PE file (missing MZ header)");
        }
        let e_lfanew = u32::from_le_bytes(data[0x3C..0x40].try_into().unwrap()) as usize;
        if e_lfanew + 0x18 > data.len() || &data[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
            bail!("Not a PE file (missing PE signature)");
        }

        let coff = e_lfanew + 4;
        let num_sections = u16::from_le_bytes(data[coff + 2..coff + 4].try_into().unwrap()) as usize;
        let opt_size = u16::from_le_bytes(data[coff + 16..coff + 18].try_into().unwrap()) as usize;
        let opt = coff + 20;

        let magic = u16::from_le_bytes(data[opt..opt + 2].try_into().unwrap());
        let image_base = match magic {
            0x20b => u64::from_le_bytes(data[opt + 24..opt + 32].try_into().unwrap()),
            0x10b => u32::from_le_bytes(data[opt + 28..opt + 32].try_into().unwrap()) as u64,
            _ => bail!("Unknown PE optional header magic 0x{:x}", magic),
        };

        let sec_off = opt + opt_size;
        let mut sections = Vec::with_capacity(num_sections);
        for i in 0..num_sections {
            let off = sec_off + i * 40;
            if off + 40 > data.len() {
                break;
            }
            let name_bytes = &data[off..off + 8];
            let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(8);
            let name = String::from_utf8_lossy(&name_bytes[..end]).to_string();
            let virtual_size = u32::from_le_bytes(data[off + 8..off + 12].try_into().unwrap());
            let virtual_address = u32::from_le_bytes(data[off + 12..off + 16].try_into().unwrap());
            let raw_size = u32::from_le_bytes(data[off + 16..off + 20].try_into().unwrap());
            let raw_pointer = u32::from_le_bytes(data[off + 20..off + 24].try_into().unwrap());
            let characteristics = u32::from_le_bytes(data[off + 36..off + 40].try_into().unwrap());
            sections.push(PeSection {
                name,
                virtual_address,
                virtual_size,
                raw_pointer,
                raw_size,
                characteristics,
            });
        }

        Ok(Self { image_base, sections, data })
    }

    /// Executable sections as (file_offset, size, rva_base) for scanning.
    pub fn executable_ranges(&self) -> Vec<(usize, usize, u32)> {
        self.sections
            .iter()
            .filter(|s| s.is_executable() && s.raw_size > 0)
            .map(|s| (s.raw_pointer as usize, s.raw_size as usize, s.virtual_address))
            .collect()
    }

    /// All data-bearing sections (code + initialized data) for value scanning.
    pub fn data_ranges(&self) -> Vec<(usize, usize, u32)> {
        self.sections
            .iter()
            .filter(|s| (s.is_executable() || s.is_initialized_data()) && s.raw_size > 0)
            .map(|s| (s.raw_pointer as usize, s.raw_size as usize, s.virtual_address))
            .collect()
    }

    /// Convert a file offset to its RVA (module-relative address).
    pub fn file_offset_to_rva(&self, file_offset: usize) -> Option<u32> {
        for s in &self.sections {
            let start = s.raw_pointer as usize;
            let end = start + s.raw_size as usize;
            if file_offset >= start && file_offset < end {
                return Some(s.virtual_address + (file_offset - start) as u32);
            }
        }
        None
    }
}
