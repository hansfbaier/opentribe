//! Byte-exact checks against the POLY D QSG data sheet and the block observed
//! on firmware 1.1.3.

use tribe_core::descriptor::{self, Evidence, Values};
use tribe_core::sysex::{self, WriteStyle};

use crate::DEVICE;

/// Block returned by `75 F7` on firmware 1.1.3, before any test writes.
const OBSERVED_BLOCK: [u8; 25] = [
    0x00, 0x00, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x02, 0x00, 0x24, 0x02, 0x00, 0x00,
    0x01, 0x60, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x03,
];

fn message_for(param: &str, value: u8) -> String {
    let mut values = Values::defaults(DEVICE);
    values.set(param, value);
    let bytes = descriptor::write_message(DEVICE, &values, WriteStyle::PacketDirect, 0x00, param)
        .unwrap_or_else(|| panic!("{param} has no message"));
    sysex::hex(&bytes)
}

#[test]
fn qsg_examples_are_byte_exact() {
    let cases: &[(&str, u8, &str)] = &[
        ("transpose", 24, "0F 18"),
        ("pitch_bend", 8, "11 08 00"),
        ("key_priority", 1, "12 01"),
        ("multi_trigger", 1, "14 01 00"),
        ("mod_curve", 2, "15 02"),
        ("note_zero_cv", 0x3C, "16 3C"),
        ("clock_out", 0, "17 00"),
        ("clock_polarity", 0, "19 00"),
        ("clock_rate", 2, "1A 02"),
        ("clock_source", 2, "1B 02"),
        ("mod_wheel_range", 2, "20 02"),
        ("out.mod_wheel", 0, "21 00"),
        ("out.pitch_wheel", 1, "22 01"),
        ("out.keyboard", 2, "23 02"),
        ("out.aftertouch", 3, "24 03"),
        ("out.sequencer", 2, "25 02"),
        ("out.arp", 1, "26 01"),
        ("velocity.curve", 0, "10 00 00 00"),
        ("accent_threshold", 0x64, "1C 64"),
        ("local_keyboard", 1, "2F 01"),
    ];
    for (param, value, payload) in cases {
        assert_eq!(
            message_for(param, *value),
            format!("F0 00 20 32 00 01 0C 00 {payload} F7"),
            "parameter {param}"
        );
    }
}

#[test]
fn channel_message_matches_qsg_example() {
    let mut values = Values::defaults(DEVICE);
    values.set("channel.tx", 3);
    values.set("channel.rx", 4);
    let bytes = descriptor::write_message(
        DEVICE,
        &values,
        WriteStyle::PacketDirect,
        0x00,
        "channel.tx",
    )
    .unwrap_or_else(|| panic!("channel message"));
    assert_eq!(sysex::hex(&bytes), "F0 00 20 32 00 01 0C 00 0E 01 03 04 F7");
}

#[test]
fn long_envelope_is_selectable_but_not_default() {
    assert_eq!(DEVICE.styles.first(), Some(&WriteStyle::PacketDirect));
    let values = Values::defaults(DEVICE);
    let bytes = descriptor::write_message(
        DEVICE,
        &values,
        WriteStyle::LongEnvelope,
        0x00,
        "local_keyboard",
    )
    .unwrap_or_else(|| panic!("local keyboard message"));
    assert_eq!(sysex::hex(&bytes), "F0 00 20 32 00 01 0C 00 78 2F 00 F7");
}

#[test]
fn read_mode_firmware_and_restore_requests() {
    assert_eq!(
        descriptor::read_request(DEVICE, 0x00)
            .map(|bytes| sysex::hex(&bytes))
            .as_deref(),
        Some("F0 00 20 32 00 01 0C 00 75 F7")
    );
    assert_eq!(
        sysex::hex(&descriptor::mode_request(DEVICE, 0x00)),
        "F0 00 20 32 00 01 0C 00 02 F7"
    );
    assert_eq!(
        sysex::hex(&descriptor::firmware_request(DEVICE, 0x00)),
        "F0 00 20 32 00 01 0C 00 08 F7"
    );
    assert_eq!(
        descriptor::restore_request(DEVICE, 0x00)
            .map(|bytes| sysex::hex(&bytes))
            .as_deref(),
        Some("F0 00 20 32 00 01 0C 00 7D F7")
    );
}

#[test]
fn observed_block_decodes_to_device_state() {
    let mut values = Values::defaults(DEVICE);
    let applied = descriptor::apply_block(DEVICE, &mut values, &OBSERVED_BLOCK);
    assert_eq!(applied, DEVICE.params.len());

    let expected: &[(&str, u8)] = &[
        ("channel.rx", 0),
        ("channel.tx", 0),
        ("transpose", 12),
        ("velocity.on", 0),
        ("velocity.off", 0),
        ("velocity.curve", 0),
        ("key_priority", 0),
        ("multi_trigger", 1),
        ("pitch_bend", 2),
        ("mod_wheel_range", 2),
        ("mod_curve", 0),
        ("note_zero_cv", 36),
        ("clock_rate", 2),
        ("clock_source", 0),
        ("local_keyboard", 0),
        ("clock_polarity", 1),
        ("accent_threshold", 96),
        ("clock_out", 3),
        ("out.pitch_wheel", 3),
        ("out.mod_wheel", 3),
        ("out.keyboard", 3),
        ("out.aftertouch", 3),
        ("out.sequencer", 3),
        ("out.arp", 3),
    ];
    for (param, value) in expected {
        assert_eq!(values.get(param), Some(*value), "parameter {param}");
    }
}

#[test]
fn settings_block_reply_is_decoded() {
    let mut message = vec![sysex::op::SETTINGS_BLOCK];
    message.extend_from_slice(&OBSERVED_BLOCK);
    let decoded =
        descriptor::decode_reply(DEVICE, &message).unwrap_or_else(|| panic!("block reply"));
    assert!(
        decoded.starts_with("settings block 25/25 bytes"),
        "{decoded}"
    );
}

#[test]
fn every_parameter_is_writable_and_read_back() {
    for param in DEVICE.params {
        let group = DEVICE
            .write_group(param.write)
            .unwrap_or_else(|| panic!("{} has no write group", param.id));
        assert!(
            group.params().any(|id| id == param.id),
            "{} is missing from group {}",
            param.id,
            group.id
        );
        assert!(
            param.read_index.is_some(),
            "{} has no read-back offset",
            param.id
        );
        assert!(matches!(param.evidence, Evidence::Qsg | Evidence::Verified));
    }
}

#[test]
fn write_packets_are_unique_and_7_bit_clean() {
    let values = Values::defaults(DEVICE);
    let mut packets = Vec::new();
    for (_, bytes) in descriptor::all_messages(DEVICE, &values, WriteStyle::PacketDirect, 0x00) {
        assert_eq!(bytes[0], 0xF0);
        assert_eq!(bytes[bytes.len() - 1], 0xF7);
        assert!(bytes[1..bytes.len() - 1].iter().all(|byte| *byte < 0x80));
        packets.push(bytes[8]);
    }
    packets.sort_unstable();
    let count = packets.len();
    packets.dedup();
    assert_eq!(packets.len(), count, "two write groups share a packet");
}

#[test]
fn rejected_packets_are_not_written() {
    let values = Values::defaults(DEVICE);
    let written: Vec<u8> =
        descriptor::all_messages(DEVICE, &values, WriteStyle::PacketDirect, 0x00)
            .into_iter()
            .map(|(_, bytes)| bytes[8])
            .collect();
    for (name, packet) in DEVICE.rejected {
        assert!(
            !written.contains(packet),
            "{name} ({packet:02X}) must not be written"
        );
    }
}
