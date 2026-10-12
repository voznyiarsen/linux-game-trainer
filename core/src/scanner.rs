use crate::memory::{MemoryRegion, ProcessMemory};
use anyhow::{Result, Context, bail};
use std::io::{Read, Seek, SeekFrom};

/// Parsed signature pattern for scanning
pub struct Signature {
    pub bytes: Vec<Option<u8>>,
}

impl Signature {
    /// Parse a signature string like "48 89 5C 24 ?? 48 89 74 24 ?? 57 48 83 EC 20"
    pub fn parse(sig_str: &str) -> Result<Self> {
        let mut bytes = Vec::new();
        
        for part in sig_str.split_whitespace() {
            if part == "??" || part == "?" {
                bytes.push(None);
            } else {
                let byte = u8::from_str_radix(part, 16)
                    .context(format!("Invalid hex byte: {}", part))?;
                bytes.push(Some(byte));
            }
        }
        
        Ok(Self { bytes })
    }
    
    pub fn len(&self) -> usize { self.bytes.len() }
    pub fn is_empty(&self) -> bool { self.bytes.is_empty() }
    
    /// Check if a memory slice matches this signature
    pub fn matches(&self, memory: &[u8], offset: usize) -> bool {
        if offset + self.bytes.len() > memory.len() { return false; }
        
        for (i, &sig_byte) in self.bytes.iter().enumerate() {
            if let Some(expected) = sig_byte {
                if memory[offset + i] != expected { return false; }
            }
        }
        true
    }
}

/// Signature scanner for finding code/data patterns in process memory
pub struct SignatureScanner<'a> {
    process: &'a mut ProcessMemory,
}

impl<'a> SignatureScanner<'a> {
    pub fn new(process: &'a mut ProcessMemory) -> Self { Self { process } }
    
    /// Scan a specific region for a signature
    pub fn scan_region(&mut self, region: &MemoryRegion, sig: &Signature) -> Result<Vec<usize>> {
        if sig.is_empty() { return Ok(Vec::new()); }
        
        const CHUNK_SIZE: usize = 65536;
        let mut results = Vec::new();
        let mut buf = vec![0u8; CHUNK_SIZE];
        let sig_len = sig.len();
        
        let mut addr = region.start;
        let mut overlap_buf = vec![0u8; sig_len.saturating_sub(1)];
        
        while addr < region.end {
            let read_size = CHUNK_SIZE.min(region.end - addr);
            
            self.process.mem_file_mut().seek(std::io::SeekFrom::Start(addr as u64))?;
            self.process.mem_file_mut().read_exact(&mut buf[..read_size])?;
            
            for i in 0..read_size.saturating_sub(sig_len) {
                if sig.matches(&buf, i) {
                    results.push(addr + i);
                }
            }
            
            if read_size == CHUNK_SIZE && addr + CHUNK_SIZE < region.end && sig_len > 1 {
                let overlap_start = CHUNK_SIZE - (sig_len - 1);
                overlap_buf.copy_from_slice(&buf[overlap_start..]);
                
                let next_addr = addr + CHUNK_SIZE;
                self.process.mem_file_mut().seek(std::io::SeekFrom::Start(next_addr as u64))?;
                let mut next_bytes = vec![0u8; sig_len - 1];
                self.process.mem_file_mut().read_exact(&mut next_bytes)?;
                
                let combined: Vec<u8> = overlap_buf.iter().chain(next_bytes.iter()).cloned().collect();
                for i in 0..(sig_len - 1) {
                    if sig.matches(&combined, i) {
                        results.push(addr + overlap_start + i);
                    }
                }
            }
            
            addr += read_size;
        }
        
        Ok(results)
    }
    
    /// Scan all executable game regions for a signature
    pub fn scan_game_code(&mut self, sig: &Signature) -> Result<Vec<usize>> {
        let regions = self.process.get_game_regions();
        let mut all_results = Vec::new();
        
        for region in regions {
            let results = self.scan_region(&region, sig)?;
            all_results.extend(results);
        }
        
        Ok(all_results)
    }
    
    /// Scan all writable game regions for a signature
    pub fn scan_game_data(&mut self, sig: &Signature) -> Result<Vec<usize>> {
        let regions = self.process.get_writable_regions();
        let mut all_results = Vec::new();
        
        for region in regions {
            let results = self.scan_region(&region, sig)?;
            all_results.extend(results);
        }
        
        Ok(all_results)
    }
    
    /// Find a unique signature match
    pub fn find_unique(&mut self, sig: &Signature) -> Result<usize> {
        let matches = self.scan_game_code(sig)?;
        
        match matches.len() {
            0 => bail!("Signature not found: no matches"),
            1 => Ok(matches[0]),
            n => bail!("Signature not unique: found {} matches", n),
        }
    }
}

/// Helper to create signatures from IDA-style patterns
pub fn ida_pattern(pattern: &str) -> Result<Signature> {
    Signature::parse(pattern)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_signature_parse() {
        let sig = Signature::parse("48 89 5C 24 ?? 48 89 74 24 ?? 57 48 83 EC 20").unwrap();
        assert_eq!(sig.bytes.len(), 14);
        assert_eq!(sig.bytes[0], Some(0x48));
        assert_eq!(sig.bytes[4], None);
        assert_eq!(sig.bytes[9], None);
    }
    
    #[test]
    fn test_signature_match() {
        let sig = Signature::parse("48 89 5C 24 ??").unwrap();
        let mem = vec![0x48, 0x89, 0x5C, 0x24, 0x10, 0x48];
        assert!(sig.matches(&mem, 0));
        assert!(!sig.matches(&mem, 1));
        
        let mem2 = vec![0x48, 0x89, 0x5C, 0x25, 0x10];
        assert!(!sig.matches(&mem2, 0));
    }
}