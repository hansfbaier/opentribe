//! Replay and status queries over a MIDI sink.
//!
//! [`Sink`] exists so the send loop can be tested without a port: `MidiHub` implements it in
//! the binary, tests implement it with a `Vec`. Every send is one complete SysEx message, in
//! file order, paced by the vendor timer (10 ms). Replies are collected and counted, never
//! required — the vendor app does not gate the transfer on them either.

use std::time::{Duration, Instant};

use tribe_core::midi::MidiHub;

use crate::device::Device;
use crate::firmware::Firmware;

/// Something that can send SysEx messages and hand back what arrived.
pub trait Sink {
    fn send(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn poll(&mut self) -> Vec<Vec<u8>>;
}

impl Sink for MidiHub {
    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        MidiHub::send(self, bytes)
    }

    fn poll(&mut self) -> Vec<Vec<u8>> {
        MidiHub::poll(self)
    }
}

/// Explicit permission to transmit. `replay_consented` takes one by value, so a caller has to
/// write `Consent::given()` at the call site before anything leaves the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Consent(bool);

impl Consent {
    /// The caller has confirmed the device, the file and the risk.
    pub fn given() -> Self {
        Self(true)
    }

    /// Printed plan only; nothing is transmitted.
    pub fn withheld() -> Self {
        Self(false)
    }

    pub fn allowed(self) -> bool {
        self.0
    }
}

/// What the replay did.
#[derive(Clone, Debug, Default)]
pub struct ReplayReport {
    pub sent: usize,
    pub acks: usize,
    pub replies: Vec<Vec<u8>>,
    pub elapsed: Duration,
}

/// What the instrument answered to the status queries.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// `Some(0x00)` in user mode, `Some(0x31)` in the bootloader, `None` if silent.
    pub mode: Option<u8>,
    pub version: Option<(u8, u8, u8)>,
    pub replies: Vec<Vec<u8>>,
}

impl Status {
    pub fn mode_label(&self) -> &'static str {
        match self.mode {
            Some(0x00) => "user mode",
            Some(0x31) => "bootloader (DFU) mode",
            Some(_) => "unrecognised mode",
            None => "no reply",
        }
    }

    pub fn version_string(&self) -> Option<String> {
        self.version
            .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"))
    }
}

/// Send the mode query (`0x02`) and read the answer. Never reboots.
pub fn query_mode<S: Sink>(
    sink: &mut S,
    device: &Device,
    timeout: Duration,
) -> Result<Status, String> {
    let replies = collect(sink, &[device.query_mode()], device, timeout)?;
    let mut status = Status {
        replies,
        ..Default::default()
    };
    for message in &status.replies {
        if message.get(device.opcode_index()) == Some(&0x03) {
            status.mode = message.get(device.opcode_index() + 1).copied();
        }
    }
    Ok(status)
}

/// Mode and firmware version, both read-only.
pub fn query_status<S: Sink>(
    sink: &mut S,
    device: &Device,
    timeout: Duration,
) -> Result<Status, String> {
    let replies = collect(
        sink,
        &[device.query_mode(), device.query_version()],
        device,
        timeout,
    )?;
    let index = device.opcode_index();
    let mut status = Status {
        replies,
        ..Default::default()
    };
    for message in &status.replies {
        match message.get(index) {
            Some(0x03) => status.mode = message.get(index + 1).copied(),
            Some(0x09) => status.version = read_version(message, index),
            _ => {}
        }
    }
    Ok(status)
}

/// Replay every message of `firmware` in order, paced by `pace`.
///
/// `progress` is called after each message with `(sent, total)`. A sink error aborts the
/// replay and is returned with the count already sent, so the caller can say how far it got.
pub fn replay<S: Sink>(
    sink: &mut S,
    firmware: &Firmware,
    pace: Duration,
    mut progress: impl FnMut(usize, usize),
) -> Result<ReplayReport, String> {
    let summary = firmware.summary();
    let total = firmware.message_count();
    let start = Instant::now();
    let mut report = ReplayReport::default();

    for (index, message) in firmware.messages().enumerate() {
        let deadline = start + pace * index as u32;
        let now = Instant::now();
        if deadline > now {
            std::thread::sleep(deadline - now);
        }
        if let Err(err) = sink.send(message) {
            return Err(format!(
                "send failed after {} of {total} messages: {err}",
                report.sent
            ));
        }
        report.sent += 1;
        for reply in sink.poll() {
            if is_ack(&reply, summary.model, summary.opcode_index) {
                report.acks += 1;
            }
            report.replies.push(reply);
        }
        progress(index + 1, total);
    }

    report.elapsed = start.elapsed();
    Ok(report)
}

/// Replay with a permission check. `Consent::withheld()` returns without transmitting.
pub fn replay_consented<S: Sink>(
    sink: &mut S,
    firmware: &Firmware,
    pace: Duration,
    consent: Consent,
    progress: impl FnMut(usize, usize),
) -> Result<ReplayReport, String> {
    if !consent.allowed() {
        return Err("refusing to transmit without consent".to_string());
    }
    replay(sink, firmware, pace, progress)
}

/// Send the reboot frame and give the instrument `wait` to re-enumerate.
///
/// The caller must reopen the port afterwards: bootloader mode may present a different port
/// name and always presents a different USB PID. This only transmits `0x03 30` and waits.
pub fn reboot_to_dfu<S: Sink>(
    sink: &mut S,
    device: &Device,
    wait: Duration,
) -> Result<Vec<Vec<u8>>, String> {
    sink.send(&device.enter_dfu())?;
    let start = Instant::now();
    let mut replies = Vec::new();
    while start.elapsed() < wait {
        std::thread::sleep(Duration::from_millis(50));
        replies.extend(sink.poll());
    }
    Ok(replies)
}

/// `F0 00 20 32 <model> [did] 01 …` — the acknowledgement opcode.
fn is_ack(message: &[u8], model: [u8; 3], opcode_index: usize) -> bool {
    message.get(1..4) == Some(&[0x00, 0x20, 0x32])
        && message.get(4..7) == Some(&model)
        && message.get(opcode_index) == Some(&0x01)
}

fn collect<S: Sink>(
    sink: &mut S,
    requests: &[Vec<u8>],
    device: &Device,
    timeout: Duration,
) -> Result<Vec<Vec<u8>>, String> {
    let _ = sink.poll();
    for request in requests {
        sink.send(request)?;
    }
    let index = device.opcode_index();
    let start = Instant::now();
    let mut replies = Vec::new();
    while start.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(20));
        replies.extend(sink.poll());
        // Both queries answered: no need to sit out the rest of the timeout.
        let mode = replies.iter().any(|m| m.get(index) == Some(&0x03));
        let version = replies.iter().any(|m| m.get(index) == Some(&0x09));
        if mode && (requests.len() == 1 || version) {
            break;
        }
    }
    Ok(replies)
}

fn read_version(message: &[u8], opcode_index: usize) -> Option<(u8, u8, u8)> {
    let body = message.get(opcode_index + 1..message.len().checked_sub(1)?)?;
    if body.len() >= 4 {
        // `09 00 major minor patch`: the sub-index byte comes first.
        let tail = &body[body.len() - 3..];
        Some((tail[0], tail[1], tail[2]))
    } else if body.len() == 3 {
        Some((body[0], body[1], body[2]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device;

    /// Records what was sent and hands back one queued reply per send, so the drain at the
    /// start of `collect` cannot eat the fixture.
    #[derive(Default)]
    struct MockSink {
        sent: Vec<Vec<u8>>,
        queued: std::collections::VecDeque<Vec<u8>>,
        pending: Vec<Vec<u8>>,
        fail_at: Option<usize>,
    }

    impl MockSink {
        fn with_replies(replies: Vec<Vec<u8>>) -> Self {
            Self {
                queued: replies.into(),
                ..Default::default()
            }
        }
    }

    impl Sink for MockSink {
        fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
            if Some(self.sent.len()) == self.fail_at {
                return Err("port closed".to_string());
            }
            self.sent.push(bytes.to_vec());
            if let Some(reply) = self.queued.pop_front() {
                self.pending.push(reply);
            }
            Ok(())
        }

        fn poll(&mut self) -> Vec<Vec<u8>> {
            std::mem::take(&mut self.pending)
        }
    }

    fn frame(model: [u8; 3], family: u8, body: &[u8]) -> Vec<u8> {
        tribe_core::sysex::envelope(model, family, 0, body)
    }

    fn stream() -> Vec<u8> {
        let model = [0x00, 0x01, 0x24];
        let mut header = vec![0x0A, 0x00];
        header.extend_from_slice(&[0x20, 0x3F, 0x08, 0x00]);
        header.extend_from_slice(&[0x6D, 0x21, 0x3D, 0x08]);
        header.extend_from_slice(b"P0E9I\x00X\x00\x01\x04\x06");
        let mut out = frame(model, 5, &header);
        for counter in 0..3u8 {
            let mut body = vec![0x0B, 0x00, counter, 0x00, 0x00, 0x00, 0x11, 0x22, 0x33];
            body.push(0x44);
            out.extend(frame(model, 5, &body));
        }
        out
    }

    fn device(id: &str) -> &'static Device {
        device::find(id).unwrap_or_else(|| panic!("device {id}"))
    }

    #[test]
    fn replay_sends_every_message_in_order() {
        let firmware = Firmware::parse(stream(), "fw.syx").expect("valid stream");
        let mut sink = MockSink::default();
        let mut seen = Vec::new();
        let report = replay(&mut sink, &firmware, Duration::from_millis(1), |sent, total| {
            seen.push((sent, total))
        })
        .expect("replay succeeds");
        assert_eq!(report.sent, 4);
        assert_eq!(sink.sent.len(), 4);
        assert_eq!(
            sink.sent[0],
            firmware.messages().next().expect("first message")
        );
        assert_eq!(seen, vec![(1, 4), (2, 4), (3, 4), (4, 4)]);
        assert!(report.elapsed >= Duration::from_millis(3));
    }

    #[test]
    fn replay_reports_a_broken_port_with_the_count() {
        let firmware = Firmware::parse(stream(), "fw.syx").expect("valid stream");
        let mut sink = MockSink {
            fail_at: Some(2),
            ..Default::default()
        };
        let error = replay(&mut sink, &firmware, Duration::ZERO, |_, _| {})
            .expect_err("third send fails");
        assert!(error.contains("after 2 of 4 messages"), "{error}");
    }

    #[test]
    fn replay_refuses_without_consent() {
        let firmware = Firmware::parse(stream(), "fw.syx").expect("valid stream");
        let mut sink = MockSink::default();
        assert!(
            replay_consented(
                &mut sink,
                &firmware,
                Duration::ZERO,
                Consent::withheld(),
                |_, _| {}
            )
            .is_err()
        );
        assert!(sink.sent.is_empty(), "a withheld consent sends nothing");
        let count = replay_consented(
            &mut sink,
            &firmware,
            Duration::ZERO,
            Consent::given(),
            |_, _| {},
        )
        .expect("consented replay")
        .sent;
        assert_eq!(count, 4);
    }

    #[test]
    fn acks_are_counted_and_other_replies_kept() {
        let firmware = Firmware::parse(stream(), "fw.syx").expect("valid stream");
        let model = [0x00, 0x01, 0x24];
        // One ACK for the first message only: the counter must not double-count.
        let mut sink = MockSink::with_replies(vec![frame(model, 5, &[0x01, 0x00, 0x00])]);
        let report = replay(&mut sink, &firmware, Duration::ZERO, |_, _| {}).expect("replay");
        assert_eq!(report.acks, 1);
        assert_eq!(report.replies.len(), 1);
    }

    #[test]
    fn status_reads_mode_and_version() {
        let pro = device("pro-800");
        let mut sink = MockSink::with_replies(vec![
            frame(pro.model, 5, &[0x03, 0x00]),
            frame(pro.model, 5, &[0x09, 0x00, 0x01, 0x04, 0x06]),
        ]);
        let status = query_status(&mut sink, pro, Duration::from_millis(300)).expect("status");
        assert_eq!(status.mode, Some(0x00));
        assert_eq!(status.version, Some((1, 4, 6)));
        assert_eq!(status.version_string().as_deref(), Some("1.4.6"));
        assert_eq!(status.mode_label(), "user mode");
        assert_eq!(sink.sent.len(), 2);
        assert_eq!(sink.sent[0], pro.query_mode());
        assert_eq!(sink.sent[1], pro.query_version());
    }

    #[test]
    fn status_survives_silence() {
        let pro = device("pro-800");
        let mut sink = MockSink::default();
        let status = query_mode(&mut sink, pro, Duration::from_millis(60)).expect("query");
        assert_eq!(status.mode, None);
        assert_eq!(status.mode_label(), "no reply");
    }

    #[test]
    fn bootloader_mode_is_recognised() {
        let pro = device("pro-800");
        let mut sink = MockSink::with_replies(vec![frame(pro.model, 5, &[0x03, 0x31])]);
        let status = query_mode(&mut sink, pro, Duration::from_millis(300)).expect("query");
        assert_eq!(status.mode, Some(0x31));
        assert_eq!(status.mode_label(), "bootloader (DFU) mode");
    }

    #[test]
    fn reboot_sends_the_documented_frame() {
        let pro = device("pro-800");
        let mut sink = MockSink::default();
        let replies = reboot_to_dfu(&mut sink, pro, Duration::from_millis(60)).expect("reboot");
        assert!(replies.is_empty());
        assert_eq!(sink.sent.len(), 1);
        assert_eq!(
            tribe_core::sysex::hex(&sink.sent[0]),
            "F0 00 20 32 00 01 24 00 03 30 F7"
        );
    }
}
