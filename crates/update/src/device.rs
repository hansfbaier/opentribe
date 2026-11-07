//! The instrument table: model bytes, USB IDs and firmware flavour.
//!
//! Values come from the vendor `config.json` files (`modelid`, `listpid`, `protocol`,
//! `aliasindfu`) as recorded in `report/supported-devices.csv` of the research repository.
//! Every setting here is either read from that config or marked as inference.

use tribe_core::sysex::{self, hex};

/// How a device's firmware reaches the instrument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavour {
    /// `0x0A` header and `0x0B` data packets, replayed from the downloaded `.syx`.
    DfuPackets,
    /// INNO bootloader: `0x5A` commands addressed to model bytes `00 7F 7E`, replayed as-is.
    InnoBootloader,
    /// STM32 USB DFU over libusb — not SysEx, so not driven by this tool.
    UsbDfu,
}

impl Flavour {
    pub fn label(self) -> &'static str {
        match self {
            Flavour::DfuPackets => "0A/0B DFU packets",
            Flavour::InnoBootloader => "INNO bootloader (5A)",
            Flavour::UsbDfu => "STM32 USB DFU (not SysEx)",
        }
    }

    /// Can this tool replay firmware for the flavour?
    pub fn replayable(self) -> bool {
        matches!(self, Flavour::DfuPackets | Flavour::InnoBootloader)
    }
}

/// One supported instrument.
#[derive(Clone, Copy, Debug)]
pub struct Device {
    /// Stable id used on the command line, e.g. `pro-800`.
    pub id: &'static str,
    /// Marketing name, e.g. `PRO 800`.
    pub name: &'static str,
    pub vid: u16,
    /// PID in user mode — the only PID observed live in the research repository.
    pub pid: u16,
    /// PID in bootloader mode. Inferred from the second `listpid` entry: every live unit
    /// enumerated under the first one.
    pub pid_dfu: u16,
    /// Model bytes used by runtime SysEx.
    pub model: [u8; 3],
    /// Model bytes used by *firmware* frames. Equal to `model` except on the MODEL D,
    /// whose bootloader answers `00 7F 7E` instead of `00 01 01`.
    pub firmware_model: [u8; 3],
    /// Protocol family: 0 and 1 use the short envelope, 3 and above insert a device byte.
    pub family: u8,
    pub flavour: Flavour,
    /// MIDI port name in bootloader mode, when the vendor config documents one
    /// (`aliasindfu`). Only the MODEL D has one.
    pub dfu_port: Option<&'static str>,
}

impl Device {
    /// Where the opcode sits inside a message.
    pub fn opcode_index(&self) -> usize {
        if self.family >= 3 { 8 } else { 7 }
    }

    /// Build a control frame in this device's runtime envelope.
    pub fn op_message(&self, opcode: u8, payload: &[u8]) -> Vec<u8> {
        sysex::op_message(self.model, self.family, 0x00, opcode, payload)
    }

    /// `0x02` with no payload: ask whether the unit is in user or bootloader mode.
    pub fn query_mode(&self) -> Vec<u8> {
        self.op_message(0x02, &[])
    }

    /// `0x03 30`: reboot into the bootloader. This is the frame the vendor app sends.
    pub fn enter_dfu(&self) -> Vec<u8> {
        self.op_message(0x03, &[0x30])
    }

    /// `0x08 00`: ask for the firmware version.
    pub fn query_version(&self) -> Vec<u8> {
        self.op_message(0x08, &[0x00])
    }

    /// Port names to try after a reboot: the runtime name first, then the documented
    /// bootloader alias.
    pub fn port_candidates(&self) -> Vec<&'static str> {
        let mut names = vec![self.name];
        if let Some(alias) = self.dfu_port {
            names.push(alias);
        }
        names
    }

    /// Does this firmware stream address this device?
    pub fn firmware_matches(&self, model: [u8; 3]) -> bool {
        model == self.firmware_model
    }

    /// `VID:PID` of the user-mode unit, for `lsusb`-style reporting.
    pub fn usb_id(&self) -> Option<String> {
        if self.pid == 0 {
            None
        } else {
            Some(format!("{:04X}:{:04X}", self.vid, self.pid))
        }
    }
}

/// Every instrument this workspace knows, in the order the research documents list them.
pub const DEVICES: &[Device] = &[
    Device {
        id: "pro-800",
        name: "PRO 800",
        vid: 0x1397,
        pid: 0x125F,
        pid_dfu: 0x1260,
        model: [0x00, 0x01, 0x24],
        firmware_model: [0x00, 0x01, 0x24],
        family: 5,
        flavour: Flavour::DfuPackets,
        dfu_port: None,
    },
    Device {
        id: "poly-d",
        name: "POLY D",
        vid: 0x1397,
        pid: 0x122A,
        pid_dfu: 0x122B,
        model: [0x00, 0x01, 0x0C],
        firmware_model: [0x00, 0x01, 0x0C],
        family: 3,
        flavour: Flavour::DfuPackets,
        dfu_port: None,
    },
    Device {
        id: "lm-drum",
        name: "LM DRUM",
        vid: 0x1397,
        pid: 0x1267,
        pid_dfu: 0x1268,
        model: [0x00, 0x01, 0x28],
        firmware_model: [0x00, 0x01, 0x28],
        family: 3,
        flavour: Flavour::DfuPackets,
        dfu_port: None,
    },
    Device {
        id: "crave",
        name: "CRAVE",
        vid: 0x1397,
        pid: 0x121E,
        pid_dfu: 0x121F,
        model: [0x00, 0x01, 0x05],
        firmware_model: [0x00, 0x01, 0x05],
        family: 1,
        flavour: Flavour::DfuPackets,
        dfu_port: None,
    },
    Device {
        id: "model-d",
        name: "MODEL D",
        vid: 0x1397,
        pid: 0x1214,
        pid_dfu: 0x1215,
        model: [0x00, 0x01, 0x01],
        firmware_model: [0x00, 0x7F, 0x7E],
        family: 0,
        flavour: Flavour::InnoBootloader,
        dfu_port: Some("MODEL D Update"),
    },
    Device {
        id: "neutron",
        name: "NEUTRON",
        vid: 0x1397,
        pid: 0,
        pid_dfu: 0,
        model: [0x00, 0x00, 0x00],
        firmware_model: [0x00, 0x00, 0x00],
        family: 0,
        flavour: Flavour::UsbDfu,
        dfu_port: None,
    },
];

/// Look up a device by id, name or a unique substring of either.
pub fn find(query: &str) -> Option<&'static Device> {
    let needle = normalise(query);
    if needle.is_empty() {
        return None;
    }
    DEVICES
        .iter()
        .find(|device| normalise(device.id) == needle)
        .or_else(|| {
            DEVICES
                .iter()
                .find(|device| normalise(device.name) == needle)
        })
        .or_else(|| {
            let mut hits = DEVICES
                .iter()
                .filter(|device| normalise(device.name).contains(&needle));
            match (hits.next(), hits.next()) {
                (Some(device), None) => Some(device),
                _ => None,
            }
        })
}

/// Lowercase alphanumerics only, so `PRO 800`, `pro-800` and `PRO800` all compare equal.
pub fn normalise(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// The device whose firmware frames carry `model`, if any.
pub fn by_firmware_model(model: [u8; 3]) -> Option<&'static Device> {
    DEVICES
        .iter()
        .find(|device| device.flavour.replayable() && device.firmware_matches(model))
}

/// Describe the control frames for a device, for `--json` output and the dry-run plan.
pub fn control_frames(device: &Device) -> Vec<(&'static str, String)> {
    let mut frames = vec![
        ("query mode", hex(&device.query_mode())),
        ("enter DFU", hex(&device.enter_dfu())),
    ];
    if device.flavour == Flavour::UsbDfu {
        frames.push(("note", "STM32 USB DFU image, not SysEx".to_string()));
    } else {
        frames.push(("query version", hex(&device.query_version())));
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str) -> &'static Device {
        find(id).unwrap_or_else(|| panic!("device {id} exists"))
    }

    #[test]
    fn every_documented_device_is_present() {
        let ids: Vec<&str> = DEVICES.iter().map(|d| d.id).collect();
        assert_eq!(
            ids,
            vec!["pro-800", "poly-d", "lm-drum", "crave", "model-d", "neutron"]
        );
    }

    #[test]
    fn lookup_accepts_name_id_and_loose_typing() {
        assert_eq!(device("PRO 800").id, "pro-800");
        assert_eq!(device("pro800").id, "pro-800");
        assert_eq!(device("Poly D").id, "poly-d");
        assert_eq!(device("lm drum").id, "lm-drum");
        assert!(find("").is_none());
        assert!(find("nope").is_none());
    }

    #[test]
    fn control_frames_match_the_family_envelope() {
        // Family 5 keeps the device byte at index 7, so the opcode sits at 8.
        assert_eq!(
            hex(&device("pro-800").query_mode()),
            "F0 00 20 32 00 01 24 00 02 F7"
        );
        assert_eq!(
            hex(&device("pro-800").enter_dfu()),
            "F0 00 20 32 00 01 24 00 03 30 F7"
        );
        assert_eq!(
            hex(&device("pro-800").query_version()),
            "F0 00 20 32 00 01 24 00 08 00 F7"
        );
    }

    #[test]
    fn family_one_omits_the_device_byte() {
        let crave = device("crave");
        assert_eq!(crave.opcode_index(), 7);
        assert_eq!(hex(&crave.enter_dfu()), "F0 00 20 32 00 01 05 03 30 F7");
    }

    #[test]
    fn family_zero_also_uses_the_short_envelope() {
        let model_d = device("model-d");
        assert_eq!(model_d.opcode_index(), 7);
        assert_eq!(
            hex(&model_d.query_mode()),
            "F0 00 20 32 00 01 01 02 F7"
        );
        assert_eq!(model_d.dfu_port, Some("MODEL D Update"));
        assert_eq!(
            model_d.port_candidates(),
            vec!["MODEL D", "MODEL D Update"]
        );
    }

    #[test]
    fn firmware_model_differs_only_for_the_model_d() {
        for dev in DEVICES.iter().filter(|d| d.flavour.replayable()) {
            if dev.id == "model-d" {
                assert_eq!(dev.firmware_model, [0x00, 0x7F, 0x7E]);
                assert_ne!(dev.firmware_model, dev.model);
            } else {
                assert_eq!(dev.firmware_model, dev.model);
                assert_eq!(by_firmware_model(dev.model).map(|d| d.id), Some(dev.id));
            }
        }
        assert_eq!(by_firmware_model([0x00, 0x7F, 0x7E]).map(|d| d.id), Some("model-d"));
        assert!(by_firmware_model([0x00, 0x01, 0x01]).is_none());
    }

    #[test]
    fn neutron_is_listed_but_not_drivable() {
        let neutron = device("neutron");
        assert_eq!(neutron.flavour, Flavour::UsbDfu);
        assert!(!neutron.flavour.replayable());
        assert!(neutron.usb_id().is_none());
    }
}
