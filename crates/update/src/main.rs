//! `opentribe-update` — firmware replay for Behringer / Music Tribe instruments.
//!
//! ```text
//! opentribe-update devices
//! opentribe-update inspect --file pro800_1.4.6.syx
//! opentribe-update ports
//! opentribe-update status --device pro-800
//! opentribe-update flash --device pro-800 --file pro800_1.4.6.syx          (dry run)
//! opentribe-update flash --device pro-800 --file pro800_1.4.6.syx --send   (transmits)
//! ```
//!
//! `flash` without `--send` prints the plan and exits. With `--send` it walks the same steps
//! the vendor application does: mode query, optional `03 30` reboot, wait for the unit to come
//! back, then replay the file at the vendor pace.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tribe_core::midi::{self, MidiHub};
use tribe_update::flash::{self, Consent};
use tribe_update::firmware::Firmware;
use tribe_update::plan::{human_duration, Plan};
use tribe_update::{device, Device, VENDOR_PACE, VENDOR_RECONNECT_WAIT};

const VERSION: &str = env!("CARGO_PKG_VERSION");

macro_rules! outln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("opentribe-update: {err}");
            std::process::ExitCode::from(1)
        }
    }
}

struct FlashArgs {
    device: &'static Device,
    file: String,
    port: Option<String>,
    md5: Option<String>,
    pace: Duration,
    wait: Duration,
    reboot: bool,
    send: bool,
    json: bool,
}

fn run(args: &[String]) -> Result<std::process::ExitCode, String> {
    let command = args.first().map(String::as_str).unwrap_or("help");
    match command {
        "help" | "--help" | "-h" => {
            outln!("{}", help());
            Ok(std::process::ExitCode::SUCCESS)
        }
        "version" | "--version" | "-V" => {
            outln!("opentribe-update {VERSION}");
            Ok(std::process::ExitCode::SUCCESS)
        }
        "devices" => {
            list_devices(has_flag(args, "--json"));
            Ok(std::process::ExitCode::SUCCESS)
        }
        "ports" => {
            list_ports(has_flag(args, "--json"))?;
            Ok(std::process::ExitCode::SUCCESS)
        }
        "inspect" => {
            inspect(&args[1..])?;
            Ok(std::process::ExitCode::SUCCESS)
        }
        "status" => {
            status(&args[1..])?;
            Ok(std::process::ExitCode::SUCCESS)
        }
        "flash" => flash_command(&args[1..]),
        other => Err(format!("unknown command {other:?}\n\n{}", help())),
    }
}

fn help() -> String {
    let mut text = String::from(
        "opentribe-update — firmware replay for Behringer / Music Tribe instruments\n\n\
         usage:\n  \
         opentribe-update devices [--json]\n  \
         opentribe-update ports [--json]\n  \
         opentribe-update inspect --file FW.syx [--device ID] [--json]\n  \
         opentribe-update status --device ID [--port NAME] [--timeout-ms N] [--json]\n  \
         opentribe-update flash --device ID --file FW.syx [options]\n\n\
         flash options:\n  \
         --send              actually transmit (default: print the plan and stop)\n  \
         --port NAME         exact MIDI port name (default: match the device name)\n  \
         --md5 HEX           refuse to send unless the file has this MD5\n  \
         --pace-ms N         delay between messages (default: 10, the vendor timer)\n  \
         --wait SECS         wait for the bootloader port after a reboot (default: 8)\n  \
         --no-reboot         skip 03 30; the unit must already be in bootloader mode\n  \
         --json              machine-readable plan or report\n\n\
         devices:\n",
    );
    for dev in device::DEVICES {
        text.push_str(&format!(
            "  {:<10} {:<9} {:<26} {}\n",
            dev.id,
            dev.name,
            dev.flavour.label(),
            dev.usb_id().unwrap_or_else(|| "-".to_string())
        ));
    }
    text.push_str(
        "\nWrites are irreversible in the sense that a failed transfer leaves the unit in\n\
         bootloader mode until a successful one. Back up patches first.\n",
    );
    text
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn value_of<'a>(args: &'a [String], flag: &str) -> Result<Option<&'a str>, String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == flag {
            return iter
                .next()
                .map(|value| Some(value.as_str()))
                .ok_or_else(|| format!("{flag} needs a value"));
        }
        if let Some(rest) = arg.strip_prefix(&format!("{flag}=")) {
            return Ok(Some(rest));
        }
    }
    Ok(None)
}

fn list_devices(json_output: bool) {
    if json_output {
        let devices: Vec<Value> = device::DEVICES
            .iter()
            .map(|dev| {
                json!({
                    "id": dev.id,
                    "name": dev.name,
                    "usb": dev.usb_id(),
                    "pid_dfu": format!("{:04X}", dev.pid_dfu),
                    "model": tribe_core::sysex::hex(&dev.model),
                    "firmware_model": tribe_core::sysex::hex(&dev.firmware_model),
                    "family": dev.family,
                    "flavour": dev.flavour.label(),
                    "replayable": dev.flavour.replayable(),
                    "dfu_port": dev.dfu_port,
                })
            })
            .collect();
        outln!("{}", json!({ "devices": devices }));
        return;
    }
    outln!(
        "{:<10} {:<9} {:<28} {:<10} {}",
        "id",
        "device",
        "firmware flavour",
        "usb",
        "bootloader usb"
    );
    for dev in device::DEVICES {
        outln!(
            "{:<10} {:<9} {:<28} {:<10} {}",
            dev.id,
            dev.name,
            dev.flavour.label(),
            dev.usb_id().unwrap_or_else(|| "-".to_string()),
            if dev.pid_dfu == 0 {
                "-".to_string()
            } else {
                format!("{:04X}:{:04X}", dev.vid, dev.pid_dfu)
            }
        );
    }
    outln!(
        "\n{} of {} devices can be replayed; the NEUTRON is an STM32 USB DFU image, not SysEx.",
        device::DEVICES
            .iter()
            .filter(|dev| dev.flavour.replayable())
            .count(),
        device::DEVICES.len()
    );
}

fn list_ports(json_output: bool) -> Result<(), String> {
    let outputs = midi::output_ports()?;
    let inputs = midi::input_ports()?;
    if json_output {
        outln!("{}", json!({ "outputs": outputs, "inputs": inputs }));
        return Ok(());
    }
    outln!("output ports:");
    if outputs.is_empty() {
        outln!("  (none)");
    }
    for name in &outputs {
        outln!("  {name}");
    }
    outln!("input ports:");
    if inputs.is_empty() {
        outln!("  (none)");
    }
    for name in &inputs {
        outln!("  {name}");
    }
    Ok(())
}

fn resolve_device(args: &[String]) -> Result<&'static Device, String> {
    let query = value_of(args, "--device")?
        .or(value_of(args, "--id")?)
        .ok_or_else(|| "--device is required (see `opentribe-update devices`)".to_string())?;
    device::find(query).ok_or_else(|| format!("unknown device {query:?}"))
}

fn inspect(args: &[String]) -> Result<(), String> {
    let file = value_of(args, "--file")?.ok_or_else(|| "--file is required".to_string())?;
    let json_output = has_flag(args, "--json");
    let firmware = load(file)?;
    let summary = firmware.summary();
    if let Some(device) = value_of(args, "--device")? {
        let device = device::find(device).ok_or_else(|| format!("unknown device {device:?}"))?;
        summary.suits(device)?;
    }
    if json_output {
        outln!(
            "{}",
            json!({
                "file": summary.source,
                "bytes": summary.bytes,
                "md5": summary.md5,
                "messages": summary.messages,
                "model": tribe_core::sysex::hex(&summary.model),
                "opcode_index": summary.opcode_index,
                "shape": summary.shape.label(),
                "headers": summary.headers,
                "data_packets": summary.data_packets,
                "payload_bytes": summary.payload_bytes,
                "inno_commands": summary.inno_commands,
                "inno_blocks": summary.inno_blocks,
                "preamble": summary.preamble.iter().map(|m| tribe_core::sysex::hex(m)).collect::<Vec<_>>(),
                "segments": summary.segments.iter().map(|s| json!({
                    "index": s.index,
                    "packets": s.packets,
                    "payload_bytes": s.payload_bytes,
                    "counter_first": s.counter_first,
                    "counter_last": s.counter_last,
                    "header_bytes": s.header_bytes,
                    "last_message_len": s.last_message_len,
                    "product": s.product,
                    "name": s.name,
                    "version": s.version_string(),
                    "le32_a": format!("{:08X}", s.le32_a),
                    "le32_b": format!("{:08X}", s.le32_b),
                })).collect::<Vec<_>>(),
                "device": summary.matched_device().map(|d| d.id),
            })
        );
        return Ok(());
    }
    for line in summary.describe() {
        outln!("{line}");
    }
    Ok(())
}

fn status(args: &[String]) -> Result<(), String> {
    let device = resolve_device(args)?;
    let port = value_of(args, "--port")?;
    let timeout = Duration::from_millis(
        value_of(args, "--timeout-ms")?
            .map(|value| value.parse::<u64>())
            .transpose()
            .map_err(|err| format!("--timeout-ms: {err}"))?
            .unwrap_or(1500),
    );
    let json_output = has_flag(args, "--json");

    let mut hub = open(device, port)?;
    let status = flash::query_status(&mut hub, device, timeout)?;
    if json_output {
        outln!(
            "{}",
            json!({
                "device": device.id,
                "port": hub.output_name(),
                "mode": status.mode,
                "mode_label": status.mode_label(),
                "version": status.version_string(),
                "replies": status.replies.iter().map(|m| tribe_core::sysex::hex(m)).collect::<Vec<_>>(),
            })
        );
        return Ok(());
    }
    outln!("device      {} ({})", device.name, device.id);
    outln!("port        {}", hub.output_name().unwrap_or("?"));
    outln!(
        "mode        {} ({})",
        status.mode_label(),
        status
            .mode
            .map(|mode| format!("0x{mode:02X}"))
            .unwrap_or_else(|| "silent".to_string())
    );
    outln!(
        "firmware    {}",
        status.version_string().unwrap_or_else(|| "unknown".to_string())
    );
    for reply in &status.replies {
        outln!("reply       {}", tribe_core::sysex::hex(reply));
    }
    Ok(())
}

fn flash_command(args: &[String]) -> Result<std::process::ExitCode, String> {
    let device = resolve_device(args)?;
    let file = value_of(args, "--file")?.ok_or_else(|| "--file is required".to_string())?;
    let pace = Duration::from_millis(
        value_of(args, "--pace-ms")?
            .map(|value| value.parse::<u64>())
            .transpose()
            .map_err(|err| format!("--pace-ms: {err}"))?
            .unwrap_or(VENDOR_PACE.as_millis() as u64),
    );
    let wait = Duration::from_secs(
        value_of(args, "--wait")?
            .map(|value| value.parse::<u64>())
            .transpose()
            .map_err(|err| format!("--wait: {err}"))?
            .unwrap_or(VENDOR_RECONNECT_WAIT.as_secs()),
    );
    let arguments = FlashArgs {
        device,
        file: file.to_string(),
        port: value_of(args, "--port")?.map(str::to_string),
        md5: value_of(args, "--md5")?.map(str::to_string),
        pace,
        wait,
        reboot: !has_flag(args, "--no-reboot"),
        send: has_flag(args, "--send"),
        json: has_flag(args, "--json"),
    };

    let firmware = load(&arguments.file)?;
    if let Some(expected) = &arguments.md5
        && !tribe_update::md5::matches(firmware.bytes(), expected)
    {
        return Err(format!(
            "MD5 mismatch: file is {}, expected {expected}",
            firmware.md5()
        ));
    }
    let plan = Plan::build(
        arguments.device,
        &firmware,
        arguments.pace,
        arguments.reboot,
        arguments.wait,
    )?;

    if arguments.json {
        outln!("{}", plan.json());
    } else {
        for line in plan.describe() {
            outln!("{line}");
        }
    }

    if !arguments.send {
        if arguments.json {
            outln!("{}", json!({ "sent": false, "reason": "dry run" }));
        } else {
            outln!("\ndry run: nothing transmitted. add --send to write this firmware.");
        }
        return Ok(std::process::ExitCode::SUCCESS);
    }

    flash_now(&arguments, &firmware)
}

fn flash_now(arguments: &FlashArgs, firmware: &Firmware) -> Result<std::process::ExitCode, String> {
    let device = arguments.device;
    let mut hub = open(device, arguments.port.as_deref())?;
    if !arguments.json {
        outln!("\nport        {}", hub.output_name().unwrap_or("?"));
    }

    let status = flash::query_status(&mut hub, device, Duration::from_millis(1500))?;
    if !arguments.json {
        outln!(
            "before      {} / firmware {}",
            status.mode_label(),
            status.version_string().unwrap_or_else(|| "unknown".to_string())
        );
    }

    let already_in_dfu = status.mode == Some(0x31);
    if !already_in_dfu {
        if !arguments.reboot {
            return Err(format!(
                "{} reports {} and --no-reboot was given; refusing to send firmware to a unit \
                 that is not in bootloader mode",
                device.name,
                status.mode_label()
            ));
        }
        outln!("rebooting   sending 0x03 30, waiting up to {} for the bootloader port", human_duration(arguments.wait));
        let _ = flash::reboot_to_dfu(&mut hub, device, Duration::from_millis(250));
        drop(hub);
        hub = wait_for_dfu(device, arguments.port.as_deref(), arguments.wait)?;
        if !arguments.json {
            outln!("port        {} (bootloader)", hub.output_name().unwrap_or("?"));
        }
    }

    let total = firmware.message_count();
    let mut last_reported = 0usize;
    let result = flash::replay_consented(
        &mut hub,
        firmware,
        arguments.pace,
        Consent::given(),
        |sent, total| {
            // Report every 5% so a long LM DRUM transfer stays visibly alive.
            if sent == total || sent - last_reported >= (total / 20).max(1) {
                last_reported = sent;
                if !arguments.json {
                    outln!(
                        "sending     {sent}/{total} ({}%)",
                        sent.saturating_mul(100) / total.max(1)
                    );
                }
            }
        },
    )?;

    if arguments.json {
        outln!(
            "{}",
            json!({
                "sent": true,
                "device": device.id,
                "port": hub.output_name(),
                "messages": result.sent,
                "acks": result.acks,
                "replies": result.replies.iter().map(|m| tribe_core::sysex::hex(m)).collect::<Vec<_>>(),
                "seconds": result.elapsed.as_secs_f64(),
                "firmware": firmware.md5(),
                "notes": "power-cycle the instrument, then check `opentribe-update status`",
            })
        );
    } else {
        outln!(
            "\nsent        {} of {total} messages in {} ({} ACK frame(s))",
            result.sent,
            human_duration(result.elapsed),
            result.acks
        );
        outln!("next        power-cycle the instrument, then run `opentribe-update status --device {}`", device.id);
        outln!(
            "note        the transfer is complete when the last message is accepted; the device \
             verifies its own image after the power cycle"
        );
        if result.acks == 0 {
            outln!(
                "            no ACK frames arrived; the vendor app does not require them either, \
                 but treat a silent transfer as unverified"
            );
        }
    }

    Ok(std::process::ExitCode::SUCCESS)
}

fn open(device: &Device, explicit: Option<&str>) -> Result<MidiHub, String> {
    let name = match explicit {
        Some(name) => name.to_string(),
        None => match_port(device)?,
    };
    let mut hub = MidiHub::default();
    hub.connect_output(&name)?;
    // Input is best effort: the send path must work even if the port cannot be opened twice.
    let _ = hub.connect_input(&name, || {});
    Ok(hub)
}

fn match_port(device: &Device) -> Result<String, String> {
    let names = midi::output_ports()?;
    for candidate in device.port_candidates() {
        if let Some(found) = midi::guess_port(&names, candidate) {
            return Ok(found);
        }
    }
    Err(format!(
        "no MIDI port for {}; visible ports: {}",
        device.name,
        if names.is_empty() {
            "(none)".to_string()
        } else {
            names.join(", ")
        }
    ))
}

/// Poll for the bootloader port until `wait` expires, then require an answer to the mode query.
fn wait_for_dfu(
    device: &Device,
    explicit: Option<&str>,
    wait: Duration,
) -> Result<MidiHub, String> {
    let deadline = Instant::now() + wait;
    let mut last_error = String::new();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
        match open(device, explicit) {
            Ok(mut hub) => {
                match flash::query_mode(&mut hub, device, Duration::from_millis(400)) {
                    Ok(status) if status.mode == Some(0x31) => return Ok(hub),
                    Ok(status) => {
                        last_error = format!("port answers {} — not bootloader mode", status.mode_label());
                    }
                    Err(err) => last_error = err,
                }
            }
            Err(err) => last_error = err,
        }
    }
    Err(format!(
        "the bootloader port did not appear within {}: {last_error}\n\
         the unit is probably in bootloader mode without a usable MIDI port; power-cycle it and \
         retry, or pass --port/--no-reboot if you know the port name",
        human_duration(wait)
    ))
}

fn load(path: &str) -> Result<Firmware, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("{path}: {err}"))?;
    Firmware::parse(bytes, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn flags_parse_in_both_forms() {
        let list = args(&["flash", "--device", "pro-800", "--pace-ms=25"]);
        assert_eq!(value_of(&list, "--device").unwrap(), Some("pro-800"));
        assert_eq!(value_of(&list, "--pace-ms").unwrap(), Some("25"));
        assert!(value_of(&list, "--md5").unwrap().is_none());
        assert!(value_of(&args(&["x", "--md5"]), "--md5").is_err());
        assert!(!has_flag(&list, "--send"));
        assert!(has_flag(&args(&["flash", "--send"]), "--send"));
    }

    #[test]
    fn device_resolution_requires_the_flag() {
        assert!(resolve_device(&args(&["flash"])).is_err());
        assert!(resolve_device(&args(&["flash", "--device", "nope"])).is_err());
        assert_eq!(
            resolve_device(&args(&["flash", "--device", "lm drum"]))
                .expect("device")
                .id,
            "lm-drum"
        );
    }
}
