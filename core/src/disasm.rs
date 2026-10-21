//! Wildcard suggestion: find the byte positions inside a machine-code
//! pattern that are *volatile* across game builds, so signatures can be
//! made relocation/ASLR tolerant.
//!
//! x86-64 patterns drift mostly because of:
//! - RIP-relative displacements (`movss xmm0, [rip+0x1234]`) — the
//!   displacement changes whenever the surrounding code moves.
//! - `call`/`jmp`/`jcc` relative operands (rel8/rel32) — target offsets
//!   change with any edit before them.
//! - Absolute immediates that look like in-module pointers.
//!
//! `iced-x86` decodes the instruction so we can mask exactly those bytes
//! instead of guessing.

use iced_x86::{Decoder, DecoderOptions, FlowControl, Instruction, OpKind};

/// Byte positions inside `code` that should be wildcards for portability.
#[derive(Debug, Clone, Default)]
pub struct VolatileMask {
    /// Positions (indices into `code`) to wildcard.
    pub positions: Vec<usize>,
    /// Human-readable reasons, e.g. "rip-relative displacement".
    pub reasons: Vec<String>,
}

impl VolatileMask {
    /// True when at least one byte should be masked.
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    fn add(&mut self, positions: impl IntoIterator<Item = usize>, reason: &str) {
        for p in positions {
            if !self.positions.contains(&p) {
                self.positions.push(p);
                self.reasons.push(reason.to_string());
            }
        }
    }
}

/// Analyze machine code and report volatile byte positions.
///
/// `code` is the byte window to analyze (usually the whole pattern),
/// `ip` is the virtual address the code is loaded at (needed to resolve
/// RIP-relative operands).
pub fn volatile_mask(code: &[u8], ip: u64) -> VolatileMask {
    let mut mask = VolatileMask::default();
    if code.is_empty() {
        return mask;
    }

    let options = DecoderOptions::NONE;
    let mut decoder = Decoder::with_ip(64, code, ip, options);

    while decoder.can_decode() {
        let instr: Instruction = decoder.decode();
        if instr.is_invalid() {
            break;
        }
        let start = instr.ip() as usize - ip as usize;
        let len = instr.len();
        if start + len > code.len() {
            break;
        }

        // RIP-relative memory operand: the trailing disp32 is volatile.
        if instr.is_ip_rel_memory_operand() && len >= 4 {
            mask.add(
                start + len - 4..start + len,
                "rip-relative displacement",
            );
        }

        // Relative branches: rel8 (1 byte) / rel32 (4 bytes) at the end.
        // The operand kind determines the encoded size; `call` reports
        // FlowControl::Call, jumps report Unconditional/ConditionalBranch.
        let rel_bytes = match instr.op0_kind() {
            OpKind::NearBranch16 => Some(2usize),
            OpKind::NearBranch32 | OpKind::NearBranch64 => Some(4),
            OpKind::Immediate8
                if matches!(
                    instr.flow_control(),
                    FlowControl::UnconditionalBranch | FlowControl::ConditionalBranch
                ) =>
            {
                Some(1)
            }
            _ => None,
        };
        if let Some(n) = rel_bytes {
            if n <= len {
                mask.add(start + len - n..start + len, "relative branch operand");
            }
        }

        // Absolute immediates that look like in-module pointers.
        for i in 0..instr.op_count() {
            match instr.op_kind(i) {
                OpKind::Immediate32 => {
                    let v = instr.immediate(i) as u32;
                    if (0x1000..0x1000_0000).contains(&v) {
                        mask.add(imm_positions(start, len, 4), "immediate looks like an address");
                    }
                }
                OpKind::Immediate64 => {
                    let v = instr.immediate(i);
                    if (0x1000..0x1000_0000).contains(&v) {
                        mask.add(imm_positions(start, len, 8), "immediate looks like an address");
                    }
                }
                _ => {}
            }
        }
    }

    mask.positions.sort_unstable();
    mask
}

/// Byte positions of a trailing immediate of `size` bytes.
fn imm_positions(start: usize, len: usize, size: usize) -> std::ops::Range<usize> {
    let size = size.min(len);
    (start + len - size)..(start + len)
}

/// Build a portable pattern string from bytes + a volatile mask.
///
/// `??` replaces masked positions.
pub fn portable_pattern(code: &[u8], mask: &VolatileMask) -> String {
    code.iter()
        .enumerate()
        .map(|(i, b)| {
            if mask.positions.contains(&i) {
                "??".to_string()
            } else {
                format!("{:02X}", b)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Heuristic (no decoder): mark bytes that look like in-module addresses.
///
/// Used as a fallback when decoding is impossible. Any 4-byte little-endian
/// window in the 0x1000..0x10000000 range (typical RVA range) is treated
/// as a pointer and masked.
pub fn heuristic_mask(code: &[u8]) -> VolatileMask {
    let mut mask = VolatileMask::default();
    if code.len() < 4 {
        return mask;
    }
    for i in 0..=code.len() - 4 {
        let v = u32::from_le_bytes([code[i], code[i + 1], code[i + 2], code[i + 3]]);
        if (0x1000..0x1000_0000).contains(&v) {
            mask.add(
                i..i + 4,
                &format!("looks like in-module address (0x{:08X})", v),
            );
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rip_relative_is_masked() {
        // movss xmm0, [rip+0x11223344]  => F3 0F 10 05 44 33 22 11
        let code = [0xF3u8, 0x0F, 0x10, 0x05, 0x44, 0x33, 0x22, 0x11];
        let mask = volatile_mask(&code, 0x140001000);
        assert!(!mask.is_empty(), "expected rip-relative displacement to be masked");
        assert!(mask.positions.contains(&4));
        assert!(mask.positions.contains(&7));
    }

    #[test]
    fn test_relative_call_is_masked() {
        // call +0x11223344 => E8 44 33 22 11
        let code = [0xE8u8, 0x44, 0x33, 0x22, 0x11];
        let mask = volatile_mask(&code, 0x140001000);
        assert!(!mask.is_empty());
        assert!(mask.positions.contains(&1));
        assert!(mask.positions.contains(&4));
    }

    #[test]
    fn test_stable_code_unmasked() {
        // movss [rdx-2C], xmm1 => F3 0F 11 4A D4
        let code = [0xF3u8, 0x0F, 0x11, 0x4A, 0xD4];
        let mask = volatile_mask(&code, 0x140001000);
        assert!(mask.is_empty(), "expected no volatile bytes, got {:?}", mask.positions);
    }

    #[test]
    fn test_portable_pattern() {
        let code = [0xF3u8, 0x0F, 0x10, 0x05, 0x44, 0x33, 0x22, 0x11];
        let mask = volatile_mask(&code, 0x140001000);
        let pat = portable_pattern(&code, &mask);
        assert!(pat.starts_with("F3 0F 10 05 ??"), "got {}", pat);
    }

    #[test]
    fn test_heuristic_mask_finds_pointer() {
        // mov rax, [0x140112233]
        let code = [0x48u8, 0x8B, 0x05, 0x33, 0x22, 0x11, 0x40, 0x01];
        let mask = heuristic_mask(&code);
        assert!(!mask.is_empty());
    }
}
