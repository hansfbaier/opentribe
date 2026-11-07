//! MD5, because firmware integrity is checked against the MD5 published by the vendor cloud
//! and the workspace has no hashing dependency (the build is offline).
//!
//! Straight RFC 1321: little-endian message length, four register words, 64 rounds. It is
//! used only to compare a downloaded file against a published digest — never as a security
//! primitive.

const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const K: [u32; 64] = [
    0xd76a_a478, 0xe8c7_b756, 0x2420_70db, 0xc1bd_ceee, 0xf57c_0faf, 0x4787_c62a, 0xa830_4613,
    0xfd46_9501, 0x6980_98d8, 0x8b44_f7af, 0xffff_5bb1, 0x895c_d7be, 0x6b90_1122, 0xfd98_7193,
    0xa679_438e, 0x49b4_0821, 0xf61e_2562, 0xc040_b340, 0x265e_5a51, 0xe9b6_c7aa, 0xd62f_105d,
    0x0244_1453, 0xd8a1_e681, 0xe7d3_fbc8, 0x21e1_cde6, 0xc337_07d6, 0xf4d5_0d87, 0x455a_14ed,
    0xa9e3_e905, 0xfcef_a3f8, 0x676f_02d9, 0x8d2a_4c8a, 0xfffa_3942, 0x8771_f681, 0x6d9d_6122,
    0xfde5_380c, 0xa4be_ea44, 0x4bde_cfa9, 0xf6bb_4b60, 0xbebf_bc70, 0x289b_7ec6, 0xeaa1_27fa,
    0xd4ef_3085, 0x0488_1d05, 0xd9d4_d039, 0xe6db_99e5, 0x1fa2_7cf8, 0xc4ac_5665, 0xf429_2244,
    0x432a_ff97, 0xab94_23a7, 0xfc93_a039, 0x655b_59c3, 0x8f0c_cc92, 0xffef_f47d, 0x8584_5dd1,
    0x6fa8_7e4f, 0xfe2c_e6e0, 0xa301_4314, 0x4e08_11a1, 0xf753_7e82, 0xbd3a_f235, 0x2ad7_d2bb,
    0xeb86_d391,
];

/// Digest of `data` as 16 raw bytes.
pub fn digest(data: &[u8]) -> [u8; 16] {
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let bit_len = (data.len() as u64).wrapping_mul(8);

    // 64-byte blocks, then the tail with RFC 1321 padding. `chunks(64)` is used because the
    // remainder is needed; `as_chunks` cannot express that in stable Rust here.
    let mut chunk = Vec::with_capacity(64);
    let whole = data.len() / 64 * 64;
    for block in data[..whole].chunks(64) {
        compress(&mut state, block);
    }

    // Padding: one 0x80 byte, zeros, then the 64-bit little-endian bit length.
    let rest = &data[whole..];
    chunk.extend_from_slice(rest);
    chunk.push(0x80);
    while chunk.len() % 64 != 56 {
        chunk.push(0);
    }
    chunk.extend_from_slice(&bit_len.to_le_bytes());
    for block in chunk.chunks(64) {
        compress(&mut state, block);
    }

    let mut out = [0u8; 16];
    for (index, word) in state.iter().enumerate() {
        out[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

fn compress(state: &mut [u32; 4], block: &[u8]) {
    let mut m = [0u32; 16];
    for (index, word) in m.iter_mut().enumerate() {
        let start = index * 4;
        *word = u32::from_le_bytes([
            block[start],
            block[start + 1],
            block[start + 2],
            block[start + 3],
        ]);
    }

    let [mut a, mut b, mut c, mut d] = *state;
    for i in 0..64 {
        let (f, g) = match i / 16 {
            0 => ((b & c) | (!b & d), i),
            1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            2 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let rotated = a
            .wrapping_add(f)
            .wrapping_add(K[i])
            .wrapping_add(m[g])
            .rotate_left(S[i]);
        a = d;
        d = c;
        c = b;
        b = b.wrapping_add(rotated);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
}

/// Lowercase hex digest, the form the vendor cloud publishes.
pub fn hex_digest(data: &[u8]) -> String {
    let mut out = String::with_capacity(32);
    for byte in digest(data) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Compare a digest string (hex, case-insensitive) with the digest of `data`.
pub fn matches(data: &[u8], expected: &str) -> bool {
    let expected: String = expected
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .flat_map(|c| c.to_lowercase())
        .collect();
    expected.len() == 32 && expected == hex_digest(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc1321_vectors() {
        let cases = [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            ("abcdefghijklmnopqrstuvwxyz", "c3fcd3d76192e4007dfb496cca67e13b"),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
            (
                "The quick brown fox jumps over the lazy dog",
                "9e107d9d372bb6826bd81d3542a419d6",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(hex_digest(input.as_bytes()), expected, "input {input:?}");
        }
    }

    #[test]
    fn block_boundary_lengths() {
        // 55, 56 and 64 bytes exercise the padding edge cases (the 56-byte case needs an
        // extra block because the length no longer fits).
        for len in [55usize, 56, 63, 64, 65, 127, 128] {
            let data = vec![b'x'; len];
            assert_eq!(digest(&data).len(), 16);
            assert_eq!(hex_digest(&data).len(), 32);
        }
        assert_eq!(
            hex_digest(&vec![b'a'; 1_000_000]),
            "7707d6ae4e027c70eea2a935c2296f21"
        );
    }

    #[test]
    fn digest_parameters_compare_leniently() {
        let digest = "d41d8cd98f00b204e9800998ecf8427e";
        assert!(matches(b"", digest));
        assert!(matches(b"", &digest.to_uppercase()));
        assert!(matches(b"", "d41d8cd9 8f00b204 e9800998 ecf8427e"));
        assert!(!matches(b"x", digest));
        assert!(!matches(b"", "d41d8cd9"));
    }
}
