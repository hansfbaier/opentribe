//! Blocking transport for the headless commands.
//!
//! One message per slot operation, paced at 30 ms. A read that gets no answer
//! within a second is an error; an answer with no payload is an empty slot
//! (`F0 … 78 <lsb> <msb> F7`). Every write is read back and compared with what
//! was sent, allowing the device to trim trailing zeros.

use std::time::{Duration, Instant};

use tribe_core::midi::{self, MidiHub};
use tribe_core::sysex;

use crate::patch::{self, FAMILY, GLOBAL_LEN, MODEL, Patch};

/// Delay between two requests, matching the vendor editor.
pub const PACE: Duration = Duration::from_millis(30);
/// How long to wait for one reply.
pub const READ_TIMEOUT: Duration = Duration::from_millis(1000);
/// How long to wait for a global-block reply.
pub const GLOBAL_TIMEOUT: Duration = Duration::from_millis(1500);

/// Mode and firmware reported by the unit.
#[derive(Clone, Copy, Debug, Default)]
pub struct Status {
    pub mode: Option<u8>,
    /// `(major, minor, patch)`.
    pub firmware: Option<(u8, u8, u8)>,
}

impl Status {
    pub fn firmware_text(&self) -> String {
        match self.firmware {
            Some((major, minor, patch)) => format!("{major}.{minor}.{patch}"),
            None => "unknown".to_string(),
        }
    }
}

/// Firmware query: `F0 00 20 32 00 01 24 00 08 00 F7`.
pub fn firmware_request() -> Vec<u8> {
    sysex::op_message(MODEL, FAMILY, 0, 0x08, &[0x00])
}

/// Operation-mode query: `F0 00 20 32 00 01 24 00 02 F7`.
pub fn mode_request() -> Vec<u8> {
    sysex::op_message(MODEL, FAMILY, 0, 0x02, &[])
}

/// Frame for a raw SysEx command given as hex.
pub fn raw_frame(hex: &str) -> Result<Vec<u8>, String> {
    let bytes = sysex::parse_hex(hex)?;
    if bytes.first() != Some(&0xF0) || bytes.last() != Some(&0xF7) {
        return Err("raw message must start with F0 and end with F7".to_string());
    }
    Ok(bytes)
}

/// An open pair of MIDI ports, with blocking request/reply helpers.
pub struct Session {
    hub: MidiHub,
    pub out_name: String,
    pub in_name: String,
}

impl Session {
    /// Open the first port that matches `hint` (the PRO-800 shows up as
    /// `PRO 800`), or the first visible port when no hint matches.
    pub fn open(hint: Option<&str>) -> Result<Self, String> {
        let outputs = midi::output_ports()?;
        let inputs = midi::input_ports()?;
        if outputs.is_empty() {
            return Err("no MIDI output ports visible".to_string());
        }
        if inputs.is_empty() {
            return Err("no MIDI input ports visible".to_string());
        }
        let hint = hint.unwrap_or("PRO 800");
        let out_name = midi::guess_port(&outputs, hint)
            .or_else(|| {
                outputs
                    .iter()
                    .find(|name| !name.to_lowercase().contains("through"))
                    .cloned()
            })
            .ok_or("no usable MIDI output port")?;
        let in_name = midi::guess_port(&inputs, hint)
            .or_else(|| {
                inputs
                    .iter()
                    .find(|name| name == &&out_name)
                    .or_else(|| inputs.first())
                    .cloned()
            })
            .ok_or("no usable MIDI input port")?;
        let mut hub = MidiHub::default();
        hub.connect_output(&out_name)?;
        hub.connect_input(&in_name, || {})?;
        // The first frame after opening the port can be dropped. Send a mode
        // query and drain the answer before the caller's first real request.
        hub.send(&mode_request())?;
        let deadline = Instant::now() + Duration::from_millis(200);
        while Instant::now() < deadline {
            hub.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(Self {
            hub,
            out_name,
            in_name,
        })
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.hub.send(bytes)
    }

    /// Send one frame and collect replies until `want` accepts one or the
    /// timeout expires. Returns the accepted frame.
    fn request(
        &mut self,
        bytes: &[u8],
        timeout: Duration,
        want: impl Fn(&[u8]) -> bool,
    ) -> Result<Vec<u8>, String> {
        self.send(bytes)?;
        let deadline = Instant::now() + timeout;
        loop {
            for message in self.hub.poll() {
                if want(&message) {
                    return Ok(message);
                }
            }
            if Instant::now() >= deadline {
                return Err(format!("no reply within {} ms", timeout.as_millis()));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Read one program. `Ok(None)` means the slot is empty.
    ///
    /// An empty slot answers with a zero-length SysEx frame (`F0 F7`) on this
    /// unit; some captures show the fuller `78 <lsb> <msb>` header instead.
    /// Both are accepted. A first request after opening the port can be lost,
    /// so a silent attempt is retried.
    pub fn read_slot(&mut self, index: u16) -> Result<Option<Patch>, String> {
        let mut last = String::new();
        for attempt in 0..3 {
            match self.read_slot_once(index) {
                Ok(reply) => return Ok(reply),
                Err(err) => {
                    last = err;
                    if attempt < 2 {
                        std::thread::sleep(Duration::from_millis(150));
                    }
                }
            }
        }
        Err(format!("slot {}: {last}", patch::slot_label(index)))
    }

    fn read_slot_once(&mut self, index: u16) -> Result<Option<Patch>, String> {
        let reply = self.request(&patch::read_frame(index), READ_TIMEOUT, |message| {
            is_empty_reply(message)
                || patch::patch_reply(message).is_some_and(|(answered, _)| answered == index)
        })?;
        if is_empty_reply(&reply) {
            return Ok(None);
        }
        Ok(patch::patch_reply(&reply).and_then(|(_, patch)| patch))
    }

    /// Read the 40-byte global block.
    pub fn read_globals(&mut self) -> Result<Vec<u8>, String> {
        let mut last = String::new();
        for attempt in 0..3 {
            match self.read_globals_once() {
                Ok(block) => return Ok(block),
                Err(err) => {
                    last = err;
                    if attempt < 2 {
                        std::thread::sleep(Duration::from_millis(150));
                    }
                }
            }
        }
        Err(last)
    }

    fn read_globals_once(&mut self) -> Result<Vec<u8>, String> {
        let reply = self.request(&patch::globals_read_frame(), GLOBAL_TIMEOUT, |message| {
            patch::global_reply(message).is_some()
        })?;
        let block = patch::global_reply(&reply).ok_or("malformed global reply")?;
        if block.len() < GLOBAL_LEN {
            return Err(format!(
                "global block is {} bytes, expected {GLOBAL_LEN}",
                block.len()
            ));
        }
        Ok(block)
    }

    /// Query firmware version and operation mode.
    pub fn status(&mut self) -> Result<Status, String> {
        let mut status = Status::default();
        if let Ok(firmware) = self.request(&firmware_request(), READ_TIMEOUT, is_firmware_reply) {
            let body = sysex::parse(MODEL, FAMILY, &firmware)
                .map(|message| message.body)
                .unwrap_or(&[]);
            status.firmware = parse_firmware(body);
        }
        std::thread::sleep(PACE);
        if let Ok(mode) = self.request(&mode_request(), READ_TIMEOUT, is_mode_reply) {
            let body = sysex::parse(MODEL, FAMILY, &mode)
                .map(|message| message.body)
                .unwrap_or(&[]);
            status.mode = body.get(1).copied();
        }
        if status.firmware.is_none() && status.mode.is_none() {
            return Err("no reply to the firmware or mode query".to_string());
        }
        Ok(status)
    }

    /// Write a program, then read it back and compare. Returns the read-back.
    ///
    /// The device acknowledges a stored program with `01 00 00`; a different
    /// code is reported as a rejection instead of relying on the read-back.
    pub fn write_slot(&mut self, patch: &Patch) -> Result<Patch, String> {
        self.send(&patch.write_frame())?;
        if let Some(code) = self.wait_ack()
            && code != 0
        {
            return Err(format!(
                "slot {} rejected the write (ACK code {code})",
                patch::slot_label(patch.index)
            ));
        }
        std::thread::sleep(PACE);
        let read_back = self
            .read_slot(patch.index)?
            .ok_or_else(|| format!("slot {} read back empty after write", patch.index))?;
        if !patch::equivalent(&read_back.bytes, &patch.bytes) {
            return Err(format!(
                "slot {} read-back differs ({} vs {} bytes)",
                patch.index,
                read_back.bytes.len(),
                patch.bytes.len()
            ));
        }
        Ok(read_back)
    }

    /// Delete a program: `78 <lsb> <msb> F7` with no payload. The device
    /// answers `01 00 00`; a read-back must then come back empty.
    pub fn clear_slot(&mut self, index: u16) -> Result<(), String> {
        self.send(&patch::empty_frame(index))?;
        if let Some(code) = self.wait_ack()
            && code != 0
        {
            return Err(format!(
                "slot {} rejected the delete (ACK code {code})",
                patch::slot_label(index)
            ));
        }
        std::thread::sleep(PACE);
        match self.read_slot(index)? {
            None => Ok(()),
            Some(patch) => Err(format!(
                "slot {} still holds {:?} after the delete",
                patch::slot_label(index),
                patch.name()
            )),
        }
    }

    /// Wait up to 500 ms for an ACK frame and return its code.
    fn wait_ack(&mut self) -> Option<u8> {
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            for message in self.hub.poll() {
                if let Some(code) = ack_code(&message) {
                    return Some(code);
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    }

    /// Write the global block, then read it back and compare.
    ///
    /// The write frame is the one the Swumpf editor uses; it is **not** verified
    /// against firmware (gate G3 in `docs/spec-pro-800.md`).
    pub fn write_globals(&mut self, block: &[u8]) -> Result<Vec<u8>, String> {
        self.send(&patch::globals_write_frame(block))?;
        std::thread::sleep(PACE);
        let read_back = self.read_globals()?;
        if !patch::equivalent(&read_back, block) {
            return Err("global block read-back differs".to_string());
        }
        Ok(read_back)
    }

    /// Send one raw SysEx frame and wait for any reply.
    pub fn send_raw(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        self.send(bytes)?;
        let deadline = Instant::now() + READ_TIMEOUT;
        let mut replies = Vec::new();
        while Instant::now() < deadline {
            let batch = self.hub.poll();
            if batch.is_empty() {
                if !replies.is_empty() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            } else {
                replies.extend(batch);
            }
        }
        Ok(replies)
    }
}

/// The zero-length reply this unit sends for an empty program slot.
pub fn is_empty_reply(message: &[u8]) -> bool {
    message == [0xF0, 0xF7]
}

fn is_firmware_reply(message: &[u8]) -> bool {
    sysex::parse(MODEL, FAMILY, message).is_some_and(|message| message.body.first() == Some(&0x09))
}

fn is_mode_reply(message: &[u8]) -> bool {
    sysex::parse(MODEL, FAMILY, message).is_some_and(|message| message.body.first() == Some(&0x03))
}

/// ACK code from a `01 00 <code>` reply.
fn ack_code(message: &[u8]) -> Option<u8> {
    let body = sysex::parse(MODEL, FAMILY, message)?.body;
    match body {
        [0x01, 0x00, code, ..] => Some(*code),
        [0x01, code, ..] => Some(*code),
        _ => None,
    }
}

/// Decode `09 00 <major> <minor> <patch>` (the leading zero is optional).
pub fn parse_firmware(body: &[u8]) -> Option<(u8, u8, u8)> {
    let rest = body.strip_prefix(&[0x09])?;
    let rest = rest.strip_prefix(&[0x00]).unwrap_or(rest);
    match rest {
        [major, minor, patch, ..] => Some((*major, *minor, *patch)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firmware_and_mode_requests_are_family_five() {
        assert_eq!(
            sysex::hex(&firmware_request()),
            "F0 00 20 32 00 01 24 00 08 00 F7"
        );
        assert_eq!(sysex::hex(&mode_request()), "F0 00 20 32 00 01 24 00 02 F7");
    }

    #[test]
    fn firmware_reply_decodes_both_shapes() {
        assert_eq!(
            parse_firmware(&[0x09, 0x00, 0x01, 0x04, 0x06]),
            Some((1, 4, 6))
        );
        assert_eq!(parse_firmware(&[0x09, 0x01, 0x04, 0x06]), Some((1, 4, 6)));
        assert_eq!(parse_firmware(&[0x09]), None);
    }

    #[test]
    fn ack_codes_decode_from_the_long_envelope() {
        let ack = sysex::envelope(MODEL, FAMILY, 0, &[0x01, 0x00, 0x00]);
        assert_eq!(ack_code(&ack), Some(0));
        let rejected = sysex::envelope(MODEL, FAMILY, 0, &[0x01, 0x00, 0x01]);
        assert_eq!(ack_code(&rejected), Some(1));
        assert_eq!(ack_code(&[]), None);
    }

    #[test]
    fn empty_slot_reply_is_a_bare_f0_f7() {
        assert!(is_empty_reply(&[0xF0, 0xF7]));
        assert!(!is_empty_reply(&[0xF0, 0xF7, 0x00]));
        assert!(!is_empty_reply(&[]));
    }

    #[test]
    fn raw_frame_requires_a_complete_sysex() {
        assert!(raw_frame("F0 00 20 32 F7").is_ok());
        assert!(raw_frame("F0 00 20 32").is_err());
        assert!(raw_frame("00 F7").is_err());
    }
}
