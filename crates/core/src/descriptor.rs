//! Device descriptor: the data half of a controller.
//!
//! A [`Device`] lists parameters, the write groups that carry them, the read
//! block layout and the evidence behind each fact. Tabs, sections and widgets
//! are derived from this data, so a new instrument normally needs no UI code.

use crate::sysex::{self, Message, WriteStyle};

pub use crate::sysex::Addr;

/// Where a documented fact comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence {
    /// Printed in the vendor documentation (e.g. the POLY D QSG data sheet).
    Qsg,
    /// Exercised on hardware: written, read back, restored.
    Verified,
    /// Present in the vendor config but rejected by the firmware.
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
}

/// The widget shape of a parameter and how its value is displayed.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// Integer slider.
    Byte { min: u8, max: u8 },
    /// On/off checkbox.
    Bool,
    /// Named values in wire order.
    Enum(&'static [&'static str]),
}

impl Kind {
    pub fn clamp(self, value: u8) -> u8 {
        match self {
            Kind::Byte { min, max } => value.clamp(min, max),
            Kind::Bool => u8::from(value != 0),
            Kind::Enum(labels) => (value as usize).min(labels.len().saturating_sub(1)) as u8,
        }
    }

    pub fn label(self, value: u8) -> String {
        match self {
            Kind::Byte { .. } => value.to_string(),
            Kind::Bool => {
                if value == 0 {
                    "off".to_string()
                } else {
                    "on".to_string()
                }
            }
            Kind::Enum(labels) => labels
                .get(value as usize)
                .copied()
                .unwrap_or("unknown value")
                .to_string(),
        }
    }

    pub fn is_bool(self) -> bool {
        matches!(self, Kind::Bool)
    }
}

/// One value placed into a write group's payload.
#[derive(Clone, Copy, Debug)]
pub enum Slot {
    /// Fixed byte, e.g. the sub-packet of the channel message or a padding zero.
    Lit(u8),
    /// Value of the parameter with this id.
    Param(&'static str),
}

/// Parameters that travel in one message.
#[derive(Clone, Copy, Debug)]
pub struct WriteGroup {
    pub id: &'static str,
    pub addr: Addr,
    pub slots: &'static [Slot],
}

impl WriteGroup {
    pub fn params(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.slots.iter().filter_map(|slot| match slot {
            Slot::Param(id) => Some(*id),
            Slot::Lit(_) => None,
        })
    }
}

/// How reads work on this device.
#[derive(Clone, Copy, Debug)]
pub struct ReadSpec {
    /// Opcode that requests the settings block (`0x75` on family 3 devices).
    pub opcode: u8,
    /// Reply opcode that carries the block (`0x76`).
    pub reply: u8,
    /// Block length in bytes; block offsets must stay below this.
    pub block_len: usize,
}

/// One editable parameter.
#[derive(Clone, Copy, Debug)]
pub struct Param {
    /// Stable machine name, e.g. `clock.source`.
    pub id: &'static str,
    pub title: &'static str,
    pub kind: Kind,
    /// Tab title this parameter belongs to.
    pub tab: &'static str,
    /// Section heading inside the tab.
    pub section: &'static str,
    /// Id of the [`WriteGroup`] that carries this value.
    pub write: &'static str,
    pub default: u8,
    /// Offset in the read-back block, when the device reports it.
    pub read_index: Option<u8>,
    pub evidence: Evidence,
    /// Short hint shown next to the widget.
    pub hint: &'static str,
}

/// A complete instrument description.
#[derive(Clone, Copy, Debug)]
pub struct Device {
    /// Stable machine name, e.g. `poly-d`.
    pub id: &'static str,
    pub name: &'static str,
    pub model: [u8; 3],
    pub family: u8,
    /// Substring used to find the MIDI port.
    pub port_hint: &'static str,
    /// Framing variants the device is known to use.
    pub styles: &'static [WriteStyle],
    pub read: Option<ReadSpec>,
    /// Packet that restores factory settings, when one is documented.
    pub restore: Option<u8>,
    /// Parameter id used as the performance channel.
    pub channel_param: Option<&'static str>,
    pub params: &'static [Param],
    pub writes: &'static [WriteGroup],
    /// Packets the vendor config lists but the firmware rejects.
    pub rejected: &'static [(&'static str, u8)],
    pub notes: &'static [&'static str],
    pub limits: &'static [&'static str],
}

impl Device {
    pub fn param(&self, id: &str) -> Option<&'static Param> {
        self.params.iter().find(|param| param.id == id)
    }

    pub fn write_group(&self, id: &str) -> Option<&'static WriteGroup> {
        self.writes.iter().find(|group| group.id == id)
    }

    /// Tab titles in descriptor order.
    pub fn tabs(&self) -> Vec<&'static str> {
        let mut tabs: Vec<&'static str> = Vec::new();
        for param in self.params {
            if !tabs.contains(&param.tab) {
                tabs.push(param.tab);
            }
        }
        tabs
    }

    /// Sections of one tab, in descriptor order.
    pub fn sections(&self, tab: &str) -> Vec<(&'static str, Vec<&'static Param>)> {
        let mut sections: Vec<(&'static str, Vec<&'static Param>)> = Vec::new();
        for param in self.params.iter().filter(|param| param.tab == tab) {
            match sections.iter_mut().find(|(name, _)| *name == param.section) {
                Some((_, params)) => params.push(param),
                None => sections.push((param.section, vec![param])),
            }
        }
        sections
    }
}

/// Current values of every parameter, indexed like [`Device::params`].
#[derive(Clone, Debug)]
pub struct Values {
    device: &'static Device,
    data: Vec<u8>,
}

impl Values {
    pub fn defaults(device: &'static Device) -> Self {
        Self {
            device,
            data: device.params.iter().map(|param| param.default).collect(),
        }
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.device.params.iter().position(|param| param.id == id)
    }

    pub fn get(&self, id: &str) -> Option<u8> {
        self.index(id).map(|index| self.data[index])
    }

    pub fn get_or(&self, id: &str, fallback: u8) -> u8 {
        self.get(id).unwrap_or(fallback)
    }

    /// Store a value, clamped to the parameter's kind. Returns `false` when the
    /// id is unknown.
    pub fn set(&mut self, id: &str, value: u8) -> bool {
        match self.index(id) {
            Some(index) => {
                let kind = self.device.params[index].kind;
                self.data[index] = kind.clamp(value);
                true
            }
            None => false,
        }
    }
}

/// Assemble the payload of one write group from the current values.
pub fn payload_for(values: &Values, group: &WriteGroup) -> Vec<u8> {
    let mut payload = Vec::with_capacity(group.slots.len() + 1);
    if let Addr::PacketSub(_, sub) = group.addr {
        payload.push(sub);
    }
    for slot in group.slots {
        match slot {
            Slot::Lit(byte) => payload.push(*byte),
            Slot::Param(id) => payload.push(values.get_or(id, 0)),
        }
    }
    payload
}

/// Build the message for one write group.
pub fn message_for(
    device: &Device,
    values: &Values,
    style: WriteStyle,
    device_id: u8,
    group: &WriteGroup,
) -> Vec<u8> {
    let payload = payload_for(values, group);
    sysex::message(
        device.model,
        device.family,
        style,
        device_id,
        group.addr.packet(),
        &payload,
    )
}

/// Build the message that rewrites `param_id`, if the device knows it.
pub fn write_message(
    device: &Device,
    values: &Values,
    style: WriteStyle,
    device_id: u8,
    param_id: &str,
) -> Option<Vec<u8>> {
    let param = device.param(param_id)?;
    let group = device.write_group(param.write)?;
    Some(message_for(device, values, style, device_id, group))
}

/// Every write group message, in descriptor order.
pub fn all_messages(
    device: &Device,
    values: &Values,
    style: WriteStyle,
    device_id: u8,
) -> Vec<(&'static str, Vec<u8>)> {
    device
        .writes
        .iter()
        .map(|group| {
            (
                group.id,
                message_for(device, values, style, device_id, group),
            )
        })
        .collect()
}

/// Request message for the settings block.
pub fn read_request(device: &Device, device_id: u8) -> Option<Vec<u8>> {
    let spec = device.read?;
    Some(sysex::op_message(
        device.model,
        device.family,
        device_id,
        spec.opcode,
        &[],
    ))
}

/// Query the DFU operation mode.
pub fn mode_request(device: &Device, device_id: u8) -> Vec<u8> {
    sysex::op_message(
        device.model,
        device.family,
        device_id,
        sysex::op::QUERY_MODE,
        &[],
    )
}

/// Query the firmware version.
pub fn firmware_request(device: &Device, device_id: u8) -> Vec<u8> {
    sysex::op_message(
        device.model,
        device.family,
        device_id,
        sysex::op::FIRMWARE,
        &[],
    )
}

/// Restore factory settings.
pub fn restore_request(device: &Device, device_id: u8) -> Option<Vec<u8>> {
    let packet = device.restore?;
    Some(sysex::op_message(
        device.model,
        device.family,
        device_id,
        packet,
        &[],
    ))
}

/// Apply a settings block to the values. Returns how many parameters were set.
pub fn apply_block(device: &Device, values: &mut Values, block: &[u8]) -> usize {
    let mut applied = 0;
    for param in device.params {
        if let Some(index) = param.read_index
            && let Some(byte) = block.get(index as usize)
        {
            values.set(param.id, *byte);
            applied += 1;
        }
    }
    applied
}

/// ACK payload code, if the body is an ACK.
pub fn ack_code(body: &[u8]) -> Option<u8> {
    match body {
        [sysex::op::ACK, 0x00, code, ..] => Some(*code),
        [sysex::op::ACK, code] => Some(*code),
        _ => None,
    }
}

/// Human-readable decode of a reply body.
pub fn decode_reply(device: &Device, body: &[u8]) -> Option<String> {
    let (&first, rest) = body.split_first()?;
    match first {
        sysex::op::ACK => {
            let code = ack_code(body).unwrap_or(0);
            return Some(match code {
                sysex::op::ACK_OK => "ack · accepted".to_string(),
                sysex::op::ACK_REJECTED => "ack · rejected (code 5)".to_string(),
                other => format!("ack · code {other}"),
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
        sysex::op::DATA => {
            return Some(match decode_body(device, rest) {
                Some(inner) => format!("data · {inner}"),
                None => format!("data · {} bytes", rest.len()),
            });
        }
        _ => {}
    }
    if first == sysex::op::SETTINGS_BLOCK
        && let Some(spec) = device.read
    {
        let count = rest.len();
        let applied = device
            .params
            .iter()
            .filter(|param| param.read_index.is_some())
            .count();
        return Some(format!(
            "settings block {count}/{} bytes · {applied} parameters",
            spec.block_len
        ));
    }
    if let Some(decoded) = decode_body(device, body) {
        return Some(decoded);
    }
    Some(format!("packet {first:02X} · {} bytes", body.len() - 1))
}

/// Decode a single parameter message by matching it against the write groups.
fn decode_body(device: &Device, body: &[u8]) -> Option<String> {
    let (&packet, payload) = body.split_first()?;
    let group = device
        .writes
        .iter()
        .find(|group| group.addr.packet() == packet)?;
    let payload = match group.addr {
        Addr::PacketSub(_, _) => payload.get(1..)?,
        Addr::Packet(_) => payload,
    };
    let mut parts = Vec::new();
    for (index, slot) in group.slots.iter().enumerate() {
        if let Slot::Param(id) = slot
            && let (Some(param), Some(value)) = (device.param(id), payload.get(index))
        {
            parts.push(format!("{} · {}", param.title, param.kind.label(*value)));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

/// Parse a message for this device.
pub fn parse<'a>(device: &Device, msg: &'a [u8]) -> Option<Message<'a>> {
    sysex::parse(device.model, device.family, msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BYTE: Kind = Kind::Byte { min: 0, max: 127 };
    const ENUM: Kind = Kind::Enum(&["Off", "DIN", "USB", "Both"]);

    static PARAMS: [Param; 3] = [
        Param {
            id: "channel.tx",
            title: "MIDI channel TX",
            kind: Kind::Byte { min: 0, max: 16 },
            tab: "Global",
            section: "Channel",
            write: "channel",
            default: 1,
            read_index: Some(2),
            evidence: Evidence::Qsg,
            hint: "",
        },
        Param {
            id: "channel.rx",
            title: "MIDI channel RX",
            kind: Kind::Byte { min: 0, max: 16 },
            tab: "Global",
            section: "Channel",
            write: "channel",
            default: 1,
            read_index: Some(1),
            evidence: Evidence::Qsg,
            hint: "",
        },
        Param {
            id: "out.sequencer",
            title: "Sequencer output",
            kind: ENUM,
            tab: "Routing",
            section: "Outputs",
            write: "out.sequencer",
            default: 3,
            read_index: Some(23),
            evidence: Evidence::Qsg,
            hint: "",
        },
    ];

    static WRITES: [WriteGroup; 2] = [
        WriteGroup {
            id: "channel",
            addr: Addr::PacketSub(0x0E, 0x01),
            slots: &[Slot::Param("channel.tx"), Slot::Param("channel.rx")],
        },
        WriteGroup {
            id: "out.sequencer",
            addr: Addr::Packet(0x25),
            slots: &[Slot::Param("out.sequencer")],
        },
    ];

    static DEVICE: Device = Device {
        id: "test",
        name: "TEST",
        model: [0x00, 0x01, 0x0C],
        family: 3,
        port_hint: "TEST",
        styles: &[WriteStyle::PacketDirect],
        read: Some(ReadSpec {
            opcode: 0x75,
            reply: 0x76,
            block_len: 25,
        }),
        restore: Some(0x7D),
        channel_param: Some("channel.tx"),
        params: &PARAMS,
        writes: &WRITES,
        rejected: &[],
        notes: &[],
        limits: &[],
    };

    #[test]
    fn tabs_and_sections_follow_descriptor_order() {
        assert_eq!(DEVICE.tabs(), vec!["Global", "Routing"]);
        let sections = DEVICE.sections("Global");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "Channel");
        assert_eq!(sections[0].1.len(), 2);
    }

    #[test]
    fn channel_group_uses_sub_packet_and_both_values() {
        let mut values = Values::defaults(&DEVICE);
        values.set("channel.tx", 3);
        values.set("channel.rx", 4);
        let message = write_message(
            &DEVICE,
            &values,
            WriteStyle::PacketDirect,
            0x00,
            "channel.tx",
        )
        .unwrap_or_else(|| panic!("channel message"));
        assert_eq!(
            sysex::hex(&message),
            "F0 00 20 32 00 01 0C 00 0E 01 03 04 F7"
        );
    }

    #[test]
    fn enum_kind_labels_and_clamps() {
        assert_eq!(ENUM.label(2), "USB");
        assert_eq!(ENUM.label(9), "unknown value");
        assert_eq!(ENUM.clamp(9), 3);
        assert_eq!(Kind::Bool.label(0), "off");
        assert_eq!(Kind::Bool.clamp(7), 1);
        assert_eq!(BYTE.clamp(200), 127);
    }

    #[test]
    fn block_applies_only_indexed_parameters() {
        let mut block = [0u8; 25];
        block[1] = 4;
        block[2] = 5;
        block[23] = 2;
        let mut values = Values::defaults(&DEVICE);
        assert_eq!(apply_block(&DEVICE, &mut values, &block), 3);
        assert_eq!(values.get("channel.rx"), Some(4));
        assert_eq!(values.get("channel.tx"), Some(5));
        assert_eq!(values.get("out.sequencer"), Some(2));
    }

    #[test]
    fn reply_decoding_covers_block_ack_and_params() {
        let block = [sysex::op::SETTINGS_BLOCK; 26];
        let decoded = decode_reply(&DEVICE, &block).unwrap_or_else(|| panic!("block reply"));
        assert!(decoded.starts_with("settings block 25/25 bytes"));

        assert_eq!(
            decode_reply(&DEVICE, &[sysex::op::ACK, 0x00, 0x00]).as_deref(),
            Some("ack · accepted")
        );
        assert_eq!(
            decode_reply(&DEVICE, &[0x25, 0x02]).as_deref(),
            Some("Sequencer output · USB")
        );
        assert_eq!(
            decode_reply(&DEVICE, &[0x0E, 0x01, 0x03, 0x00]).as_deref(),
            Some("MIDI channel TX · 3; MIDI channel RX · 0")
        );
        let mut unknown = [0u8; 27];
        unknown[0] = 0x12;
        assert_eq!(
            decode_reply(&DEVICE, &unknown).as_deref(),
            Some("packet 12 · 26 bytes")
        );
    }
}
