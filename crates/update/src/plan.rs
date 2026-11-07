//! The dry-run plan: what would be sent, to which device, at what pace, and how long it takes.
//!
//! Building a plan never touches MIDI. It is the reviewable artefact the `flash` command
//! prints before anything is written, and the `--json` form is what a script should diff.

use std::time::Duration;

use serde_json::{json, Value};

use crate::device::{self, Device};
use crate::firmware::{Firmware, Summary};

/// A validated, not-yet-sent update.
#[derive(Clone, Debug)]
pub struct Plan {
    pub device: &'static Device,
    pub summary: Summary,
    pub pace: Duration,
    /// Send `0x03 30` and wait for the unit to come back before replaying.
    pub reboot: bool,
    pub reconnect_wait: Duration,
}

impl Plan {
    /// Validate the firmware against the device and build the plan.
    pub fn build(
        device: &'static Device,
        firmware: &Firmware,
        pace: Duration,
        reboot: bool,
        reconnect_wait: Duration,
    ) -> Result<Self, String> {
        let summary = firmware.summary();
        summary.suits(device)?;
        if device.flavour == crate::device::Flavour::UsbDfu {
            return Err(format!(
                "{} has no SysEx firmware path; this tool does not flash it",
                device.name
            ));
        }
        if summary.messages == 0 {
            return Err("firmware stream contains no messages".to_string());
        }
        Ok(Self {
            device,
            summary,
            pace,
            reboot,
            reconnect_wait,
        })
    }

    /// Control frames sent before the file is replayed.
    pub fn prelude(&self) -> Vec<(&'static str, Vec<u8>)> {
        let mut frames = vec![("query mode", self.device.query_mode())];
        if self.reboot {
            frames.push(("enter DFU", self.device.enter_dfu()));
        }
        frames
    }

    /// Messages that will be replayed (control frames excluded).
    pub fn messages(&self) -> usize {
        self.summary.messages
    }

    /// Bytes replayed, header bytes included.
    pub fn bytes(&self) -> usize {
        self.summary.bytes
    }

    /// Wall-clock estimate: paced replay plus the reconnect wait when rebooting.
    pub fn estimate(&self) -> Duration {
        let replay = self.pace.saturating_mul(self.summary.messages as u32);
        let reconnect = if self.reboot {
            self.reconnect_wait
        } else {
            Duration::ZERO
        };
        replay + reconnect
    }

    /// The human form, one line per fact.
    pub fn describe(&self) -> Vec<String> {
        let device = self.device;
        let mut lines = vec![
            format!("device      {} ({})", device.name, device.id),
            format!(
                "usb         {} (bootloader {})",
                device
                    .usb_id()
                    .unwrap_or_else(|| "unknown".to_string()),
                format!("{:04X}:{:04X}", device.vid, device.pid_dfu)
            ),
            format!("firmware    {}", self.summary.source),
            format!(
                "stream      {} messages, {} bytes, {} payload, {}",
                self.summary.messages,
                self.summary.bytes,
                self.summary.payload_len(),
                self.summary.shape.label()
            ),
            format!("md5         {}", self.summary.md5),
            format!(
                "pace        {} ms per message (vendor timer)",
                self.pace.as_millis()
            ),
            format!(
                "prelude     {}",
                if self.reboot {
                    "0x02 mode query, then 0x03 30 and a reconnect wait"
                } else {
                    "0x02 mode query only, no reboot"
                }
            ),
            format!("estimate    {}", human_duration(self.estimate())),
        ];
        if !self.summary.preamble.is_empty() {
            lines.push(format!(
                "note        the file itself starts with {} frame(s), e.g. {:?}; they are replayed verbatim",
                self.summary.preamble.len(),
                tribe_core::sysex::hex(&self.summary.preamble[0])
            ));
        }
        for (label, bytes) in self.prelude() {
            lines.push(format!(
                "frame       {label:13} {}",
                tribe_core::sysex::hex(&bytes)
            ));
        }
        lines
    }

    pub fn json(&self) -> Value {
        json!({
            "device": {
                "id": self.device.id,
                "name": self.device.name,
                "usb": self.device.usb_id(),
                "pid_dfu": format!("{:04X}", self.device.pid_dfu),
                "family": self.device.family,
                "flavour": self.device.flavour.label(),
                "dfu_port": self.device.dfu_port,
            },
            "firmware": {
                "source": self.summary.source,
                "bytes": self.summary.bytes,
                "md5": self.summary.md5,
                "messages": self.summary.messages,
                "shape": self.summary.shape.label(),
                "payload_bytes": self.summary.payload_len(),
                "inno_blocks": self.summary.inno_blocks,
                "segments": self.summary.segments.iter().map(|s| json!({
                    "index": s.index,
                    "packets": s.packets,
                    "payload_bytes": s.payload_bytes,
                    "name": s.name,
                    "product": s.product,
                    "version": s.version_string(),
                })).collect::<Vec<_>>(),
            },
            "prelude": self.prelude().into_iter().map(|(label, bytes)| json!({
                "label": label,
                "bytes": tribe_core::sysex::hex(&bytes),
            })).collect::<Vec<_>>(),
            "reboot": self.reboot,
            "pace_ms": self.pace.as_millis(),
            "estimate_seconds": self.estimate().as_secs_f64(),
        })
    }
}

/// `1m 42s` style durations for the console.
pub fn human_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else if seconds > 0 {
        format!("{seconds}s")
    } else {
        format!("{}ms", duration.as_millis())
    }
}

/// Convenience: the devices whose firmware can be replayed at all.
pub fn replayable_devices() -> Vec<&'static Device> {
    device::DEVICES
        .iter()
        .filter(|device| device.flavour.replayable())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firmware::Firmware;

    fn frame(model: [u8; 3], family: u8, body: &[u8]) -> Vec<u8> {
        tribe_core::sysex::envelope(model, family, 0, body)
    }

    fn pro800_stream(packets: u8) -> Vec<u8> {
        let model = [0x00, 0x01, 0x24];
        let mut header = vec![0x0A, 0x00];
        header.extend_from_slice(&[0x20, 0x3F, 0x08, 0x00]);
        header.extend_from_slice(&[0x6D, 0x21, 0x3D, 0x08]);
        header.extend_from_slice(b"P0E9I\x00BEHRINGER PRO 800\x00");
        header.extend_from_slice(&[0x01, 0x04, 0x06]);
        let mut stream = frame(model, 5, &header);
        for counter in 0..packets {
            let mut body = vec![0x0B, 0x00, counter, 0x00, 0x00, 0x00];
            body.extend(std::iter::repeat_n(0x24u8, 8));
            stream.extend(frame(model, 5, &body));
        }
        stream
    }

    fn device(id: &str) -> &'static Device {
        device::find(id).unwrap_or_else(|| panic!("device {id}"))
    }

    #[test]
    fn plan_counts_messages_and_estimates_time() {
        let firmware = Firmware::parse(pro800_stream(4), "fw.syx").expect("valid stream");
        let plan = Plan::build(
            device("pro-800"),
            &firmware,
            crate::VENDOR_PACE,
            true,
            crate::VENDOR_RECONNECT_WAIT,
        )
        .expect("plan builds");
        assert_eq!(plan.messages(), 5, "one header plus four packets");
        assert_eq!(plan.prelude().len(), 2);
        assert_eq!(plan.estimate().as_millis(), 5 * 10 + 8_000);
    }

    #[test]
    fn plan_without_reboot_has_one_prelude_frame() {
        let firmware = Firmware::parse(pro800_stream(1), "fw.syx").expect("valid stream");
        let plan = Plan::build(
            device("pro-800"),
            &firmware,
            crate::VENDOR_PACE,
            false,
            crate::VENDOR_RECONNECT_WAIT,
        )
        .expect("plan builds");
        assert_eq!(plan.prelude().len(), 1);
        assert_eq!(plan.estimate().as_millis(), 20);
    }

    #[test]
    fn plan_refuses_the_wrong_device_and_the_neutron() {
        let firmware = Firmware::parse(pro800_stream(1), "fw.syx").expect("valid stream");
        assert!(
            Plan::build(
                device("poly-d"),
                &firmware,
                crate::VENDOR_PACE,
                true,
                crate::VENDOR_RECONNECT_WAIT
            )
            .is_err()
        );
        assert!(
            Plan::build(
                device("neutron"),
                &firmware,
                crate::VENDOR_PACE,
                true,
                crate::VENDOR_RECONNECT_WAIT
            )
            .is_err()
        );
    }

    #[test]
    fn json_carries_the_facts_a_script_needs() {
        let firmware = Firmware::parse(pro800_stream(2), "fw.syx").expect("valid stream");
        let plan = Plan::build(
            device("pro-800"),
            &firmware,
            crate::VENDOR_PACE,
            true,
            crate::VENDOR_RECONNECT_WAIT,
        )
        .expect("plan builds");
        let json = plan.json();
        assert_eq!(json["device"]["id"], "pro-800");
        assert_eq!(json["firmware"]["md5"], firmware.md5());
        assert_eq!(json["firmware"]["segments"][0]["name"], "BEHRINGER PRO 800");
        assert_eq!(json["pace_ms"], 10);
        assert!(json["estimate_seconds"].as_f64().unwrap_or_default() > 8.0);
    }

    #[test]
    fn human_durations_read_naturally() {
        assert_eq!(human_duration(Duration::from_millis(4)), "4ms");
        assert_eq!(human_duration(Duration::from_secs(9)), "9s");
        assert_eq!(human_duration(Duration::from_secs(102)), "1m 42s");
    }

    #[test]
    fn only_four_devices_are_replayable() {
        let ids: Vec<&str> = replayable_devices().iter().map(|d| d.id).collect();
        assert_eq!(ids, vec!["pro-800", "poly-d", "lm-drum", "crave", "model-d"]);
    }
}
