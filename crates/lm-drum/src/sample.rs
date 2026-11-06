//! Sample upload for the Behringer LM DRUM.
//!
//! Clean-room implementation from the documented wire contract in
//! `behringer-synthribe/devices/LM-DRUM/PROTOCOL-NOTES.md` (measured on
//! firmware 1.2.2, 2026-11-06). This module does not contain vendor code.
//!
//! One sample is a `0x78 0x10` header packet followed by one or more
//! `0x78 0x11` data packets, each carrying 7-in-8 packed bytes and each
//! acknowledged with `01 00 00`.

/// LM DRUM native transfer rate.
pub const SAMPLE_RATE: u32 = 24_000;
/// Transfer container width (16-bit PCM words).
pub const SAMPLE_WIDTH: u16 = 16;
/// PCM bytes per data packet used by the vendor app.
pub const CHUNK_BYTES: usize = 2048;
/// `config.json` per-sample limit.
pub const MAX_SAMPLE_BYTES: usize = 1_048_576;

const MODEL: [u8; 8] = [0xF0, 0x00, 0x20, 0x32, 0x00, 0x01, 0x28, 0x00];

/// Additive checksum used in the upload header.
///
/// Measured from two known-good vendor uploads of the same PCM:
/// `sum(PCM bytes) + sum(16-byte name) + rate + width`, wrapping at 2^32.
pub fn checksum(pcm: &[u8], name: &str, rate: u32, width: u16) -> Result<u32, String> {
    let name = name16(name)?;
    Ok(checksum_parts(
        pcm.iter().map(|&b| u32::from(b)).fold(0u32, u32::wrapping_add),
        &name,
        rate,
        width,
    ))
}

fn checksum_parts(pcm_sum: u32, name: &[u8; 16], rate: u32, width: u16) -> u32 {
    let name_sum = name.iter().fold(0u32, |acc, &b| acc.wrapping_add(u32::from(b)));
    pcm_sum
        .wrapping_add(name_sum)
        .wrapping_add(rate)
        .wrapping_add(u32::from(width))
}

fn name16(name: &str) -> Result<[u8; 16], String> {
    if name.is_empty() {
        return Err("sample name is empty".into());
    }
    if !name.is_ascii() {
        return Err("sample name must be ASCII".into());
    }
    let bytes = name.as_bytes();
    if bytes.len() > 16 {
        return Err(format!("sample name is {} bytes; maximum is 16", bytes.len()));
    }
    let mut out = [0u8; 16];
    out[..bytes.len()].copy_from_slice(bytes);
    Ok(out)
}

/// Build the 32-byte upload header, without SysEx framing.
pub fn header(pcm: &[u8], name: &str, rate: u32, width: u16) -> Result<[u8; 32], String> {
    if pcm.len() % 2 != 0 {
        return Err("PCM length must be a multiple of 2 (16-bit words)".into());
    }
    if pcm.len() > MAX_SAMPLE_BYTES {
        return Err(format!(
            "sample is {} bytes; the per-sample limit is {}",
            pcm.len(),
            MAX_SAMPLE_BYTES
        ));
    }
    let name = name16(name)?;
    let sum = checksum_parts(
        pcm.iter().map(|&b| u32::from(b)).fold(0u32, u32::wrapping_add),
        &name,
        rate,
        width,
    );
    let mut out = [0u8; 32];
    out[0..4].copy_from_slice(&(pcm.len() as u32).to_le_bytes());
    out[4..8].copy_from_slice(&sum.to_le_bytes());
    out[8..12].copy_from_slice(&rate.to_le_bytes());
    out[12..14].copy_from_slice(&width.to_le_bytes());
    out[16..32].copy_from_slice(&name);
    Ok(out)
}

/// 7-in-8 packing used by the vendor app: one MSB byte, then seven low bytes.
/// The final group is padded with zero bytes.
pub fn pack_7in8(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len().div_ceil(7) * 8);
    for block in data.chunks(7) {
        let mut msb = 0u8;
        for (i, &b) in block.iter().enumerate() {
            if b & 0x80 != 0 {
                msb |= 1 << i;
            }
        }
        out.push(msb);
        out.extend(block.iter().map(|&b| b & 0x7F));
        out.extend(std::iter::repeat_n(0u8, 7 - block.len()));
    }
    out
}

fn frame(opcode: [u8; 3], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(MODEL.len() + 1 + 3 + body.len() + 1);
    out.extend_from_slice(&MODEL);
    out.extend_from_slice(&[0x78, opcode[0], opcode[1], opcode[2]]);
    out.extend_from_slice(&pack_7in8(body));
    out.push(0xF7);
    out
}

/// Build every message for one sample: header first, then data chunks.
pub fn frames(
    pcm: &[u8],
    name: &str,
    rate: u32,
    width: u16,
    chunk_bytes: usize,
) -> Result<Vec<Vec<u8>>, String> {
    if chunk_bytes == 0 {
        return Err("chunk size must be positive".into());
    }
    let header = header(pcm, name, rate, width)?;
    let mut out = vec![frame([0x10, 0x00, 0x00], &header)];
    for block in pcm.chunks(chunk_bytes) {
        out.push(frame([0x11, 0x00, 0x00], block));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_checksums_match_both_names() {
        // Same PCM sum as the captured 54,534-byte Windows upload.
        let pcm_sum = 0x3BE365;
        let a = checksum_parts(pcm_sum, &name16("HANDCLAP909-SHOT").unwrap(), 24_000, 16);
        let b = checksum_parts(pcm_sum, &name16("CLAP").unwrap(), 24_000, 16);
        assert_eq!(a, 0x3C457D);
        assert_eq!(b, 0x3C4255);
    }

    #[test]
    fn header_layout_and_checksum() {
        let pcm = [0u8; 4];
        let h = header(&pcm, "TEST", 24_000, 16).unwrap();
        assert_eq!(u32::from_le_bytes(h[0..4].try_into().unwrap()), 4);
        assert_eq!(u32::from_le_bytes(h[8..12].try_into().unwrap()), 24_000);
        assert_eq!(u16::from_le_bytes(h[12..14].try_into().unwrap()), 16);
        assert_eq!(&h[16..20], b"TEST");
        let expected = checksum(&pcm, "TEST", 24_000, 16).unwrap();
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), expected);
    }

    #[test]
    fn frames_use_documented_sizes() {
        let pcm = vec![0u8; CHUNK_BYTES];
        let f = frames(&pcm, "TEST", 24_000, 16, CHUNK_BYTES).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].len(), 8 + 4 + 40 + 1); // 53
        assert_eq!(f[1].len(), 8 + 4 + 2344 + 1); // 2357
        assert_eq!(&f[0][8..12], &[0x78, 0x10, 0x00, 0x00]);
        assert_eq!(&f[1][8..12], &[0x78, 0x11, 0x00, 0x00]);
        assert_eq!(*f[0].last().unwrap(), 0xF7);
    }

    #[test]
    fn reject_long_or_non_ascii_names() {
        assert!(name16("12345678901234567").is_err());
        assert!(name16("é").is_err());
    }
}
