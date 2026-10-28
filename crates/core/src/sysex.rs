//! Beam-format SysEx framing shared by the Behringer / Music Tribe family.
//!
//! ```text
//! F0 00 20 32 m1 m2 m3 [DID] <packet> [<sub-packet>] <payload…> F7
//! │  │        │        │     │
//! │  │        │        │     └ packet number
//! │  │        │        └─────── device ID, present only for protocol families >= 3
//! │  │        └──────────────── model ID bytes (per device)
//! │  └───────────────────────── Behringer / Music Tribe manufacturer ID
//! └──────────────────────────── SysEx start
//! ```
//!
//! Protocol families 0 and 1 omit the device-ID byte. Family 3 and above insert
//! it; the POLY D (family 3) is the reference for that layout.

use std::fmt::Write as _;

/// Behringer / Music Tribe manufacturer ID.
pub const MANUFACTURER: [u8; 3] = [0x00, 0x20, 0x32];
/// Shortest valid message with a one-byte body and a device ID.
pub const MIN_MESSAGE_LEN: usize = 10;

/// Opcodes that sit where the packet number normally sits.
pub mod op {
    /// Device replies arrive in this container for some devices.
    pub const DATA: u8 = 0x0C;
    /// Bootloader/command acknowledgement: `01 00 <code>`.
    pub const ACK: u8 = 0x01;
    /// DFU operation-mode query.
    pub const QUERY_MODE: u8 = 0x02;
    /// Reply to [`QUERY_MODE`].
    pub const RETURN_MODE: u8 = 0x03;
    /// Firmware version query.
    pub const FIRMWARE: u8 = 0x08;
    /// Reply to [`FIRMWARE`].
    pub const RETURN_FIRMWARE: u8 = 0x09;
    /// `ReadEntireParameters` on family 3 devices.
    pub const READ_ENTIRE: u8 = 0x75;
    /// Reply container for [`READ_ENTIRE`] on family 3 devices.
    pub const SETTINGS_BLOCK: u8 = 0x76;
    /// LongPacketRead (`77 <address> <selector>`). Rejected by the POLY D
    /// firmware (ACK code 5), so the app does not send it.
    pub const READ: u8 = 0x77;
    /// LongPacketWrite opcode, used by the alternate long-envelope framing.
    pub const WRITE: u8 = 0x78;

    /// ACK payload code for an accepted request.
    pub const ACK_OK: u8 = 0x00;
    /// ACK payload code for an unknown packet, out-of-range value or bad framing.
    pub const ACK_REJECTED: u8 = 0x05;
}

/// Where a write lands: a SysEx packet (optionally with a sub-packet byte) or a
/// MIDI control-change number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Addr {
    Packet(u8),
    PacketSub(u8, u8),
    Cc(u8),
}

impl Addr {
    /// Stable key for de-duplicating repeats. SysEx packets carry their
    /// sub-packet so different sub-indices of one container stay distinct;
    /// CCs live above that range.
    pub fn key(self) -> u32 {
        match self {
            Addr::Packet(packet) | Addr::PacketSub(packet, _) => {
                (u32::from(packet) << 8) | u32::from(self.sub().unwrap_or(0))
            }
            Addr::Cc(control) => 0x1_0000 + u32::from(control),
        }
    }

    pub fn packet(self) -> Option<u8> {
        match self {
            Addr::Packet(packet) | Addr::PacketSub(packet, _) => Some(packet),
            Addr::Cc(_) => None,
        }
    }

    pub fn sub(self) -> Option<u8> {
        match self {
            Addr::Packet(_) | Addr::Cc(_) => None,
            Addr::PacketSub(_, sub) => Some(sub),
        }
    }

    pub fn cc(self) -> Option<u8> {
        match self {
            Addr::Cc(control) => Some(control),
            _ => None,
        }
    }
}

/// How a parameter write is framed on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteStyle {
    /// `F0 … [DID] <packet> <payload> F7` — the form printed in the POLY D QSG.
    PacketDirect,
    /// `F0 … [DID] 78 <packet> <payload> F7` — long envelope used by the vendor
    /// editor. Some firmware rejects it (POLY D 1.1.3 answers ACK code 5).
    LongEnvelope,
}

impl WriteStyle {
    pub fn label(self) -> &'static str {
        match self {
            WriteStyle::PacketDirect => "packet direct (QSG)",
            WriteStyle::LongEnvelope => "long envelope (0x78)",
        }
    }

    /// Bytes inserted before the packet number.
    fn lead(self) -> &'static [u8] {
        match self {
            WriteStyle::PacketDirect => &[],
            WriteStyle::LongEnvelope => &[op::WRITE],
        }
    }
}

fn clamp7(value: u8) -> u8 {
    value & 0x7F
}

/// Wrap a 7-bit body in the device envelope.
pub fn envelope(model: [u8; 3], family: u8, device_id: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(MIN_MESSAGE_LEN + body.len());
    out.push(0xF0);
    out.extend_from_slice(&MANUFACTURER);
    out.extend_from_slice(&model);
    if family >= 3 {
        out.push(clamp7(device_id));
    }
    out.extend(body.iter().copied().map(clamp7));
    out.push(0xF7);
    out
}

/// Build a parameter message from a packet number and payload.
pub fn message(
    model: [u8; 3],
    family: u8,
    style: WriteStyle,
    device_id: u8,
    packet: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut body = Vec::with_capacity(2 + payload.len());
    body.extend_from_slice(style.lead());
    body.push(clamp7(packet));
    body.extend(payload.iter().copied().map(clamp7));
    envelope(model, family, device_id, &body)
}

/// Build a message whose body starts with an opcode instead of a packet number.
pub fn op_message(
    model: [u8; 3],
    family: u8,
    device_id: u8,
    opcode: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut body = Vec::with_capacity(1 + payload.len());
    body.push(clamp7(opcode));
    body.extend(payload.iter().copied().map(clamp7));
    envelope(model, family, device_id, &body)
}

/// A received message with the envelope stripped.
#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    pub device_id: u8,
    pub body: &'a [u8],
}

/// Does this byte slice carry the envelope for `model`?
pub fn matches(model: [u8; 3], msg: &[u8]) -> bool {
    let header_len = 1 + MANUFACTURER.len() + model.len() + 1;
    msg.len() > header_len
        && msg[0] == 0xF0
        && msg[1..4] == MANUFACTURER
        && msg[4..7] == model
        && msg[msg.len() - 1] == 0xF7
}

/// Parse a received message. Returns `None` for anything that is not a
/// well-formed message for `model`.
pub fn parse(model: [u8; 3], family: u8, msg: &[u8]) -> Option<Message<'_>> {
    if !matches(model, msg) {
        return None;
    }
    let body_start = if family >= 3 { 8 } else { 7 };
    if msg.len() <= body_start + 1 {
        return None;
    }
    Some(Message {
        device_id: if family >= 3 { msg[7] } else { 0 },
        body: &msg[body_start..msg.len() - 1],
    })
}

/// Uppercase hex, space separated.
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

/// Parse space/comma separated hex, with or without `0x` prefixes, `:` or
/// continuous pairs such as `F0F7`.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
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

    const MODEL: [u8; 3] = [0x00, 0x01, 0x0C];
    const FAMILY: u8 = 3;

    #[test]
    fn family_three_envelope_carries_device_id() {
        assert_eq!(
            hex(&message(
                MODEL,
                FAMILY,
                WriteStyle::PacketDirect,
                0x00,
                0x2F,
                &[0x01]
            )),
            "F0 00 20 32 00 01 0C 00 2F 01 F7"
        );
    }

    #[test]
    fn family_one_envelope_omits_device_id() {
        let model = [0x00, 0x01, 0x05];
        assert_eq!(
            hex(&message(
                model,
                1,
                WriteStyle::PacketDirect,
                0x00,
                0x0F,
                &[0x0C]
            )),
            "F0 00 20 32 00 01 05 0F 0C F7"
        );
        let bytes = message(model, 1, WriteStyle::PacketDirect, 0, 0x0F, &[12]);
        let parsed = parse(model, 1, &bytes).unwrap_or_else(|| panic!("valid family 1 message"));
        assert_eq!(parsed.device_id, 0);
        assert_eq!(parsed.body, &[0x0F, 0x0C]);
    }

    #[test]
    fn long_envelope_inserts_write_opcode() {
        assert_eq!(
            hex(&message(
                MODEL,
                FAMILY,
                WriteStyle::LongEnvelope,
                0x00,
                0x11,
                &[0x0C, 0x00]
            )),
            "F0 00 20 32 00 01 0C 00 78 11 0C 00 F7"
        );
    }

    #[test]
    fn parse_round_trip() {
        let bytes = message(MODEL, FAMILY, WriteStyle::PacketDirect, 0x07, 0x12, &[0x02]);
        let msg = parse(MODEL, FAMILY, &bytes).unwrap_or_else(|| panic!("valid message"));
        assert_eq!(msg.device_id, 0x07);
        assert_eq!(msg.body, &[0x12, 0x02]);
        assert!(parse(MODEL, FAMILY, &bytes[..bytes.len() - 1]).is_none());
        assert!(parse(MODEL, FAMILY, &[0xF0, 0xF7]).is_none());
        assert!(parse([0x00, 0x01, 0x28], FAMILY, &bytes).is_none());
    }

    #[test]
    fn payload_is_clamped_to_7_bits() {
        let bytes = message(
            MODEL,
            FAMILY,
            WriteStyle::PacketDirect,
            0xFF,
            0x12,
            &[0x80, 0xFF],
        );
        assert_eq!(bytes[7], 0x7F, "device ID is clamped to 7 bits");
        assert_eq!(&bytes[8..], &[0x12, 0x00, 0x7F, 0xF7]);
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
