//! Wire checks for the documented CCs and the measured SysEx surface.

use tribe_core::descriptor::{self, Addr, Evidence, Values};
use tribe_core::sysex::{self, WriteStyle};

use crate::DEVICE;

/// Block returned by `75 F7` on firmware 1.1.7, before any write.
const OBSERVED_BLOCK: [u8; 108] = [
    0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x67, 0x10, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x24, 0x28,
    0x29, 0x26, 0x27, 0x32, 0x2E, 0x2F, 0x33, 0x31, 0x2A, 0x2B, 0x2C, 0x2D, 0x30, 0x25, 0x02, 0x00,
    0x0A, 0x0B, 0x0E, 0x00, 0x6D, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
    0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x00, 0x01, 0x0B, 0x06, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x02, 0x02, 0x02,
    0x02, 0x01, 0x01, 0x02, 0x02, 0x02, 0x02, 0x01, 0x01, 0x01, 0x01, 0x02,
];

fn message_for(param: &str, value: u8, channel: u8) -> Vec<u8> {
    let mut values = Values::defaults(DEVICE);
    values.set(param, value);
    descriptor::write_message(
        DEVICE,
        &values,
        WriteStyle::PacketDirect,
        0x00,
        channel,
        param,
    )
    .unwrap_or_else(|| panic!("{param} has no message"))
}

#[test]
fn documented_ccs_become_channel_messages() {
    // Manual p. 27: CC 74 filter cutoff, CC 75-90 voice tunings.
    assert_eq!(message_for("filter.cutoff", 0x64, 1), vec![0xB0, 74, 0x64]);
    assert_eq!(
        message_for("tuning.bass_drum", 0x40, 1),
        vec![0xB0, 75, 0x40]
    );
    assert_eq!(message_for("tuning.claps", 0x7F, 1), vec![0xB0, 90, 0x7F]);
    // Channel 16 → status 0xBF.
    assert_eq!(message_for("filter.cutoff", 0x00, 16), vec![0xBF, 74, 0x00]);
}

#[test]
fn cc_groups_never_get_a_sysex_envelope() {
    let values = Values::defaults(DEVICE);
    for (group_id, bytes) in
        descriptor::all_messages(DEVICE, &values, WriteStyle::PacketDirect, 0x00, 1)
    {
        let group = DEVICE
            .write_group(group_id)
            .unwrap_or_else(|| panic!("{group_id}"));
        if matches!(group.addr, Addr::Cc(_)) {
            assert_eq!(bytes.len(), 3, "{group_id} is not a 3-byte CC");
            assert_eq!(bytes[0] & 0xF0, 0xB0, "{group_id} status");
        } else {
            assert_eq!(bytes[0], 0xF0, "{group_id} envelope");
        }
    }
}

#[test]
fn channel_and_note_map_frames_match_measurements() {
    let mut values = Values::defaults(DEVICE);
    values.set("channel.tx", 1);
    values.set("channel.rx", 2);
    let bytes = descriptor::write_message(
        DEVICE,
        &values,
        WriteStyle::PacketDirect,
        0x00,
        2,
        "channel.tx",
    )
    .unwrap_or_else(|| panic!("channel message"));
    assert_eq!(sysex::hex(&bytes), "F0 00 20 32 00 01 28 00 0E 01 01 02 F7");

    let mut note_message = String::from("F0 00 20 32 00 01 28 00 29");
    for slot in 1..=16u8 {
        let id = format!("note.slot{slot:02}");
        values.set(&id, 35 + slot);
        note_message.push_str(&format!(" {:02X}", 35 + slot));
    }
    note_message.push_str(" F7");
    let bytes = descriptor::write_message(
        DEVICE,
        &values,
        WriteStyle::PacketDirect,
        0x00,
        2,
        "note.slot01",
    )
    .unwrap_or_else(|| panic!("note map message"));
    assert_eq!(sysex::hex(&bytes), note_message);
}

#[test]
fn measured_frames_match_the_wire() {
    // 28 <a> <b> is a two-field frame: offsets 9 and 10.
    let mut values = Values::defaults(DEVICE);
    values.set("midi.m2u", 1);
    values.set("midi.mst", 0);
    let bytes = descriptor::write_message(
        DEVICE,
        &values,
        WriteStyle::PacketDirect,
        0x00,
        1,
        "midi.m2u",
    )
    .unwrap_or_else(|| panic!("midi through message"));
    assert_eq!(sysex::hex(&bytes), "F0 00 20 32 00 01 28 00 28 01 00 F7");

    // Single-byte frames measured 2026-10-28.
    let cases: &[(&str, u8, &str)] = &[
        ("end_mode", 0x01, "2A 01"),
        ("clock.source", 0x02, "1B 02"),
        ("clock.rate", 100, "1A 64"),
        ("sample.mode", 0x00, "74 0A 00"),
        ("accent_threshold", 0x40, "1C 40"),
        ("autosave", 0x00, "74 0B 00"),
        ("beep", 0x01, "74 0C 01"),
        ("random_hat", 0x01, "74 09 01"),
        ("trigger.main", 0x02, "2C 00 02"),
        ("trigger.ex", 0x03, "2C 01 03"),
        ("trigger.exex", 0x0A, "2C 02 0A"),
    ];
    for (param, value, payload) in cases {
        assert_eq!(
            sysex::hex(&message_for(param, *value, 1)),
            format!("F0 00 20 32 00 01 28 00 {payload} F7"),
            "parameter {param}"
        );
    }
}

#[test]
fn read_request_uses_the_measured_opcode() {
    assert_eq!(
        descriptor::read_request(DEVICE, 0x00)
            .map(|bytes| sysex::hex(&bytes))
            .as_deref(),
        Some("F0 00 20 32 00 01 28 00 75 F7")
    );
}

#[test]
fn observed_block_decodes_to_measured_state() {
    let mut values = Values::defaults(DEVICE);
    let applied = descriptor::apply_block(DEVICE, &mut values, &OBSERVED_BLOCK);
    let with_offset = DEVICE
        .params
        .iter()
        .filter(|param| param.read_index.is_some())
        .count();
    assert_eq!(applied, with_offset);

    assert_eq!(values.get("channel.rx"), Some(16));
    assert_eq!(values.get("channel.tx"), Some(0));
    assert_eq!(values.get("accent_threshold"), Some(109));
    assert_eq!(values.get("autosave"), Some(1));
    assert_eq!(values.get("beep"), Some(0));
    assert_eq!(values.get("random_hat"), Some(0));
    assert_eq!(values.get("midi.m2u"), Some(0));
    assert_eq!(values.get("midi.mst"), Some(0));
    assert_eq!(values.get("clock.source"), Some(0));
    assert_eq!(values.get("clock.rate"), Some(103));
    assert_eq!(values.get("sample.mode"), Some(1));
    assert_eq!(values.get("end_mode"), Some(2));
    assert_eq!(values.get("trigger.main"), Some(10));
    assert_eq!(values.get("trigger.ex"), Some(11));
    assert_eq!(values.get("trigger.exex"), Some(14));
    let slots: Vec<u8> = (1..=16u8)
        .map(|slot| values.get(&format!("note.slot{slot:02}")).unwrap_or(0))
        .collect();
    assert_eq!(
        slots,
        vec![
            36, 40, 41, 38, 39, 50, 46, 47, 51, 49, 42, 43, 44, 45, 48, 37
        ]
    );
}

#[test]
fn cc_parameters_have_no_read_back_and_sysex_ones_do() {
    for param in DEVICE.params {
        let group = DEVICE
            .write_group(param.write)
            .unwrap_or_else(|| panic!("{} has no group", param.id));
        match group.addr {
            Addr::Cc(_) => {
                assert!(
                    param.read_index.is_none(),
                    "{} is CC but has an offset",
                    param.id
                );
                assert_eq!(param.evidence, Evidence::Qsg, "{} CC source", param.id);
            }
            _ => {
                assert!(param.read_index.is_some(), "{} has no offset", param.id);
                assert!(
                    matches!(param.evidence, Evidence::Verified | Evidence::VendorNamed),
                    "{} source",
                    param.id
                );
            }
        }
    }
}

#[test]
fn write_addresses_are_unique() {
    let mut keys: Vec<u32> = DEVICE.writes.iter().map(|group| group.addr.key()).collect();
    keys.sort_unstable();
    let count = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), count, "duplicate write address");
}
