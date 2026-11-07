//! Parsing and describing a firmware `.syx` stream.
//!
//! The vendor files are a plain concatenation of complete SysEx messages: `F0` … `F7`, no
//! gaps, every body byte below `0x80`. Nothing here interprets the firmware itself; the
//! summary exists so a wrong file or a truncated download is rejected *before* anything is
//! sent to an instrument.

use crate::device::{self, Device, Flavour};
use crate::md5;

/// Longest single message accepted. The largest vendor message is 526 bytes; anything much
/// larger is a corrupt or re-wrapped file.
const MAX_MESSAGE: usize = 8192;

/// A parsed firmware stream: the raw bytes plus the span of every message.
pub struct Firmware {
    source: String,
    data: Vec<u8>,
    spans: Vec<(usize, usize)>,
}

impl Firmware {
    /// Parse a firmware stream. `source` is only used in messages and reports.
    pub fn parse(data: Vec<u8>, source: impl Into<String>) -> Result<Self, String> {
        let source = source.into();
        if data.is_empty() {
            return Err(format!("{source}: file is empty"));
        }
        if data[0] != 0xF0 {
            return Err(format!(
                "{source}: does not start with a SysEx message (first byte {:02X})",
                data[0]
            ));
        }

        let mut spans = Vec::new();
        let mut index = 0usize;
        while index < data.len() {
            if data[index] != 0xF0 {
                return Err(format!(
                    "{source}: byte {} is {:02X}, outside any SysEx frame",
                    index, data[index]
                ));
            }
            let end = data[index + 1..]
                .iter()
                .position(|&byte| byte == 0xF7)
                .map(|offset| index + 1 + offset)
                .ok_or_else(|| format!("{source}: message at byte {index} has no F7"))?;
            if end - index + 1 > MAX_MESSAGE {
                return Err(format!(
                    "{source}: message at byte {index} is {} bytes, over the {MAX_MESSAGE} limit",
                    end - index + 1
                ));
            }
            if let Some(bad) = data[index + 1..end].iter().find(|byte| **byte >= 0x80) {
                return Err(format!(
                    "{source}: byte {:02X} inside the message at {index} is not 7-bit clean",
                    bad
                ));
            }
            spans.push((index, end + 1));
            index = end + 1;
        }

        Ok(Self {
            source,
            data,
            spans,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn message_count(&self) -> usize {
        self.spans.len()
    }

    pub fn messages(&self) -> impl Iterator<Item = &[u8]> {
        self.spans
            .iter()
            .map(|(start, end)| &self.data[*start..*end])
    }

    /// Digest of the whole file, the value the vendor cloud publishes per version.
    pub fn md5(&self) -> String {
        md5::hex_digest(&self.data)
    }

    /// The instrument this stream addresses, if it is one of ours.
    pub fn device(&self) -> Option<&'static Device> {
        let first = self.messages().next()?;
        let model = message_model(first)?;
        device::by_firmware_model(model)
    }

    /// Structural description: segments, packet counts, payload bytes, product strings.
    pub fn summary(&self) -> Summary {
        let first = self.messages().next().unwrap_or(&[]);
        let model = message_model(first).unwrap_or([0, 0, 0]);
        let opcode_index = detect_opcode_index(self.messages()).unwrap_or_else(|| {
            // Fall back to the family of the device this file addresses, so a file whose
            // header is unusual still gets a sensible index in the report.
            self.device().map(|d| d.opcode_index()).unwrap_or(8)
        });

        let mut segments: Vec<Segment> = Vec::new();
        let mut preamble: Vec<Vec<u8>> = Vec::new();
        let mut shape = Shape::Unknown;
        let mut data_packets = 0usize;
        let mut payload_bytes = 0usize;
        let mut headers = 0usize;
        let mut inno_commands = 0usize;
        let mut inno_blocks = 0usize;

        for message in self.messages() {
            if message.len() <= opcode_index {
                preamble.push(message.to_vec());
                continue;
            }
            match message[opcode_index] {
                0x0A => {
                    headers += 1;
                    shape = Shape::Dfu;
                    segments.push(segment_from_header(message, opcode_index, segments.len()));
                }
                0x0B if matches!(shape, Shape::Dfu | Shape::Unknown) => {
                    shape = Shape::Dfu;
                    data_packets += 1;
                    let payload = message.len().saturating_sub(opcode_index + 7);
                    payload_bytes += payload;
                    let counter = counter_of(message, opcode_index);
                    match segments.last_mut() {
                        Some(segment) => segment.push_packet(message.len(), payload, counter),
                        None => {
                            // Data before any header: keep the packet, flag it as segment 0.
                            let mut segment = Segment::empty(0);
                            segment.push_packet(message.len(), payload, counter);
                            segments.push(segment);
                        }
                    }
                }
                0x5A => {
                    shape = Shape::Inno;
                    inno_commands += 1;
                    if message.len() > opcode_index + 2 && message[opcode_index + 1] == 0x02 {
                        inno_blocks += 1;
                        payload_bytes += message.len().saturating_sub(opcode_index + 4);
                    }
                }
                _ => preamble.push(message.to_vec()),
            }
        }

        Summary {
            source: self.source.clone(),
            bytes: self.data.len(),
            md5: self.md5(),
            messages: self.spans.len(),
            model,
            opcode_index,
            shape,
            headers,
            data_packets,
            payload_bytes,
            inno_commands,
            inno_blocks,
            preamble,
            segments,
        }
    }
}

/// One `0x0A` header and everything that follows it until the next header.
#[derive(Clone, Debug, Default)]
pub struct Segment {
    /// Segment index byte from the header (0, 1, 2 on POLY D).
    pub index: u8,
    pub header_bytes: usize,
    pub le32_a: u32,
    pub le32_b: u32,
    pub product: String,
    pub name: String,
    /// `major.minor.patch` from the header; `(0, 0, 0)` when it could not be parsed.
    pub version: (u8, u8, u8),
    pub packets: usize,
    pub payload_bytes: usize,
    pub counter_first: Option<u16>,
    pub counter_last: Option<u16>,
    /// Length of the last message of the segment; always shorter than a full packet.
    pub last_message_len: usize,
    /// Length of the last payload, for the human report.
    pub last_payload_len: usize,
}

impl Segment {
    fn empty(index: u8) -> Self {
        Self {
            index,
            ..Default::default()
        }
    }

    fn push_packet(&mut self, message_len: usize, payload_len: usize, counter: Option<u16>) {
        self.packets += 1;
        self.payload_bytes += payload_len;
        if let Some(counter) = counter {
            self.counter_first.get_or_insert(counter);
            self.counter_last = Some(counter);
        }
        self.last_message_len = message_len;
        self.last_payload_len = payload_len;
    }

    /// `1.2.2`, or `unknown` when the header did not carry a version triple.
    pub fn version_string(&self) -> String {
        if self.version == (0, 0, 0) {
            "unknown".to_string()
        } else {
            format!("{}.{}.{}", self.version.0, self.version.1, self.version.2)
        }
    }
}

/// What kind of stream this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// `0x0A` header plus `0x0B` data packets: PRO 800, POLY D, LM DRUM, CRAVE.
    Dfu,
    /// INNO bootloader `0x5A` commands: MODEL D.
    Inno,
    /// Recognised as SysEx, but not as firmware for any device here.
    Unknown,
}

impl Shape {
    pub fn label(self) -> &'static str {
        match self {
            Shape::Dfu => "0x0A header + 0x0B packets",
            Shape::Inno => "INNO bootloader 0x5A commands",
            Shape::Unknown => "unrecognised",
        }
    }
}

/// Everything `inspect` prints.
#[derive(Clone, Debug)]
pub struct Summary {
    pub source: String,
    pub bytes: usize,
    pub md5: String,
    pub messages: usize,
    pub model: [u8; 3],
    pub opcode_index: usize,
    pub shape: Shape,
    pub headers: usize,
    pub data_packets: usize,
    pub payload_bytes: usize,
    pub inno_commands: usize,
    pub inno_blocks: usize,
    pub preamble: Vec<Vec<u8>>,
    pub segments: Vec<Segment>,
}

impl Summary {
    /// Total payload sent to the instrument.
    pub fn payload_len(&self) -> usize {
        self.payload_bytes
    }

    /// Human-readable report, one line per fact.
    pub fn describe(&self) -> Vec<String> {
        let mut lines = vec![
            format!("file        {}", self.source),
            format!("bytes       {} ({} messages)", self.bytes, self.messages),
            format!("md5         {}", self.md5),
            format!(
                "envelope    opcode at index {}, model {:02X} {:02X} {:02X}",
                self.opcode_index, self.model[0], self.model[1], self.model[2]
            ),
            format!("shape       {}", self.shape.label()),
        ];
        if self.shape == Shape::Dfu {
            lines.push(format!(
                "segments    {} ({} data packets, {} payload bytes)",
                self.headers, self.data_packets, self.payload_bytes
            ));
        } else if self.shape == Shape::Inno {
            lines.push(format!(
                "commands    {} ({} blocks, {} payload bytes)",
                self.inno_commands, self.inno_blocks, self.payload_bytes
            ));
        }
        if !self.preamble.is_empty() {
            lines.push(format!(
                "preamble    {} message(s) before the first header: {}",
                self.preamble.len(),
                self.preamble
                    .iter()
                    .map(|m| tribe_core::sysex::hex(m))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
        for segment in &self.segments {
            lines.push(format!(
                "segment {}   {} packets, {} payload bytes, {}..{} counter, last message {} B, {} version {}",
                segment.index,
                segment.packets,
                segment.payload_bytes,
                segment
                    .counter_first
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                segment
                    .counter_last
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                segment.last_message_len,
                if segment.name.is_empty() {
                    "<no name>".to_string()
                } else {
                    format!("{} ({})", segment.name, segment.product)
                },
                segment.version_string()
            ));
        }
        if let Some(device) = self.matched_device() {
            lines.push(format!(
                "device      {} ({}) — {}",
                device.name,
                device.id,
                device.flavour.label()
            ));
        }
        lines
    }

    /// The device this stream is addressed to, resolved through its model bytes.
    pub fn matched_device(&self) -> Option<&'static Device> {
        device::by_firmware_model(self.model)
    }

    /// Does the stream's shape suit the device's firmware flavour?
    pub fn suits(&self, device: &Device) -> Result<(), String> {
        if !device.firmware_matches(self.model) {
            return Err(format!(
                "{} carries firmware for model {:02X} {:02X} {:02X}; {} expects {:02X} {:02X} {:02X}",
                self.source,
                self.model[0],
                self.model[1],
                self.model[2],
                device.name,
                device.firmware_model[0],
                device.firmware_model[1],
                device.firmware_model[2]
            ));
        }
        match (device.flavour, self.shape) {
            (Flavour::DfuPackets, Shape::Dfu) | (Flavour::InnoBootloader, Shape::Inno) => Ok(()),
            (Flavour::UsbDfu, _) => Err(format!(
                "{} uses STM32 USB DFU, not SysEx; this tool cannot flash it",
                device.name
            )),
            (flavour, shape) => Err(format!(
                "{} is a {} device but the file is {}",
                device.name,
                flavour.label(),
                shape.label()
            )),
        }
    }
}

fn message_model(message: &[u8]) -> Option<[u8; 3]> {
    if message.len() < 7 || message[0] != 0xF0 || message[1..4] != [0x00, 0x20, 0x32] {
        return None;
    }
    Some([message[4], message[5], message[6]])
}

/// Find the opcode index the stream actually uses: 8 when byte 8 is a `0x0A` header,
/// 7 when byte 7 is the header (`CRAVE`) or the `0x5A` command block (`MODEL D`).
fn detect_opcode_index<'a>(mut messages: impl Iterator<Item = &'a [u8]>) -> Option<usize> {
    messages.find_map(|message| {
        if message.len() > 8 && message[8] == 0x0A {
            Some(8)
        } else if message.len() > 7 && (message[7] == 0x0A || message[7] == 0x5A) {
            Some(7)
        } else {
            None
        }
    })
}

fn counter_of(message: &[u8], opcode_index: usize) -> Option<u16> {
    let low = *message.get(opcode_index + 2)?;
    let high = *message.get(opcode_index + 3)?;
    Some(u16::from(low) | (u16::from(high) << 7))
}

fn segment_from_header(message: &[u8], opcode_index: usize, fallback_index: usize) -> Segment {
    let mut segment = Segment {
        header_bytes: message.len(),
        index: *message.get(opcode_index + 1).unwrap_or(&(fallback_index as u8)),
        ..Default::default()
    };
    let read32 = |offset: usize| -> u32 {
        let mut value = 0u32;
        for index in 0..4 {
            if let Some(byte) = message.get(offset + index) {
                value |= u32::from(*byte) << (8 * index);
            }
        }
        value
    };
    segment.le32_a = read32(opcode_index + 2);
    segment.le32_b = read32(opcode_index + 6);

    let product_start = opcode_index + 10;
    let product_end = product_start + 5;
    if message.len() > product_end {
        segment.product = ascii(&message[product_start..product_end]);
        // Layout: product[5] 00 name… 00 version[3] trailer…
        if message[product_end] == 0x00
            && let Some(nul) = message[product_end + 1..]
                .iter()
                .position(|byte| *byte == 0x00)
        {
            let name_end = product_end + 1 + nul;
            segment.name = ascii(&message[product_end + 1..name_end]);
            let version = &message[name_end + 1..];
            if version.len() >= 3 {
                segment.version = (version[0], version[1], version[2]);
            }
        }
    }
    segment
}

fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '.'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device;

    fn frame(model: [u8; 3], family: u8, body: &[u8]) -> Vec<u8> {
        tribe_core::sysex::envelope(model, family, 0, body)
    }

    /// A minimal PRO 800-shaped stream: one header and two data packets.
    fn pro800_stream() -> Vec<u8> {
        let model = [0x00, 0x01, 0x24];
        let mut header = vec![0x0A, 0x00];
        header.extend_from_slice(&0x0008_3F20u32.to_le_bytes());
        header.extend_from_slice(&0x083D_216Du32.to_le_bytes());
        header.extend_from_slice(b"P0E9I");
        header.push(0x00);
        header.extend_from_slice(b"BEHRINGER PRO 800");
        header.push(0x00);
        header.extend_from_slice(&[0x01, 0x04, 0x06]);
        header.extend_from_slice(&[0xAA; 20]);

        let packet = |counter: u8, len: usize| {
            let mut body = vec![0x0B, 0x00, counter, 0x00, 0x00, 0x00];
            body.extend(std::iter::repeat_n(0x24u8, len));
            frame(model, 5, &body)
        };

        let mut stream = frame(model, 5, &header);
        stream.extend(packet(0, 240));
        stream.extend(packet(1, 170));
        stream
    }

    fn crave_stream() -> Vec<u8> {
        let model = [0x00, 0x01, 0x05];
        let mut header = vec![0x0A, 0x00];
        header.extend_from_slice(&0x0004_623Cu32.to_le_bytes());
        header.extend_from_slice(&0x053C_7F4Eu32.to_le_bytes());
        header.extend_from_slice(b"P0DPX");
        header.push(0x00);
        header.extend_from_slice(b"CRAVE");
        header.push(0x00);
        header.extend_from_slice(&[0x01, 0x01, 0x04]);

        let mut body = vec![0x0B, 0x00, 0x00, 0x00, 0x00, 0x00];
        body.extend(std::iter::repeat_n(0x11u8, 120));

        let mut stream = frame(model, 1, &[0x03, 0x30]);
        stream.extend(frame(model, 1, &header));
        stream.extend(frame(model, 1, &body));
        stream
    }

    fn model_d_stream() -> Vec<u8> {
        let boot = [0x00, 0x7F, 0x7E];
        let mut stream = frame(boot, 0, &[0x5A, 0x01, 0x00]);
        let mut block = vec![0x5A, 0x02, 0x00];
        block.extend(std::iter::repeat_n(0x33u8, 512));
        stream.extend(frame(boot, 0, &block));
        stream
    }

    #[test]
    fn pro800_stream_is_described() {
        let firmware = Firmware::parse(pro800_stream(), "test.syx").expect("valid stream");
        let summary = firmware.summary();
        assert_eq!(summary.messages, 3);
        assert_eq!(summary.opcode_index, 8);
        assert_eq!(summary.shape, Shape::Dfu);
        assert_eq!(summary.headers, 1);
        assert_eq!(summary.data_packets, 2);
        assert_eq!(summary.payload_bytes, 240 + 170);
        assert!(summary.preamble.is_empty());

        let segment = &summary.segments[0];
        assert_eq!(segment.index, 0);
        assert_eq!(segment.name, "BEHRINGER PRO 800");
        assert_eq!(segment.product, "P0E9I");
        assert_eq!(segment.version, (1, 4, 6));
        assert_eq!(segment.version_string(), "1.4.6");
        assert_eq!(segment.le32_a, 0x0008_3F20);
        assert_eq!(segment.le32_b, 0x083D_216D);
        assert_eq!(segment.packets, 2);
        assert_eq!(segment.counter_first, Some(0));
        assert_eq!(segment.counter_last, Some(1));
        assert_eq!(segment.last_message_len, 185, "the short final packet");
        assert_eq!(summary.matched_device().map(|d| d.id), Some("pro-800"));
    }

    #[test]
    fn crave_uses_the_short_envelope_and_keeps_its_preamble() {
        let firmware = Firmware::parse(crave_stream(), "crave.syx").expect("valid stream");
        let summary = firmware.summary();
        assert_eq!(summary.opcode_index, 7);
        assert_eq!(summary.preamble.len(), 1, "the 03 30 reboot is a preamble frame");
        assert_eq!(
            tribe_core::sysex::hex(&summary.preamble[0]),
            "F0 00 20 32 00 01 05 03 30 F7"
        );
        assert_eq!(summary.segments[0].name, "CRAVE");
        assert_eq!(summary.segments[0].version, (1, 1, 4));
        assert_eq!(summary.segments[0].payload_bytes, 120);
        assert_eq!(summary.matched_device().map(|d| d.id), Some("crave"));
    }

    #[test]
    fn model_d_is_the_inno_bootloader_shape() {
        let firmware = Firmware::parse(model_d_stream(), "model-d.syx").expect("valid stream");
        let summary = firmware.summary();
        assert_eq!(summary.shape, Shape::Inno);
        assert_eq!(summary.inno_commands, 2);
        assert_eq!(summary.inno_blocks, 1);
        assert_eq!(summary.payload_bytes, 512);
        assert_eq!(summary.opcode_index, 7);
        assert_eq!(summary.matched_device().map(|d| d.id), Some("model-d"));
    }

    #[test]
    fn a_file_for_one_device_is_rejected_for_another() {
        let firmware = Firmware::parse(pro800_stream(), "pro800.syx").expect("valid stream");
        let summary = firmware.summary();
        assert!(summary.suits(device::find("pro-800").expect("device")).is_ok());
        let wrong = summary
            .suits(device::find("poly-d").expect("device"))
            .expect_err("POLY D must reject PRO 800 firmware");
        assert!(wrong.contains("expects 00 01 0C"), "{wrong}");
        assert!(summary.suits(device::find("neutron").expect("device")).is_err());
    }

    #[test]
    fn parsing_rejects_broken_streams() {
        assert!(Firmware::parse(Vec::new(), "empty").is_err());
        assert!(Firmware::parse(vec![0x01, 0x02, 0x03], "no-f0").is_err());
        assert!(Firmware::parse(vec![0xF0, 0x01, 0x02], "no-f7").is_err());
        assert!(
            Firmware::parse(vec![0xF0, 0x01, 0xF7, 0x55], "trailing").is_err(),
            "bytes outside a frame must be rejected"
        );
        assert!(
            Firmware::parse(vec![0xF0, 0x81, 0xF7], "high-bit").is_err(),
            "a body byte above 0x7F is not valid MIDI data"
        );
    }

    #[test]
    fn digest_matches_the_parsed_bytes() {
        let stream = pro800_stream();
        let expected = crate::md5::hex_digest(&stream);
        let firmware = Firmware::parse(stream, "test.syx").expect("valid stream");
        assert_eq!(firmware.md5(), expected);
        assert_eq!(firmware.summary().md5, expected);
    }
}
