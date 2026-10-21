//! Integration tests for the offline scan / drift-repair pipeline.
//!
//! Builds a minimal but valid PE64 image in memory, then verifies:
//! - exact pattern scanning finds the right file offset,
//! - a drifted pattern is repaired to the correct bytes,
//! - volatile-byte masking produces a portable pattern.

use trainer_core::disasm::{portable_pattern, volatile_mask};
use trainer_core::pe::PeImage;
use trainer_core::scanner::{find_drifted_in_file, scan_pe_file, Signature};

/// Build a minimal PE64 with one executable section holding `code`.
fn build_pe(code: &[u8]) -> Vec<u8> {
    const DOS_SIZE: usize = 0x80;
    const HEADERS_SIZE: usize = 0x400; // file-aligned header size
    const SECTION_RVA: u32 = 0x1000;

    let mut out = vec![0u8; HEADERS_SIZE + code.len()];

    // DOS header
    out[0] = b'M';
    out[1] = b'Z';
    out[0x3C..0x40].copy_from_slice(&(DOS_SIZE as u32).to_le_bytes());

    // PE signature (at e_lfanew) + COFF header (right after it)
    out[DOS_SIZE..DOS_SIZE + 4].copy_from_slice(b"PE\0\0");
    let coff = DOS_SIZE + 4;
    out[coff + 0..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes()); // machine x64
    out[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes()); // 1 section
    out[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes()); // optional header size
    out[coff + 20..coff + 22].copy_from_slice(&0x20bu16.to_le_bytes()); // PE32+ magic

    // Optional header: ImageBase at +24
    let opt = coff + 20;
    out[opt + 24..opt + 32].copy_from_slice(&0x140000000u64.to_le_bytes());

    // Section header
    let sec = opt + 240;
    out[sec..sec + 8].copy_from_slice(b".text\0\0\0");
    out[sec + 8..sec + 12].copy_from_slice(&(code.len() as u32).to_le_bytes()); // virtual size
    out[sec + 12..sec + 16].copy_from_slice(&SECTION_RVA.to_le_bytes()); // virtual address
    out[sec + 16..sec + 20].copy_from_slice(&(code.len() as u32).to_le_bytes()); // raw size
    out[sec + 20..sec + 24].copy_from_slice(&(HEADERS_SIZE as u32).to_le_bytes()); // raw pointer
    out[sec + 36..sec + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes()); // code|exec|read

    // Section data
    out[HEADERS_SIZE..HEADERS_SIZE + code.len()].copy_from_slice(code);
    out
}

#[test]
fn test_pe_parse_and_scan() {
    // movss [rdx-2C], xmm1 ; jmp ; mov rcx, -1
    let code: Vec<u8> = vec![
        0xF3, 0x0F, 0x11, 0x4A, 0xD4, 0xEB, 0x71, 0x48, 0xB9, 0xFF, 0xFF, 0xFF, 0xFF,
    ];
    let bytes = build_pe(&code);
    let pe = PeImage::from_bytes(bytes).expect("PE should parse");

    assert_eq!(pe.image_base, 0x140000000);
    assert_eq!(pe.sections.len(), 1);
    assert_eq!(pe.sections[0].name, ".text");
    assert!(pe.sections[0].is_executable());

    let sig = Signature::parse("F3 0F 11 4A D4").unwrap();
    let hits = scan_pe_file(&pe, &sig, true).expect("scan should succeed");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0], 0x400);
    // file offset 0x400 -> RVA 0x1000
    assert_eq!(pe.file_offset_to_rva(hits[0]), Some(0x1000));
}

#[test]
fn test_drift_repair_finds_corrected_pattern() {
    // The real site: F3 0F 11 4A D4
    let code: Vec<u8> = vec![0xF3, 0x0F, 0x11, 0x4A, 0xD4, 0x90, 0x90];
    let bytes = build_pe(&code);
    let pe = PeImage::from_bytes(bytes).unwrap();

    // A drifted pattern: byte 0 and byte 3 changed (09 C2 -> 0B D0 style).
    let drifted = Signature::parse("1F 0F 11 4B D4").unwrap();
    assert!(scan_pe_file(&pe, &drifted, true).unwrap().is_empty());

    let candidates = find_drifted_in_file(&pe, &drifted, 2, true).unwrap();
    assert_eq!(candidates.len(), 1, "expected one drift candidate");

    let cand = &candidates[0];
    assert_eq!(cand.address, 0x400);
    // The suggestion is the *real* bytes at the site.
    assert_eq!(cand.suggested_pattern(), "F3 0F 11 4A D4");
    assert_eq!(cand.relaxed_positions.len(), 2);
}

#[test]
fn test_drift_repair_respects_max_relax() {
    let code: Vec<u8> = vec![0xF3, 0x0F, 0x11, 0x4A, 0xD4, 0x90];
    let bytes = build_pe(&code);
    let pe = PeImage::from_bytes(bytes).unwrap();

    // 3 bytes differ -> needs max_relax = 3.
    let drifted = Signature::parse("1F 1F 11 4B D4").unwrap();
    assert!(find_drifted_in_file(&pe, &drifted, 2, true).unwrap().is_empty());
    let found = find_drifted_in_file(&pe, &drifted, 3, true).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].suggested_pattern(), "F3 0F 11 4A D4");
}

#[test]
fn test_portable_pattern_masks_rip_relative() {
    // movss xmm0, [rip+0x11223344]
    let code: Vec<u8> = vec![0xF3, 0x0F, 0x10, 0x05, 0x44, 0x33, 0x22, 0x11];
    let mask = volatile_mask(&code, 0x140001000);
    assert!(!mask.is_empty());
    let portable = portable_pattern(&code, &mask);
    assert_eq!(portable, "F3 0F 10 05 ?? ?? ?? ??");
}

#[test]
fn test_portable_pattern_keeps_stable_code() {
    // movss [rdx-2C], xmm1
    let code: Vec<u8> = vec![0xF3, 0x0F, 0x11, 0x4A, 0xD4];
    let mask = volatile_mask(&code, 0x140001000);
    assert!(mask.is_empty());
    assert_eq!(portable_pattern(&code, &mask), "F3 0F 11 4A D4");
}
