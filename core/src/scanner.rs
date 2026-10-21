use crate::memory::{MemoryRegion, ProcessMemory};
use crate::pe::PeImage;
use anyhow::{Result, Context, bail};
use std::io::{Read, Seek, SeekFrom};

/// Parsed signature pattern for scanning
#[derive(Debug, Clone)]
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
    
    /// Indices of the fixed (non-wildcard) bytes.
    pub fn fixed_positions(&self) -> Vec<usize> {
        self.bytes.iter().enumerate().filter_map(|(i, b)| b.map(|_| i)).collect()
    }
    
    /// A copy of this signature with the given positions turned into wildcards.
    pub fn wildcarded(&self, positions: &[usize]) -> Signature {
        let mut bytes = self.bytes.clone();
        for &p in positions {
            if p < bytes.len() {
                bytes[p] = None;
            }
        }
        Signature { bytes }
    }
    
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
    
    /// Render back to a config-style string ("F3 0F 11 ?? 4A D4").
    pub fn to_pattern_string(&self) -> String {
        self.bytes.iter().map(|b| match b {
            Some(v) => format!("{:02X}", v),
            None => "??".to_string(),
        }).collect::<Vec<_>>().join(" ")
    }
}

/// A near-match found while hunting for a drifted pattern.
#[derive(Debug, Clone)]
pub struct DriftCandidate {
    /// Address of the near-match (live VA or file offset).
    pub address: usize,
    /// The relaxed pattern that matched (wildcards at the drifting bytes).
    pub relaxed: Signature,
    /// Byte positions that were relaxed to find this match.
    pub relaxed_positions: Vec<usize>,
    /// The real bytes found at `address` — the corrected pattern.
    pub actual_bytes: Vec<u8>,
}

impl DriftCandidate {
    /// Corrected pattern string, ready to paste into a game config.
    pub fn suggested_pattern(&self) -> String {
        self.actual_bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
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
        scan_bytes(&mut |off, buf| {
            self.process.mem_file_mut().seek(SeekFrom::Start((region.start + off) as u64))?;
            self.process.mem_file_mut().read_exact(buf)?;
            Ok(())
        }, 0, region.end - region.start, sig)
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

    /// Hunt for a pattern that has drifted between game builds.
    ///
    /// Progressively relaxes up to `max_relax` byte positions to wildcards and
    /// rescans; the first relaxation level that yields hits produces
    /// candidates carrying the *real* bytes found, i.e. the corrected
    /// pattern. This is what turns "signature not found" into an actionable
    /// fix (e.g. `09 C2 41 89 13` -> `0B D0 41 89 13`).
    pub fn find_drifted(&mut self, sig: &Signature, max_relax: usize) -> Result<Vec<DriftCandidate>> {
        let fixed = sig.fixed_positions();
        if fixed.len() < 3 {
            bail!("Pattern too short to relax safely ({} fixed bytes)", fixed.len());
        }

        for relax_count in 1..=max_relax.min(fixed.len()) {
            let mut candidates = Vec::new();
            for combo in combinations(&fixed, relax_count) {
                let relaxed = sig.wildcarded(&combo);
                let hits = self.scan_game_code(&relaxed)?;
                if hits.is_empty() {
                    continue;
                }
                for hit in hits {
                    let mut actual = vec![0u8; sig.len()];
                    self.process.read_memory(hit, &mut actual)?;
                    candidates.push(DriftCandidate {
                        address: hit,
                        relaxed: relaxed.clone(),
                        relaxed_positions: combo.clone(),
                        actual_bytes: actual,
                    });
                }
                // A relaxation level that hits is enough; more hits at this
                // level just add noise.
                if !candidates.is_empty() {
                    candidates.sort_by_key(|c| c.address);
                    candidates.dedup_by_key(|c| c.address);
                    return Ok(candidates);
                }
            }
        }
        Ok(Vec::new())
    }
}

/// Scan a byte source for a signature.
///
/// `read` is called with (offset, buffer) and must fill the buffer.
fn scan_bytes<F>(read: &mut F, start: usize, len: usize, sig: &Signature) -> Result<Vec<usize>>
where
    F: FnMut(usize, &mut [u8]) -> Result<()>,
{
    if sig.is_empty() || len < sig.len() {
        return Ok(Vec::new());
    }

    const CHUNK: usize = 1 << 20; // 1 MiB
    let mut results = Vec::new();
    let mut buf = vec![0u8; CHUNK + sig.len()];
    let fixed = sig.fixed_positions();
    if fixed.is_empty() {
        return Ok(Vec::new());
    }
    // Anchor on the first fixed byte so we can use memchr-backed search.
    let anchor = fixed[0];
    let anchor_byte = sig.bytes[anchor].unwrap();

    let mut offset = start;
    let end = start + len;
    while offset < end {
        let want = CHUNK.min(end - offset);
        read(offset, &mut buf[..want])?;

        // memchr-style search for the anchor byte
        let mut search = 0usize;
        while search < want {
            let hit = match buf[search..want].iter().position(|&b| b == anchor_byte) {
                Some(i) => search + i,
                None => break,
            };
            if hit < anchor {
                search = hit + 1;
                continue;
            }
            let cand = hit - anchor;
            if sig.matches(&buf, cand) {
                results.push(offset + cand);
            }
            search = hit + 1;
        }
        offset += want;
    }
    Ok(results)
}

/// Scan a PE file on disk (offline mode) for a signature.
///
/// Returns file offsets of matches.
pub fn scan_pe_file(pe: &PeImage, sig: &Signature, executable_only: bool) -> Result<Vec<usize>> {
    let ranges = if executable_only { pe.executable_ranges() } else { pe.data_ranges() };
    let mut results = Vec::new();
    for (file_off, size, _rva) in ranges {
        let mut read = |off: usize, buf: &mut [u8]| -> Result<()> {
            let start = file_off + off;
            let end = (start + buf.len()).min(pe.data.len());
            if start >= end {
                bail!("read past end of file");
            }
            buf[..end - start].copy_from_slice(&pe.data[start..end]);
            Ok(())
        };
        let hits = scan_bytes(&mut read, 0, size, sig)?;
        results.extend(hits.into_iter().map(|h| file_off + h));
    }
    Ok(results)
}

/// Offline drift hunt on a PE file.
pub fn find_drifted_in_file(
    pe: &PeImage,
    sig: &Signature,
    max_relax: usize,
    executable_only: bool,
) -> Result<Vec<DriftCandidate>> {
    let fixed = sig.fixed_positions();
    if fixed.len() < 3 {
        bail!("Pattern too short to relax safely ({} fixed bytes)", fixed.len());
    }
    for relax_count in 1..=max_relax.min(fixed.len()) {
        let mut candidates = Vec::new();
        for combo in combinations(&fixed, relax_count) {
            let relaxed = sig.wildcarded(&combo);
            let hits = scan_pe_file(pe, &relaxed, executable_only)?;
            for hit in hits {
                let end = hit + sig.len();
                if end > pe.data.len() {
                    continue;
                }
                let actual = pe.data[hit..end].to_vec();
                candidates.push(DriftCandidate {
                    address: hit,
                    relaxed: relaxed.clone(),
                    relaxed_positions: combo.clone(),
                    actual_bytes: actual,
                });
            }
            if !candidates.is_empty() {
                candidates.sort_by_key(|c| c.address);
                candidates.dedup_by_key(|c| c.address);
                return Ok(candidates);
            }
        }
    }
    Ok(Vec::new())
}

/// All k-element combinations of `items` (order preserved).
fn combinations(items: &[usize], k: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    if k == 0 || k > items.len() {
        return out;
    }
    let n = items.len();
    let mut idx: Vec<usize> = (0..k).collect();
    loop {
        out.push(idx.iter().map(|&i| items[i]).collect());
        let mut i = k;
        loop {
            if i == 0 {
                return out;
            }
            i -= 1;
            if idx[i] != i + n - k {
                break;
            }
            if i == 0 {
                return out;
            }
        }
        idx[i] += 1;
        for j in i + 1..k {
            idx[j] = idx[j - 1] + 1;
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
        assert_eq!(sig.bytes.len(), 15);
        assert_eq!(sig.bytes[0], Some(0x48));
        assert_eq!(sig.bytes[4], None);
        assert_eq!(sig.bytes[9], None);
        assert_eq!(sig.bytes[14], Some(0x20));
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

    #[test]
    fn test_wildcarded_and_positions() {
        let sig = Signature::parse("09 C2 41 89 13").unwrap();
        assert_eq!(sig.fixed_positions(), vec![0, 1, 2, 3, 4]);
        let w = sig.wildcarded(&[0, 1]);
        assert_eq!(w.to_pattern_string(), "?? ?? 41 89 13");
        assert_eq!(w.fixed_positions(), vec![2, 3, 4]);
    }

    #[test]
    fn test_combinations() {
        let c = combinations(&[0, 1, 2, 3], 2);
        assert_eq!(c.len(), 6);
        assert!(c.contains(&vec![0, 1]));
        assert!(c.contains(&vec![2, 3]));
        assert!(c.contains(&vec![1, 3]));
    }

    #[test]
    fn test_pattern_string_roundtrip() {
        let sig = Signature::parse("f3 0f 11 4a d4").unwrap();
        assert_eq!(sig.to_pattern_string(), "F3 0F 11 4A D4");
    }

    #[test]
    fn test_scan_bytes_finds_and_skips() {
        let data: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        let sig = Signature::parse("00 01 02 03").unwrap();
        let mut read = |off: usize, buf: &mut [u8]| -> Result<()> {
            buf.copy_from_slice(&data[off..off + buf.len()]);
            Ok(())
        };
        let hits = scan_bytes(&mut read, 0, data.len(), &sig).unwrap();
        assert_eq!(hits, vec![0, 256, 512, 768, 1024, 1280, 1536, 1792, 2048, 2304, 2560, 2816, 3072, 3328, 3584, 3840]);
    }

    #[test]
    fn test_scan_bytes_with_wildcard() {
        let data = vec![0xAAu8, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
        let sig = Signature::parse("?? CC DD").unwrap();
        let mut read = |off: usize, buf: &mut [u8]| -> Result<()> {
            buf.copy_from_slice(&data[off..off + buf.len()]);
            Ok(())
        };
        let hits = scan_bytes(&mut read, 0, data.len(), &sig).unwrap();
        assert_eq!(hits, vec![1]);
    }
}
