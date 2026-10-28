//! POLY D SysEx framing, parsing and small helpers.
//!
//! The wire format is the one printed in the POLY D Quick Start Guide,
//! "POLY D System Exclusive Data Sheet" (p. 82-85), and summarised in
//! `devices/POLY-D/MIDI-IMPLEMENTATION.md`:
//!
//! ```text
//! F0 00 20 32 00 01 0C DID b b s D0..Dn F7
//! │  │        │           │   │ │ │   │
//! │  │        │           │   │ │ │   └ payload (7-bit clean)
//! │  │        │           │   │ │ └──── sub-packet, only used by packet 0x0E
//! │  │        │           │   │ └────── packet number (config.json "pkt")
//! │  │        │           │   └──────── device ID (0x00 matches the hardware)
//! │  │        │           └──────────── model ID 00 01 0C = POLY D
//! │  │        └──────────────────────── Behringer / Music Tribe manufacturer ID
//! │  └───────────────────────────────── SysEx start
//! └──────────────────────────────────── SysEx end
//! ```
//!
//! The alternate [`Framing::SynthTribeLong`] form inserts the SynthTribe
//! long-envelope write opcode `0x78` before the packet number. That framing is
//! what the SynthTribe binary uses for `PKT78` writes on protocol families 3
//! and above. It is *not* the framing printed in the QSG, so [`Framing::Qsg`]
//! is the default.

use std::fmt::Write as _;

/// Behringer / Music Tribe manufacturer ID.
pub const MANUFACTURER: [u8; 3] = [0x00, 0x20, 0x32];
/// POLY D model ID.
pub const MODEL: [u8; 3] = [0x00, 0x01, 0x0C];
/// SYNTHTRIBE protocol family. Families >= 3 use the long envelope
/// (device-ID byte after the model ID).
pub const FAMILY: u8 = 3;
/// Shipped hardware device ID.
pub const DEFAULT_DEVICE_ID: u8 = 0x00;
/// Shortest valid message: F0 + 3 + 3 + 1 + (1 body byte) + F7.
pub const MIN_MESSAGE_LEN: usize = 10;

/// Packet numbers (SynthTribe `config.json` calls this field `pkt`).
pub mod packet {
    pub const CHANNEL: u8 = 0x0E;
    pub const TRANSPOSE: u8 = 0x0F;
    pub const VELOCITY: u8 = 0x10;
    pub const PITCH_BEND: u8 = 0x11;
    pub const KEY_PRIORITY: u8 = 0x12;
    pub const MULTI_TRIGGER: u8 = 0x14;
    pub const MOD_CURVE: u8 = 0x15;
    pub const NOTE_ZERO_CV: u8 = 0x16;
    pub const CLOCK_OUT: u8 = 0x17;
    pub const CLOCK_POLARITY: u8 = 0x19;
    pub const CLOCK_RATE: u8 = 0x1A;
    pub const CLOCK_SOURCE: u8 = 0x1B;
    pub const ACCENT_THRESHOLD: u8 = 0x1C;
    pub const SEQ_AUTO_SYNC: u8 = 0x1D;
    pub const ASSIGN: u8 = 0x1F;
    pub const MOD_WHEEL_RANGE: u8 = 0x20;
    pub const OUT_MOD_WHEEL: u8 = 0x21;
    pub const OUT_PITCH_WHEEL: u8 = 0x22;
    pub const OUT_KEYBOARD: u8 = 0x23;
    pub const OUT_AFTERTOUCH: u8 = 0x24;
    pub const OUT_SEQUENCER: u8 = 0x25;
    pub const OUT_ARP: u8 = 0x26;
    pub const LOCAL_KEYBOARD: u8 = 0x2F;
    pub const RESTORE_FACTORY: u8 = 0x7D;
}

/// Opcodes (second body byte, after the device ID) from the platform protocol
/// table. Only the ones this app sends are named.
pub mod op {
    /// Device replies arrive in this container.
    pub const DATA: u8 = 0x0C;
    /// Bootloader acknowledgement.
    pub const ACK: u8 = 0x01;
    /// DFU operation-mode query.
    pub const QUERY_MODE: u8 = 0x02;
    /// Reply to [`QUERY_MODE`].
    pub const RETURN_MODE: u8 = 0x03;
    /// Firmware version query.
    pub const FIRMWARE: u8 = 0x08;
    /// Reply to [`FIRMWARE`].
    pub const RETURN_FIRMWARE: u8 = 0x09;
    /// `ReadEntireParameters`. The device answers with a [`SETTINGS_BLOCK`].
    pub const READ_ENTIRE: u8 = 0x75;
    /// Reply container for [`READ_ENTIRE`]: the 25-byte global block.
    pub const SETTINGS_BLOCK: u8 = 0x76;
    /// LongPacketRead, `77 <address> <selector>`. The POLY D rejects the two
    /// selectors SynthTribe sends with ACK code 5 on firmware 1.1.3, so the app
    /// does not send this opcode.
    pub const READ: u8 = 0x77;
    /// LongPacketWrite (SynthTribe envelope; see [`Framing`]).
    pub const WRITE: u8 = 0x78;
}

/// How a parameter write is framed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// `F0 00 20 32 00 01 0C DID <packet> <payload> F7` — the form printed in
    /// the POLY D QSG data sheet.
    Qsg,
    /// `F0 00 20 32 00 01 0C DID 78 <packet> <payload> F7` — SynthTribe
    /// `PKT78` long envelope. Unverified on this device.
    SynthTribeLong,
}

impl Framing {
    pub fn label(self) -> &'static str {
        match self {
            Framing::Qsg => "QSG (packet first)",
            Framing::SynthTribeLong => "SynthTribe (78 + packet)",
        }
    }

    /// Body leading bytes before the packet number.
    fn lead(self) -> &'static [u8] {
        match self {
            Framing::Qsg => &[],
            Framing::SynthTribeLong => &[op::WRITE],
        }
    }
}

fn clamp7(value: u8) -> u8 {
    value & 0x7F
}

/// Wrap an arbitrary 7-bit body in the POLY D envelope.
pub fn envelope(device_id: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(MIN_MESSAGE_LEN + body.len());
    out.push(0xF0);
    out.extend_from_slice(&MANUFACTURER);
    out.extend_from_slice(&MODEL);
    out.push(clamp7(device_id));
    out.extend(body.iter().copied().map(clamp7));
    out.push(0xF7);
    out
}

/// Build a parameter write. `payload` is clamped to 7 bits.
pub fn param(framing: Framing, device_id: u8, packet: u8, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(2 + payload.len());
    body.extend_from_slice(framing.lead());
    body.push(clamp7(packet));
    body.extend(payload.iter().copied().map(clamp7));
    envelope(device_id, &body)
}

/// Build a single-byte parameter write.
pub fn param_u8(framing: Framing, device_id: u8, packet: u8, value: u8) -> Vec<u8> {
    param(framing, device_id, packet, &[value])
}

/// Envelope for a message that starts with an opcode instead of a packet number.
pub fn op_message(device_id: u8, opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(1 + payload.len());
    body.push(clamp7(opcode));
    body.extend(payload.iter().copied().map(clamp7));
    envelope(device_id, &body)
}

/// `ReadEntireParameters` (opcode 0x75).
pub fn read_entire(device_id: u8) -> Vec<u8> {
    op_message(device_id, op::READ_ENTIRE, &[])
}

/// DFU operation-mode query.
pub fn query_mode(device_id: u8) -> Vec<u8> {
    op_message(device_id, op::QUERY_MODE, &[])
}

/// Firmware version query.
pub fn firmware_version(device_id: u8) -> Vec<u8> {
    op_message(device_id, op::FIRMWARE, &[0x00])
}

/// Restore factory settings (packet `0x7D`).
pub fn restore_factory(device_id: u8) -> Vec<u8> {
    op_message(device_id, packet::RESTORE_FACTORY, &[])
}

/// A received message with the envelope stripped.
#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    pub device_id: u8,
    pub body: &'a [u8],
}

/// Does this byte slice carry the POLY D envelope?
pub fn matches_model(msg: &[u8]) -> bool {
    msg.len() >= MIN_MESSAGE_LEN
        && msg[0] == 0xF0
        && msg[1..4] == MANUFACTURER
        && msg[4..7] == MODEL
        && msg[msg.len() - 1] == 0xF7
}

/// Parse a received message. Returns `None` for anything that is not a
/// well-formed POLY D SysEx message.
pub fn parse(msg: &[u8]) -> Option<Message<'_>> {
    if !matches_model(msg) {
        return None;
    }
    Some(Message {
        device_id: msg[7],
        body: &msg[8..msg.len() - 1],
    })
}

/// Uppercase hex, space separated, as printed in the protocol notes.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{byte:02X}");
    }
    out
}

/// Parse space/comma separated hex, with or without `0x` prefixes and pairs
/// such as `F0`, `0xF0` or `F0F7`.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    // Tokenise: keep hex digits, drop separators and `0x` prefixes.
    let mut digits = String::new();
    for ch in text.chars() {
        if ch.is_ascii_hexdigit() {
            digits.push(ch);
        } else if ch == 'x' || ch == 'X' {
            // `0x` prefix: the leading 0 is already a digit, drop it.
            if digits.ends_with('0') {
                digits.pop();
            }
        } else if ch.is_ascii_whitespace() || ch == ',' || ch == ';' || ch == ':' || ch == '-' {
            continue;
        } else {
            return Err(format!("unexpected character {ch:?} in hex input"));
        }
    }
    if digits.is_empty() {
        return Err("empty hex input".to_string());
    }
    if !digits.len().is_multiple_of(2) {
        return Err("odd number of hex digits".to_string());
    }
    let mut bytes = Vec::with_capacity(digits.len() / 2);
    for pair in digits.as_bytes().chunks(2) {
        // Both nibbles were validated by `is_ascii_hexdigit` above.
        let hi = hex_value(pair[0]).ok_or("invalid hex digit")?;
        let lo = hex_value(pair[1]).ok_or("invalid hex digit")?;
        bytes.push((hi << 4) | lo);
    }
    Ok(bytes)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qsg(bytes: &[u8]) -> Vec<u8> {
        param(Framing::Qsg, DEFAULT_DEVICE_ID, bytes[0], &bytes[1..])
    }

    #[test]
    fn channel_matches_qsg_example() {
        // QSG: "Set MIDI channel" example, TX 3 / RX 4.
        assert_eq!(
            hex(&param(
                Framing::Qsg,
                DEFAULT_DEVICE_ID,
                packet::CHANNEL,
                &[0x01, 3, 4]
            )),
            "F0 00 20 32 00 01 0C 00 0E 01 03 04 F7"
        );
    }

    #[test]
    fn qsg_examples_are_byte_exact() {
        let cases: &[(&[u8], u8)] = &[
            (&[0x0F, 24], 0),
            (&[0x10, 0x00, 0x30, 0x00], 0),
            (&[0x11, 0x08, 0x00], 0),
            (&[0x12, 0x01], 0),
            (&[0x14, 0x01, 0x00], 0),
            (&[0x15, 0x02], 0),
            (&[0x16, 0x3C], 0),
            (&[0x17, 0x00], 0),
            (&[0x19, 0x00], 0),
            (&[0x1A, 0x02], 0),
            (&[0x1B, 0x02], 0),
            (&[0x20, 0x02], 0),
            (&[0x21, 0x00], 0),
            (&[0x22, 0x01], 0),
            (&[0x23, 0x02], 0),
            (&[0x24, 0x03], 0),
            (&[0x25, 0x02], 0),
            (&[0x26, 0x01], 0),
        ];
        for (bytes, _) in cases {
            let expected = format!("F0 00 20 32 00 01 0C 00 {} F7", hex(&bytes[..]));
            assert_eq!(hex(&qsg(&bytes[..])), expected, "packet {:02X}", bytes[0]);
        }
        assert_eq!(
            hex(&restore_factory(DEFAULT_DEVICE_ID)),
            "F0 00 20 32 00 01 0C 00 7D F7"
        );
    }

    #[test]
    fn synthtribe_framing_inserts_write_opcode() {
        assert_eq!(
            hex(&param_u8(
                Framing::SynthTribeLong,
                DEFAULT_DEVICE_ID,
                packet::LOCAL_KEYBOARD,
                1
            )),
            "F0 00 20 32 00 01 0C 00 78 2F 01 F7"
        );
    }

    #[test]
    fn read_requests_match_protocol_notes() {
        assert_eq!(
            hex(&read_entire(DEFAULT_DEVICE_ID)),
            "F0 00 20 32 00 01 0C 00 75 F7"
        );
        assert_eq!(
            hex(&query_mode(DEFAULT_DEVICE_ID)),
            "F0 00 20 32 00 01 0C 00 02 F7"
        );
    }

    #[test]
    fn parse_round_trip() {
        let bytes = param_u8(Framing::Qsg, 0x07, packet::KEY_PRIORITY, 2);
        let msg = parse(&bytes).expect("valid message");
        assert_eq!(msg.device_id, 0x07);
        assert_eq!(msg.body, &[0x12, 0x02]);
        assert!(parse(&bytes[..bytes.len() - 1]).is_none());
        assert!(parse(&[0xF0, 0xF7]).is_none());
        assert!(parse(b"not sysex at all.......").is_none());
    }

    #[test]
    fn payload_is_clamped_to_7_bits() {
        let bytes = param(Framing::Qsg, 0xFF, packet::KEY_PRIORITY, &[0x80, 0xFF]);
        assert_eq!(bytes[7], 0x7F, "device ID is clamped to 7 bits");
        assert_eq!(&bytes[8..], &[0x12, 0x00, 0x7F, 0xF7]);
        // Everything between the F0 start byte and the F7 end byte must be 7-bit.
        assert!(bytes[1..bytes.len() - 1].iter().all(|b| *b < 0x80));
    }

    #[test]
    fn parse_hex_accepts_common_forms() {
        let expected = vec![0xF0, 0x00, 0x20, 0x32];
        assert_eq!(parse_hex("F0 00 20 32").unwrap(), expected);
        assert_eq!(parse_hex("f0002032").unwrap(), expected);
        assert_eq!(parse_hex("0xF0, 0x00, 0x20, 0x32").unwrap(), expected);
        assert_eq!(parse_hex("F0:00:20:32").unwrap(), expected);
        assert!(parse_hex("F0 0").is_err());
        assert!(parse_hex("").is_err());
        assert!(parse_hex("zz").is_err());
    }
}
