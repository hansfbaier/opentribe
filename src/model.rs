//! Parameter model: defaults, wire payloads, read-back decoding and the
//! reference table.
//!
//! Defaults follow `devices/POLY-D/MIDI-IMPLEMENTATION.md`: the "default"
//! column of the QSG SysEx table where the packet is documented there, and the
//! SynthTribe `config.json` value for the packets the QSG does not list.
//!
//! Read-back was verified against a POLY D on firmware 1.1.3:
//!
//! ```text
//! tx  F0 00 20 32 00 01 0C 00 75 F7
//! rx  F0 00 20 32 00 01 0C 00 76 <25-byte block> F7
//! ```
//!
//! The block offsets are the `index` fields from `config.json`, confirmed by
//! writing one packet at a time and re-reading the block.

use crate::sysex::{self, packet as pkt};

pub const OUTPUT_LABELS: &[&str] = &["Off", "DIN", "USB", "Both"];
pub const KEY_PRIORITY_LABELS: &[&str] = &["Low", "High", "Last"];
pub const CLOCK_SOURCE_LABELS: &[&str] = &["Internal", "MIDI DIN", "MIDI USB", "TRIG"];
pub const CLOCK_RATE_LABELS: &[&str] = &["1 PPS", "2 PPQ", "24 PPQ", "48 PPQ"];
pub const CLOCK_POLARITY_LABELS: &[&str] = &["Falling", "Rising"];
pub const MOD_CURVE_LABELS: &[&str] = &["Soft", "Medium", "Hard"];
pub const VELOCITY_CURVE_LABELS: &[&str] = &["Curve 0", "Curve 1", "Curve 2"];

/// Size of the settings block returned by `ReadEntireParameters` (`0x75`).
pub const BLOCK_LEN: usize = 25;

/// Where a documented range/default comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence {
    /// Printed in the POLY D QSG SysEx data sheet (p. 82-85).
    Qsg,
    /// Exercised on hardware: written, read back, restored.
    Verified,
    /// In the SynthTribe config but rejected by the firmware (ACK code 5).
    Unsupported,
}

impl Evidence {
    pub fn tag(self) -> &'static str {
        match self {
            Evidence::Qsg => "QSG",
            Evidence::Verified => "verified",
            Evidence::Unsupported => "rejected",
        }
    }

    /// Can the app write this packet?
    #[cfg(test)]
    pub fn writable(self) -> bool {
        self != Evidence::Unsupported
    }
}

/// One row of the reference table. `SPECS` order matches
/// [`GlobalParams::payloads`] for every writable row, which a unit test
/// enforces.
pub struct Spec {
    pub name: &'static str,
    pub packet: u8,
    /// Payload bytes after the packet number.
    pub wire: &'static str,
    pub range: &'static str,
    pub default: &'static str,
    /// Offset of this value in the `0x76` read-back block, when known.
    pub index: Option<usize>,
    pub source: Evidence,
}

pub const SPECS: &[Spec] = &[
    Spec {
        name: "MIDI channel",
        packet: pkt::CHANNEL,
        wire: "01 <tx> <rx>",
        range: "tx 0-16, rx 0-16 (0 = any)",
        default: "1 / 1",
        index: Some(2),
        source: Evidence::Qsg,
    },
    Spec {
        name: "MIDI IN transpose",
        packet: pkt::TRANSPOSE,
        wire: "<value>",
        range: "0-24 (12 = none)",
        default: "12 (none)",
        index: Some(3),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Velocity",
        packet: pkt::VELOCITY,
        wire: "<on> <off> <curve>",
        range: "0-127, curve 0-2",
        default: "0 / 0 / 0",
        index: Some(4),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Pitch bend range",
        packet: pkt::PITCH_BEND,
        wire: "<value> 00",
        range: "0-24",
        default: "12",
        index: Some(9),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Key priority",
        packet: pkt::KEY_PRIORITY,
        wire: "<value>",
        range: "0 low, 1 high, 2 last",
        default: "0 (low)",
        index: Some(7),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Multi-trigger",
        packet: pkt::MULTI_TRIGGER,
        wire: "<value> 00",
        range: "0 off, 1 on",
        default: "1 (on)",
        index: Some(8),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Modulation curve",
        packet: pkt::MOD_CURVE,
        wire: "<value>",
        range: "0 soft, 1 medium, 2 hard",
        default: "0 (soft)",
        index: Some(11),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Note at zero CV",
        packet: pkt::NOTE_ZERO_CV,
        wire: "<value>",
        range: "0-127",
        default: "36 (C2)",
        index: Some(12),
        source: Evidence::Qsg,
    },
    Spec {
        name: "MIDI clock out",
        packet: pkt::CLOCK_OUT,
        wire: "<value>",
        range: "0 off, 1 DIN, 2 USB, 3 both",
        default: "3 (both)",
        index: Some(18),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Clock polarity",
        packet: pkt::CLOCK_POLARITY,
        wire: "<value>",
        range: "0 falling, 1 rising",
        default: "1 (rising)",
        index: Some(16),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Clock rate",
        packet: pkt::CLOCK_RATE,
        wire: "<value>",
        range: "0 1PPS, 1 2PPQ, 2 24PPQ, 3 48PPQ",
        default: "2 (24 PPQ)",
        index: Some(13),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Clock source",
        packet: pkt::CLOCK_SOURCE,
        wire: "<value>",
        range: "0 internal, 1 DIN, 2 USB, 3 TRIG",
        default: "0 (internal)",
        index: Some(14),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Accent velocity threshold",
        packet: pkt::ACCENT_THRESHOLD,
        wire: "<value>",
        range: "0-127",
        default: "96",
        index: Some(17),
        source: Evidence::Verified,
    },
    Spec {
        name: "Modulation wheel range",
        packet: pkt::MOD_WHEEL_RANGE,
        wire: "<value>",
        range: "0-4",
        default: "2 (100 %)",
        index: Some(10),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Mod wheel output",
        packet: pkt::OUT_MOD_WHEEL,
        wire: "<value>",
        range: "0 off, 1 DIN, 2 USB, 3 both",
        default: "3 (both)",
        index: Some(20),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Pitch wheel output",
        packet: pkt::OUT_PITCH_WHEEL,
        wire: "<value>",
        range: "0 off, 1 DIN, 2 USB, 3 both",
        default: "3 (both)",
        index: Some(19),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Keyboard output",
        packet: pkt::OUT_KEYBOARD,
        wire: "<value>",
        range: "0 off, 1 DIN, 2 USB, 3 both",
        default: "3 (both)",
        index: Some(21),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Keyboard aftertouch output",
        packet: pkt::OUT_AFTERTOUCH,
        wire: "<value>",
        range: "0 off, 1 DIN, 2 USB, 3 both",
        default: "3 (both)",
        index: Some(22),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Sequencer output",
        packet: pkt::OUT_SEQUENCER,
        wire: "<value>",
        range: "0 off, 1 DIN, 2 USB, 3 both",
        default: "3 (both)",
        index: Some(23),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Arpeggiator output",
        packet: pkt::OUT_ARP,
        wire: "<value>",
        range: "0 off, 1 DIN, 2 USB, 3 both",
        default: "3 (both)",
        index: Some(24),
        source: Evidence::Qsg,
    },
    Spec {
        name: "Local keyboard mode",
        packet: pkt::LOCAL_KEYBOARD,
        wire: "<value>",
        range: "0 off, 1 on",
        default: "0 (off)",
        index: Some(15),
        source: Evidence::Verified,
    },
    Spec {
        name: "Sequencer auto-sync",
        packet: pkt::SEQ_AUTO_SYNC,
        wire: "-",
        range: "rejected (ACK 5)",
        default: "-",
        index: None,
        source: Evidence::Unsupported,
    },
    Spec {
        name: "Assign value",
        packet: pkt::ASSIGN,
        wire: "-",
        range: "rejected (ACK 5)",
        default: "-",
        index: None,
        source: Evidence::Unsupported,
    },
];

/// Every global parameter the app writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalParams {
    /// Wire value 0-16; 0 = any channel.
    pub tx_channel: u8,
    /// Wire value 0-16; 0 = any channel ("All" in SynthTribe).
    pub rx_channel: u8,
    /// Wire value 0-24; 12 = no transpose.
    pub transpose: u8,
    /// Wire value 0-127; 0 = dynamic velocity.
    pub velocity_on: u8,
    pub velocity_off: u8,
    /// 0-2.
    pub velocity_curve: u8,
    /// Semitones, 0-24.
    pub pitch_bend: u8,
    /// 0 low, 1 high, 2 last.
    pub key_priority: u8,
    pub multi_trigger: bool,
    /// 0 soft, 1 medium, 2 hard.
    pub mod_curve: u8,
    pub note_zero_cv: u8,
    /// 0 off, 1 DIN, 2 USB, 3 both.
    pub clock_out: u8,
    pub clock_polarity: u8,
    /// 0 1PPS, 1 2PPQ, 2 24PPQ, 3 48PPQ.
    pub clock_rate: u8,
    /// 0 internal, 1 DIN, 2 USB, 3 TRIG.
    pub clock_source: u8,
    pub accent_threshold: u8,
    pub mod_wheel_range: u8,
    pub out_mod_wheel: u8,
    pub out_pitch_wheel: u8,
    pub out_keyboard: u8,
    pub out_aftertouch: u8,
    pub out_sequencer: u8,
    pub out_arp: u8,
    pub local_keyboard: bool,
}

impl Default for GlobalParams {
    fn default() -> Self {
        Self {
            tx_channel: 1,
            rx_channel: 1,
            transpose: 12,
            velocity_on: 0,
            velocity_off: 0,
            velocity_curve: 0,
            pitch_bend: 12,
            key_priority: 0,
            multi_trigger: true,
            mod_curve: 0,
            note_zero_cv: 36,
            clock_out: 3,
            clock_polarity: 1,
            clock_rate: 2,
            clock_source: 0,
            accent_threshold: 96,
            mod_wheel_range: 2,
            out_mod_wheel: 3,
            out_pitch_wheel: 3,
            out_keyboard: 3,
            out_aftertouch: 3,
            out_sequencer: 3,
            out_arp: 3,
            local_keyboard: false,
        }
    }
}

impl GlobalParams {
    /// Packet payloads in [`SPECS`] order. Framing and envelope are applied by
    /// the caller through [`crate::sysex::param`].
    pub fn payloads(&self) -> Vec<(&'static str, u8, Vec<u8>)> {
        vec![
            (
                "MIDI channel",
                pkt::CHANNEL,
                vec![0x01, self.tx_channel, self.rx_channel],
            ),
            ("MIDI IN transpose", pkt::TRANSPOSE, vec![self.transpose]),
            (
                "Velocity",
                pkt::VELOCITY,
                vec![self.velocity_on, self.velocity_off, self.velocity_curve],
            ),
            (
                "Pitch bend range",
                pkt::PITCH_BEND,
                vec![self.pitch_bend, 0x00],
            ),
            ("Key priority", pkt::KEY_PRIORITY, vec![self.key_priority]),
            (
                "Multi-trigger",
                pkt::MULTI_TRIGGER,
                vec![u8::from(self.multi_trigger), 0x00],
            ),
            ("Modulation curve", pkt::MOD_CURVE, vec![self.mod_curve]),
            (
                "Note at zero CV",
                pkt::NOTE_ZERO_CV,
                vec![self.note_zero_cv],
            ),
            ("MIDI clock out", pkt::CLOCK_OUT, vec![self.clock_out]),
            (
                "Clock polarity",
                pkt::CLOCK_POLARITY,
                vec![self.clock_polarity],
            ),
            ("Clock rate", pkt::CLOCK_RATE, vec![self.clock_rate]),
            ("Clock source", pkt::CLOCK_SOURCE, vec![self.clock_source]),
            (
                "Accent velocity threshold",
                pkt::ACCENT_THRESHOLD,
                vec![self.accent_threshold],
            ),
            (
                "Modulation wheel range",
                pkt::MOD_WHEEL_RANGE,
                vec![self.mod_wheel_range],
            ),
            (
                "Mod wheel output",
                pkt::OUT_MOD_WHEEL,
                vec![self.out_mod_wheel],
            ),
            (
                "Pitch wheel output",
                pkt::OUT_PITCH_WHEEL,
                vec![self.out_pitch_wheel],
            ),
            (
                "Keyboard output",
                pkt::OUT_KEYBOARD,
                vec![self.out_keyboard],
            ),
            (
                "Keyboard aftertouch output",
                pkt::OUT_AFTERTOUCH,
                vec![self.out_aftertouch],
            ),
            (
                "Sequencer output",
                pkt::OUT_SEQUENCER,
                vec![self.out_sequencer],
            ),
            ("Arpeggiator output", pkt::OUT_ARP, vec![self.out_arp]),
            (
                "Local keyboard mode",
                pkt::LOCAL_KEYBOARD,
                vec![u8::from(self.local_keyboard)],
            ),
        ]
    }

    /// Decode the 25-byte block carried by a `0x76` reply.
    ///
    /// Offsets are the `index` values from the SynthTribe config; each was
    /// confirmed on hardware by writing a packet and re-reading the block.
    /// Offsets 0, 5, 6 are unused by the global UI: 0 is always zero on
    /// firmware 1.1.3, and the observed block holds the same value in 5 and 6
    /// as the config's velocity constants.
    pub fn from_block(block: &[u8]) -> Result<Self, String> {
        if block.len() < BLOCK_LEN {
            return Err(format!(
                "settings block is {} bytes, expected {BLOCK_LEN}",
                block.len()
            ));
        }
        Ok(Self {
            rx_channel: block[1],
            tx_channel: block[2],
            transpose: block[3],
            velocity_on: block[4],
            velocity_off: block[5],
            velocity_curve: block[6],
            key_priority: block[7],
            multi_trigger: block[8] != 0,
            pitch_bend: block[9],
            mod_wheel_range: block[10],
            mod_curve: block[11],
            note_zero_cv: block[12],
            clock_rate: block[13],
            clock_source: block[14],
            local_keyboard: block[15] != 0,
            clock_polarity: block[16],
            accent_threshold: block[17],
            clock_out: block[18],
            out_pitch_wheel: block[19],
            out_mod_wheel: block[20],
            out_keyboard: block[21],
            out_aftertouch: block[22],
            out_sequencer: block[23],
            out_arp: block[24],
        })
    }
}

fn on_off(value: u8) -> &'static str {
    if value == 0 { "off" } else { "on" }
}

fn route(value: u8) -> String {
    labelled(OUTPUT_LABELS, value)
}

fn labelled(labels: &[&str], value: u8) -> String {
    labels
        .get(value as usize)
        .map(|label| label.to_string())
        .unwrap_or_else(|| format!("value {value}"))
}

/// ACK payload code from a reply body, if the body is an ACK.
///
/// `0` = accepted, `5` = rejected (unknown packet, out-of-range value, or the
/// wrong framing).
pub fn ack_code(body: &[u8]) -> Option<u8> {
    match body {
        [sysex::op::ACK, _, code, ..] => Some(*code),
        [sysex::op::ACK, code] => Some(*code),
        _ => None,
    }
}

/// Best-effort decode of a received body (envelope already stripped).
pub fn decode(body: &[u8]) -> Option<String> {
    let (&first, rest) = body.split_first()?;
    if first == sysex::op::DATA {
        return Some(match decode(rest) {
            Some(inner) => format!("data · {inner}"),
            None => format!("data · {} bytes", rest.len()),
        });
    }
    match first {
        sysex::op::ACK => {
            let code = ack_code(body).unwrap_or(0);
            return Some(match code {
                0 => "ack · accepted".to_string(),
                other => format!("ack · rejected (code {other})"),
            });
        }
        sysex::op::RETURN_MODE => {
            return Some(match rest.first().copied() {
                Some(0) => "operation mode · normal".to_string(),
                Some(mode) => format!("operation mode · {mode:#04X}"),
                None => "operation mode".to_string(),
            });
        }
        sysex::op::RETURN_FIRMWARE => {
            return Some(match rest {
                [0, major, minor, patch] => format!("firmware · {major}.{minor}.{patch}"),
                [major, minor, patch] => format!("firmware · {major}.{minor}.{patch}"),
                _ => format!("firmware · {} bytes", rest.len()),
            });
        }
        sysex::op::SETTINGS_BLOCK => {
            return Some(match GlobalParams::from_block(rest) {
                Ok(_) => format!("settings block · {} bytes", rest.len()),
                Err(_) => format!("settings block · {} bytes (short)", rest.len()),
            });
        }
        sysex::op::READ => return Some("long read reply".to_string()),
        _ => {}
    }
    match first {
        pkt::CHANNEL if rest.len() >= 3 && rest[0] == 0x01 => Some(format!(
            "MIDI channel · TX {} / RX {}",
            rest[1],
            if rest[2] == 0 {
                "Any".to_string()
            } else {
                rest[2].to_string()
            }
        )),
        pkt::TRANSPOSE if !rest.is_empty() => Some(format!(
            "MIDI IN transpose · {} ({:+})",
            rest[0],
            i16::from(rest[0]) - 12
        )),
        pkt::VELOCITY if rest.len() >= 3 => Some(format!(
            "Velocity · on {} / off {} / curve {}",
            rest[0], rest[1], rest[2]
        )),
        pkt::PITCH_BEND if !rest.is_empty() => Some(format!("Pitch bend range · {} st", rest[0])),
        pkt::KEY_PRIORITY if !rest.is_empty() => Some(format!(
            "Key priority · {}",
            labelled(KEY_PRIORITY_LABELS, rest[0])
        )),
        pkt::MULTI_TRIGGER if !rest.is_empty() => {
            Some(format!("Multi-trigger · {}", on_off(rest[0])))
        }
        pkt::MOD_CURVE if !rest.is_empty() => Some(format!(
            "Modulation curve · {}",
            labelled(MOD_CURVE_LABELS, rest[0])
        )),
        pkt::NOTE_ZERO_CV if !rest.is_empty() => Some(format!("Note at zero CV · {}", rest[0])),
        pkt::CLOCK_OUT if !rest.is_empty() => Some(format!("MIDI clock out · {}", route(rest[0]))),
        pkt::CLOCK_POLARITY if !rest.is_empty() => Some(format!(
            "Clock polarity · {}",
            labelled(CLOCK_POLARITY_LABELS, rest[0])
        )),
        pkt::CLOCK_RATE if !rest.is_empty() => Some(format!(
            "Clock rate · {}",
            labelled(CLOCK_RATE_LABELS, rest[0])
        )),
        pkt::CLOCK_SOURCE if !rest.is_empty() => Some(format!(
            "Clock source · {}",
            labelled(CLOCK_SOURCE_LABELS, rest[0])
        )),
        pkt::ACCENT_THRESHOLD if !rest.is_empty() => {
            Some(format!("Accent velocity threshold · {}", rest[0]))
        }
        pkt::MOD_WHEEL_RANGE if !rest.is_empty() => {
            Some(format!("Modulation wheel range · {}", rest[0]))
        }
        pkt::OUT_MOD_WHEEL if !rest.is_empty() => {
            Some(format!("Mod wheel output · {}", route(rest[0])))
        }
        pkt::OUT_PITCH_WHEEL if !rest.is_empty() => {
            Some(format!("Pitch wheel output · {}", route(rest[0])))
        }
        pkt::OUT_KEYBOARD if !rest.is_empty() => {
            Some(format!("Keyboard output · {}", route(rest[0])))
        }
        pkt::OUT_AFTERTOUCH if !rest.is_empty() => {
            Some(format!("Keyboard aftertouch output · {}", route(rest[0])))
        }
        pkt::OUT_SEQUENCER if !rest.is_empty() => {
            Some(format!("Sequencer output · {}", route(rest[0])))
        }
        pkt::OUT_ARP if !rest.is_empty() => {
            Some(format!("Arpeggiator output · {}", route(rest[0])))
        }
        pkt::LOCAL_KEYBOARD if !rest.is_empty() => {
            Some(format!("Local keyboard mode · {}", on_off(rest[0])))
        }
        other => Some(format!(
            "packet {other:02X} · {} bytes",
            body.len().saturating_sub(1)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysex::Framing;

    /// Block read from a POLY D on firmware 1.1.3, before any test writes.
    const OBSERVED_BLOCK: [u8; BLOCK_LEN] = [
        0x00, 0x00, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x02, 0x00, 0x24, 0x02, 0x00,
        0x00, 0x01, 0x60, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
    ];

    #[allow(dead_code)]
    const _: () = assert!(BLOCK_LEN == 25);

    #[test]
    fn writable_specs_and_payloads_agree() {
        let defaults = GlobalParams::default();
        let payloads = defaults.payloads();
        let writable: Vec<&Spec> = SPECS.iter().filter(|spec| spec.source.writable()).collect();
        assert_eq!(payloads.len(), writable.len());
        for (payload, spec) in payloads.iter().zip(writable) {
            assert_eq!(payload.1, spec.packet, "packet mismatch for {}", spec.name);
            assert_eq!(
                payload.0, spec.name,
                "name mismatch for packet {:02X}",
                spec.packet
            );
        }
        assert!(SPECS.iter().any(|s| !s.source.writable()));
    }

    #[test]
    fn every_writable_spec_has_a_read_back_index() {
        for spec in SPECS.iter().filter(|spec| spec.source.writable()) {
            assert!(
                spec.index.is_some(),
                "{} has no read-back offset",
                spec.name
            );
            assert!(spec.index.unwrap() < BLOCK_LEN);
        }
    }

    #[test]
    fn defaults_are_7_bit_clean_and_in_range() {
        let payloads = GlobalParams::default().payloads();
        for (name, packet, payload) in payloads {
            assert!(!payload.is_empty(), "{name} has an empty payload");
            for byte in payload {
                assert!(
                    byte < 0x80,
                    "{name} (packet {packet:02X}) has non-7-bit byte {byte:#04X}"
                );
            }
        }
        let p = GlobalParams::default();
        assert!((0..=16).contains(&p.tx_channel));
        assert!((0..=16).contains(&p.rx_channel));
        assert!((0..=24).contains(&p.transpose));
        assert!((0..=24).contains(&p.pitch_bend));
        assert!((0..=2).contains(&p.key_priority));
        assert!((0..=2).contains(&p.mod_curve));
        assert!((0..=2).contains(&p.velocity_curve));
        assert!((0..=4).contains(&p.mod_wheel_range));
        assert!((0..=3).contains(&p.clock_source));
        assert!((0..=3).contains(&p.clock_rate));
        assert!((0..=3).contains(&p.clock_out));
    }

    #[test]
    fn local_keyboard_message_matches_verified_write() {
        let params = GlobalParams {
            local_keyboard: true,
            ..GlobalParams::default()
        };
        let payload = params
            .payloads()
            .into_iter()
            .find(|(name, _, _)| *name == "Local keyboard mode")
            .map(|(_, packet, payload)| {
                sysex::param(Framing::Qsg, sysex::DEFAULT_DEVICE_ID, packet, &payload)
            });
        assert_eq!(
            payload.as_deref().map(sysex::hex).as_deref(),
            Some("F0 00 20 32 00 01 0C 00 2F 01 F7")
        );
    }

    #[test]
    fn observed_block_decodes_to_expected_values() {
        let params = GlobalParams::from_block(&OBSERVED_BLOCK).expect("block decodes");
        assert_eq!(params.tx_channel, 0);
        assert_eq!(params.rx_channel, 0);
        assert_eq!(params.transpose, 12);
        assert_eq!(params.velocity_on, 0);
        assert_eq!(params.velocity_off, 0);
        assert_eq!(params.velocity_curve, 0);
        assert_eq!(params.key_priority, 0);
        assert!(params.multi_trigger);
        assert_eq!(params.pitch_bend, 2);
        assert_eq!(params.mod_wheel_range, 2);
        assert_eq!(params.mod_curve, 0);
        assert_eq!(params.note_zero_cv, 36);
        assert_eq!(params.clock_rate, 2);
        assert_eq!(params.clock_source, 0);
        assert!(!params.local_keyboard);
        assert_eq!(params.clock_polarity, 1);
        assert_eq!(params.accent_threshold, 96);
        assert_eq!(params.clock_out, 3);
        assert_eq!(params.out_mod_wheel, 3);
        assert_eq!(params.out_pitch_wheel, 3);
        assert_eq!(params.out_keyboard, 3);
        assert_eq!(params.out_aftertouch, 3);
        assert_eq!(params.out_sequencer, 3);
        assert_eq!(params.out_arp, 3);
    }

    #[test]
    fn short_block_is_rejected() {
        assert!(GlobalParams::from_block(&OBSERVED_BLOCK[..BLOCK_LEN - 1]).is_err());
        assert!(GlobalParams::from_block(&[]).is_err());
    }

    #[test]
    fn ack_codes_are_classified() {
        assert_eq!(ack_code(&[sysex::op::ACK, 0x00, 0x00]), Some(0));
        assert_eq!(ack_code(&[sysex::op::ACK, 0x00, 0x05]), Some(5));
        assert_eq!(ack_code(&[sysex::op::DATA, 0x00, 0x05]), None);
    }

    #[test]
    fn decode_reports_known_replies() {
        assert_eq!(
            decode(&[sysex::op::ACK, 0x00, 0x00]).as_deref(),
            Some("ack · accepted")
        );
        assert_eq!(
            decode(&[sysex::op::ACK, 0x00, 0x05]).as_deref(),
            Some("ack · rejected (code 5)")
        );
        assert_eq!(
            decode(&[sysex::op::RETURN_FIRMWARE, 0x00, 0x01, 0x01, 0x03]).as_deref(),
            Some("firmware · 1.1.3")
        );
        assert_eq!(
            decode(&[sysex::op::RETURN_MODE, 0x00]).as_deref(),
            Some("operation mode · normal")
        );
        assert_eq!(
            decode(&[sysex::op::SETTINGS_BLOCK]).as_deref(),
            Some("settings block · 0 bytes (short)")
        );
        assert_eq!(
            decode(&[pkt::KEY_PRIORITY, 2]).as_deref(),
            Some("Key priority · Last")
        );
        assert_eq!(
            decode(&[pkt::OUT_SEQUENCER, 2]).as_deref(),
            Some("Sequencer output · USB")
        );
        assert_eq!(
            decode(&[pkt::CHANNEL, 1, 3, 0]).as_deref(),
            Some("MIDI channel · TX 3 / RX Any")
        );
        assert_eq!(
            decode(&[sysex::op::DATA, pkt::LOCAL_KEYBOARD, 1]).as_deref(),
            Some("data · Local keyboard mode · on")
        );
        assert_eq!(decode(&[]), None);
    }
}
