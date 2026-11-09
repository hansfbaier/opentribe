//! PRO-800 patch payload: 7-in-8 packing, the 173-byte record and its name.
//!
//! A program dump is a whole patch: there is no per-parameter write frame. The
//! record is not fixed-width — trailing zero bytes are trimmed by the device —
//! so writes re-send the bytes that were read, with only the edited bytes
//! changed. See `devices/PRO-800/PROTOCOL-NOTES.md` in the research notebook.

use tribe_core::sysex;

/// SysEx model bytes (`00 01 24`).
pub const MODEL: [u8; 3] = [0x00, 0x01, 0x24];
/// Protocol family. Family 5 puts the device ID at index 7 and the opcode at 8.
pub const FAMILY: u8 = 5;
/// Full payload length for a patch that fills every known field (`0x00`-`0xAC`).
pub const PAYLOAD_LEN: usize = 173;
/// Unpacked offset of the NUL-padded name field (`0x96`).
pub const NAME_OFFSET: usize = 0x96;
/// Name field width in bytes.
pub const NAME_LEN: usize = 16;
/// Unpacked length of the global settings block.
pub const GLOBAL_LEN: usize = 40;
/// Highest program address the device accepts.
pub const MAX_SLOT: u16 = 399;

/// Read opcode for one program.
pub const OP_READ: u8 = 0x77;
/// Write/reply opcode for one program (also the global block).
pub const OP_WRITE: u8 = 0x78;
/// Address of the global settings block.
pub const GLOBAL_ADDR: [u8; 2] = [0x7E, 0x03];

const HEADER: [u8; 8] = [
    0xF0,
    sysex::MANUFACTURER[0],
    sysex::MANUFACTURER[1],
    sysex::MANUFACTURER[2],
    MODEL[0],
    MODEL[1],
    MODEL[2],
    0x00,
];

/// One stored program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    /// Program address 0..=399.
    pub index: u16,
    /// Unpacked payload exactly as it arrived (or was built).
    pub bytes: Vec<u8>,
}

impl Patch {
    pub fn new(index: u16, bytes: Vec<u8>) -> Self {
        Self { index, bytes }
    }

    /// The factory "Init Program" captured from B080, re-addressed.
    pub fn init(index: u16) -> Self {
        let mut bytes = patches_from_syx(include_bytes!("../../../verified/pro-800-init.syx"))
            .first()
            .map(|patch| patch.bytes.clone())
            .unwrap_or_else(|| vec![0u8; PAYLOAD_LEN]);
        let mut patch = Self {
            index,
            bytes: std::mem::take(&mut bytes),
        };
        let _ = patch.set_name("Init Program");
        patch
    }

    /// Name as stored at [`NAME_OFFSET`], trimmed at the first NUL and at any
    /// non-printable byte. Empty for a slot that has no name yet.
    pub fn name(&self) -> String {
        let start = NAME_OFFSET.min(self.bytes.len());
        let end = (start + NAME_LEN).min(self.bytes.len());
        let field = &self.bytes[start..end];
        let end = field
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(field.len());
        field[..end]
            .iter()
            .take_while(|byte| (0x20..=0x7E).contains(*byte))
            .map(|byte| *byte as char)
            .collect()
    }

    /// Replace the name. Rejects names longer than [`NAME_LEN`] rather than
    /// truncating them; pads with NUL and extends the payload if it was trimmed
    /// before the name field.
    pub fn set_name(&mut self, name: &str) -> Result<(), String> {
        let bytes = name.as_bytes();
        if bytes.len() > NAME_LEN {
            return Err(format!(
                "name is {} bytes; the field holds {NAME_LEN}",
                bytes.len()
            ));
        }
        if let Some(bad) = bytes.iter().find(|byte| !(0x20..=0x7E).contains(*byte)) {
            return Err(format!("name contains the non-printable byte {bad:#04X}"));
        }
        self.ensure(NAME_OFFSET + NAME_LEN);
        for (index, slot) in self.bytes[NAME_OFFSET..NAME_OFFSET + NAME_LEN]
            .iter_mut()
            .enumerate()
        {
            *slot = bytes.get(index).copied().unwrap_or(0);
        }
        Ok(())
    }

    /// Little-endian 16-bit field; `None` when the trimmed record ends early.
    pub fn u16_at(&self, offset: usize) -> Option<u16> {
        let pair = self.bytes.get(offset..offset + 2)?;
        Some(u16::from_le_bytes([pair[0], pair[1]]))
    }

    pub fn byte_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(offset).copied()
    }

    /// Write a little-endian 16-bit field, extending the record with zeros.
    pub fn set_u16(&mut self, offset: usize, value: u16) {
        self.ensure(offset + 2);
        self.bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    /// Write one byte, extending the record with zeros.
    pub fn set_byte(&mut self, offset: usize, value: u8) {
        self.ensure(offset + 1);
        self.bytes[offset] = value;
    }

    /// Extend the payload with zero bytes so that `offset` is readable.
    pub fn ensure_for(&mut self, len: usize) {
        self.ensure(len);
    }

    fn ensure(&mut self, len: usize) {
        if self.bytes.len() < len {
            self.bytes.resize(len, 0);
        }
    }

    /// Payload packed for the wire (no padding in a partial final group).
    pub fn packed(&self) -> Vec<u8> {
        pack(&self.bytes)
    }

    /// Payload padded out to the full 173-byte layout. A write must carry the
    /// whole record: the verified write tool pads trimmed captures before
    /// sending, and the device stores the padded form.
    pub fn padded(&self) -> Vec<u8> {
        let mut bytes = self.bytes.clone();
        if bytes.len() < PAYLOAD_LEN {
            bytes.resize(PAYLOAD_LEN, 0);
        }
        bytes
    }

    /// Complete write frame for this patch, padded to [`PAYLOAD_LEN`].
    pub fn write_frame(&self) -> Vec<u8> {
        write_frame(self.index, &pack(&self.padded()))
    }

    /// Frame that preserves the record exactly as it was read. Use this for
    /// files on disk; `write_frame` is the form that goes back to the device.
    pub fn record_frame(&self) -> Vec<u8> {
        write_frame(self.index, &self.packed())
    }
}

/// Pack 8-bit bytes into the 7-in-8 form used by the PRO-800.
///
/// Every complete group of seven bytes becomes one MSB bitmap followed by the
/// seven low bytes. A final partial group carries only the bytes that exist —
/// the device omits the zero padding, and re-packing a captured dump without
/// padding reproduces it byte for byte.
pub fn pack(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() / 7 * 8 + 8);
    for group in bytes.chunks(7) {
        let mut msb = 0u8;
        for (index, byte) in group.iter().enumerate() {
            if byte & 0x80 != 0 {
                msb |= 1 << index;
            }
        }
        out.push(msb);
        out.extend(group.iter().map(|byte| byte & 0x7F));
    }
    out
}

/// Unpack 7-in-8 data. A trailing partial group is decoded as far as it goes.
pub fn unpack(packed: &[u8]) -> Vec<u8> {
    sysex::unpack_7in8(packed)
}

/// Address bytes for a program index (`LSB`, `MSB`).
pub fn address(index: u16) -> [u8; 2] {
    [(index & 0x7F) as u8, (index >> 7) as u8]
}

/// Parse an address pair back into a program index.
pub fn index_of(lsb: u8, msb: u8) -> u16 {
    u16::from(lsb & 0x7F) | (u16::from(msb & 0x7F) << 7)
}

/// Program index as stored in the global block: a full byte plus an 8-bit
/// carry, not the 7-bit SysEx address pair.
pub fn global_index(lsb: u8, msb: u8) -> u16 {
    u16::from(lsb) | (u16::from(msb) << 8)
}

/// `F0 00 20 32 00 01 24 00 77 <lsb> <msb> F7`.
pub fn read_frame(index: u16) -> Vec<u8> {
    let [lsb, msb] = address(index);
    frame(OP_READ, &[lsb, msb], &[])
}

/// `F0 00 20 32 00 01 24 00 78 <lsb> <msb> <packed> F7`.
pub fn write_frame(index: u16, packed: &[u8]) -> Vec<u8> {
    let [lsb, msb] = address(index);
    frame(OP_WRITE, &[lsb, msb], packed)
}

/// `F0 00 20 32 00 01 24 00 78 <lsb> <msb> F7` — the frame an empty slot
/// answers with, usable as a record that a backup slot held nothing.
pub fn empty_frame(index: u16) -> Vec<u8> {
    let [lsb, msb] = address(index);
    frame(OP_WRITE, &[lsb, msb], &[])
}

/// `F0 00 20 32 00 01 24 00 77 7E 03 F7`.
pub fn globals_read_frame() -> Vec<u8> {
    frame(OP_READ, &GLOBAL_ADDR, &[])
}

/// `F0 00 20 32 00 01 24 00 78 7E 03 <packed> F7`.
///
/// This is the frame the Swumpf editor sends for a global write. It is not
/// verified against firmware (gate G3 in `docs/spec-pro-800.md`).
pub fn globals_write_frame(bytes: &[u8]) -> Vec<u8> {
    frame(
        OP_WRITE,
        &GLOBAL_ADDR,
        &pack(&bytes[..bytes.len().min(GLOBAL_LEN)]),
    )
}

fn frame(opcode: u8, addr: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER.len() + 1 + addr.len() + payload.len() + 1);
    out.extend_from_slice(&HEADER);
    out.push(opcode);
    out.extend_from_slice(addr);
    out.extend(payload.iter().map(|byte| byte & 0x7F));
    out.push(0xF7);
    out
}

/// A parsed reply frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply<'a> {
    pub opcode: u8,
    pub addr: [u8; 2],
    pub payload: &'a [u8],
}

/// Parse a PRO-800 SysEx frame. Returns `None` for anything that is not one.
pub fn parse_reply(msg: &[u8]) -> Option<Reply<'_>> {
    if msg.len() < 12 || msg.first() != Some(&0xF0) || msg.last() != Some(&0xF7) {
        return None;
    }
    if msg[1..8] != HEADER[1..8] {
        return None;
    }
    Some(Reply {
        opcode: msg[8],
        addr: [msg[9], msg[10]],
        payload: &msg[11..msg.len() - 1],
    })
}

/// Does this frame carry a program or the global block?
pub fn patch_reply(msg: &[u8]) -> Option<(u16, Option<Patch>)> {
    let reply = parse_reply(msg)?;
    if reply.opcode != OP_WRITE {
        return None;
    }
    if reply.addr == GLOBAL_ADDR {
        return None;
    }
    let index = index_of(reply.addr[0], reply.addr[1]);
    if index > MAX_SLOT {
        return None;
    }
    if reply.payload.is_empty() {
        return Some((index, None));
    }
    let bytes = unpack(reply.payload);
    if bytes.is_empty() {
        return None;
    }
    Some((index, Some(Patch::new(index, bytes))))
}

/// Unpacked global block from a reply frame, when it is one.
pub fn global_reply(msg: &[u8]) -> Option<Vec<u8>> {
    let reply = parse_reply(msg)?;
    if reply.opcode != OP_WRITE || reply.addr != GLOBAL_ADDR || reply.payload.is_empty() {
        return None;
    }
    Some(unpack(reply.payload))
}

/// Every complete SysEx frame in `data`.
pub fn split_frames(data: &[u8]) -> Vec<&[u8]> {
    let mut frames = Vec::new();
    let mut start = None;
    for (index, byte) in data.iter().enumerate() {
        match byte {
            0xF0 => start = Some(index),
            0xF7 => {
                if let Some(begin) = start.take() {
                    frames.push(&data[begin..=index]);
                }
            }
            _ => {}
        }
    }
    frames
}

/// Parse every program frame in a `.syx` file, in file order.
pub fn patches_from_syx(data: &[u8]) -> Vec<Patch> {
    split_frames(data)
        .into_iter()
        .filter_map(|frame| patch_reply(frame).and_then(|(_, patch)| patch))
        .collect()
}

/// Read a `.syx` file from disk.
pub fn load_syx(path: &std::path::Path) -> Result<Vec<Patch>, String> {
    let data = std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let patches = patches_from_syx(&data);
    if patches.is_empty() {
        return Err(format!("{}: no PRO-800 program frames", path.display()));
    }
    Ok(patches)
}

/// Do two payloads describe the same program, allowing the device to trim
/// trailing zeros from the record it stores?
pub fn equivalent(left: &[u8], right: &[u8]) -> bool {
    let common = left.len().min(right.len());
    if left[..common] != right[..common] {
        return false;
    }
    left[common..].iter().all(|byte| *byte == 0) && right[common..].iter().all(|byte| *byte == 0)
}

/// Program label used on disk and in the librarian: `A000` … `D099`.
pub fn slot_label(index: u16) -> String {
    let bank = (b'A' + (index / 100).min(3) as u8) as char;
    format!("{bank}{:03}", index % 100)
}

/// Parse `B094`, `b94`, `194` or `194/0xC2` into a program index.
pub fn parse_slot(text: &str) -> Result<u16, String> {
    let text = text.split('/').next().unwrap_or(text).trim();
    if text.is_empty() {
        return Err("empty slot".to_string());
    }
    let first = text.as_bytes()[0];
    if !first.is_ascii_alphabetic() {
        let index: u16 = text
            .parse()
            .map_err(|_| format!("{text:?} is not a slot"))?;
        return if index > MAX_SLOT {
            Err(format!("{text:?}: address is 0-{MAX_SLOT}"))
        } else {
            Ok(index)
        };
    }
    let bank = first.to_ascii_uppercase();
    if !(b'A'..=b'D').contains(&bank) {
        return Err(format!("bank {} is not A-D", bank as char));
    }
    let number: u16 = text[1..]
        .parse()
        .map_err(|_| format!("{text:?} is not a slot"))?;
    if number > 99 {
        return Err(format!("{text:?}: program number is 0-99"));
    }
    Ok(u16::from(bank - b'A') * 100 + number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../verified")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
    }

    #[test]
    fn pack_round_trips_captured_dumps_without_padding() {
        for name in ["pro-800-a000.syx", "pro-800-a001.syx", "pro-800-c099.syx"] {
            let data = fixture(name);
            let (_, patch) = patch_reply(&data).unwrap_or_else(|| panic!("{name}: no patch"));
            let patch = patch.unwrap_or_else(|| panic!("{name}: empty"));
            let payload = &data[11..data.len() - 1];
            assert_eq!(pack(&patch.bytes), payload, "{name}");
            assert_eq!(unpack(payload), patch.bytes, "{name}");
        }
    }

    #[test]
    fn captured_names_match_the_patch_map() {
        let a000 = patches_from_syx(&fixture("pro-800-a000.syx"));
        assert_eq!(a000.len(), 1);
        assert_eq!(a000[0].name(), "Classical Brass");
        let c099 = patches_from_syx(&fixture("pro-800-c099.syx"));
        assert_eq!(c099[0].name(), "Synckeys");
        let b094 = patches_from_syx(&fixture("pro-800-b094.syx"));
        assert_eq!(b094[0].name(), "FunkBass HB");
        let c054 = patches_from_syx(&fixture("pro-800-c054.syx"));
        assert_eq!(c054[0].name(), "Korg Lead");
    }

    #[test]
    fn global_reply_unpacks_to_forty_bytes() {
        let data = fixture("pro-800-globals.syx");
        let block = global_reply(&data).unwrap_or_else(|| panic!("no global reply"));
        assert_eq!(block.len(), 40);
        assert_eq!(global_index(block[5], block[6]), 199);
        assert_eq!(block[20], 1);
    }

    #[test]
    fn frames_match_the_documented_wire_format() {
        assert_eq!(
            sysex::hex(&read_frame(194)),
            "F0 00 20 32 00 01 24 00 77 42 01 F7"
        );
        assert_eq!(
            sysex::hex(&read_frame(399)),
            "F0 00 20 32 00 01 24 00 77 0F 03 F7"
        );
        assert_eq!(
            sysex::hex(&globals_read_frame()),
            "F0 00 20 32 00 01 24 00 77 7E 03 F7"
        );
        assert_eq!(
            sysex::hex(&write_frame(0, &[0x00, 0x01])),
            "F0 00 20 32 00 01 24 00 78 00 00 00 01 F7"
        );
    }

    #[test]
    fn writes_pad_the_record_to_173_bytes() {
        let patch = Patch::new(0, vec![0x01, 0x02, 0x03]);
        let frame = patch.write_frame();
        assert_eq!(frame.len(), 12 + 198);
        assert_eq!(&frame[11..14], &[0x00, 0x01, 0x02]);
        assert!(unpack(&frame[11..frame.len() - 1]).len() == PAYLOAD_LEN);
        let record = patch.record_frame();
        assert_eq!(record.len(), 12 + 4);
        assert_eq!(unpack(&record[11..record.len() - 1]), vec![1, 2, 3]);
    }

    #[test]
    fn empty_slot_reply_has_no_payload() {
        let reply = [
            0xF0, 0x00, 0x20, 0x32, 0x00, 0x01, 0x24, 0x00, 0x78, 0x42, 0x01, 0xF7,
        ];
        assert_eq!(patch_reply(&reply), Some((194, None)));
    }

    #[test]
    fn parse_slot_accepts_the_documented_forms() {
        assert_eq!(parse_slot("B094"), Ok(194));
        assert_eq!(parse_slot("b94"), Ok(194));
        assert_eq!(parse_slot("194"), Ok(194));
        assert_eq!(parse_slot("194/0xC2"), Ok(194));
        assert_eq!(parse_slot("A000"), Ok(0));
        assert_eq!(parse_slot("399"), Ok(399));
        assert!(parse_slot("E000").is_err());
        assert!(parse_slot("100").is_ok());
        assert!(parse_slot("400").is_err());
        assert!(parse_slot("B100").is_err());
    }

    #[test]
    fn slot_labels_cover_all_four_banks() {
        assert_eq!(slot_label(0), "A000");
        assert_eq!(slot_label(194), "B094");
        assert_eq!(slot_label(399), "D099");
    }

    #[test]
    fn names_are_limited_and_nul_padded() {
        let mut patch = Patch::init(0);
        assert_eq!(patch.name(), "Init Program");
        assert!(patch.set_name("0123456789abcdef").is_ok());
        assert_eq!(patch.name(), "0123456789abcdef");
        assert!(patch.set_name("0123456789abcdefg").is_err());
        assert!(patch.set_name("bad\nname").is_err());
        let trimmed = Patch {
            index: 0,
            bytes: vec![0u8; 100],
        };
        assert!(patch.set_name("Hi").is_ok());
        assert_eq!(&patch.bytes[150..152], b"Hi");
        assert_eq!(patch.bytes[152..166], [0u8; 14]);
        assert!(trimmed.name().is_empty());
    }

    #[test]
    fn equivalent_allows_trimmed_trailing_zeros() {
        assert!(equivalent(&[1, 2, 0, 0], &[1, 2]));
        assert!(equivalent(&[1, 2], &[1, 2, 0]));
        assert!(!equivalent(&[1, 2, 0, 3], &[1, 2]));
        assert!(!equivalent(&[1, 2], &[1, 3]));
    }

    #[test]
    fn address_round_trip() {
        for index in [0u16, 99, 100, 194, 399] {
            let [lsb, msb] = address(index);
            assert_eq!(index_of(lsb, msb), index);
        }
    }
}
