//! Headless interface: CLI commands and an MCP stdio server.
//!
//! The device binaries call [`main_for`]. With no arguments the GUI starts;
//! with a command the same descriptor is driven from a terminal so scripts and
//! agents can read, write and monitor the instrument.
//!
//! ```text
//! opentribe-poly-d                     GUI
//! opentribe-poly-d list-ports
//! opentribe-poly-d params --json
//! opentribe-poly-d read --json
//! opentribe-poly-d get key_priority
//! opentribe-poly-d set clock_source usb
//! opentribe-poly-d send "2F 01"
//! opentribe-poly-d monitor --json
//! opentribe-poly-d mcp
//! ```

use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::descriptor::{self, Device, Kind, Param, Values};
use crate::midi::{self, MidiHub};
use crate::sysex::{self, WriteStyle};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Print to stdout, ignoring a closed pipe (e.g. `| head`).
macro_rules! outln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}
const MCP_PROTOCOL: &str = "2025-06-18";
const READ_TIMEOUT: Duration = Duration::from_millis(2000);
const ACK_TIMEOUT: Duration = Duration::from_millis(500);

/// Entry point for the device binaries.
pub fn main_for(device: &'static Device) -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match parse(device, &args) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("{err}\n\n{}", help(device));
            return 2;
        }
    };
    let result = match cli.command {
        Command::Gui => {
            crate::run(device);
            return 0;
        }
        Command::Help => {
            outln!("{}", help(device));
            return 0;
        }
        Command::Version => {
            outln!("{} {} ({})", device.name, VERSION, device.id);
            return 0;
        }
        Command::ListPorts => list_ports(),
        Command::Params { json } => list_params(device, json),
        Command::Read { json } => {
            with_session(device, cli.port.as_deref(), cli.device_id, |session| {
                read_all(device, session, json)
            })
        }
        Command::Get { id, json } => {
            with_session(device, cli.port.as_deref(), cli.device_id, |session| {
                get_param(device, session, &id, json)
            })
        }
        Command::Set { id, value, dry_run } => {
            if dry_run {
                set_param_dry(device, &id, &value)
            } else {
                with_session(device, cli.port.as_deref(), cli.device_id, |session| {
                    set_param(device, session, &id, &value)
                })
            }
        }
        Command::Send { hex } => {
            with_session(device, cli.port.as_deref(), cli.device_id, |session| {
                send_hex(device, session, &hex)
            })
        }
        Command::Monitor { json, duration_ms } => {
            with_session(device, cli.port.as_deref(), cli.device_id, |session| {
                monitor(device, session, json, duration_ms)
            })
        }
        Command::Mcp => McpServer::new(device, cli.device_id, cli.port.clone()).run(),
    };
    match result {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

// --------------------------------------------------------------------- args

#[derive(Debug, PartialEq)]
enum Command {
    Gui,
    Help,
    Version,
    ListPorts,
    Params {
        json: bool,
    },
    Read {
        json: bool,
    },
    Get {
        id: String,
        json: bool,
    },
    Set {
        id: String,
        value: String,
        dry_run: bool,
    },
    Send {
        hex: String,
    },
    Monitor {
        json: bool,
        duration_ms: Option<u64>,
    },
    Mcp,
}

#[derive(Debug, PartialEq)]
struct Cli {
    port: Option<String>,
    device_id: u8,
    command: Command,
}

fn parse(device: &'static Device, args: &[String]) -> Result<Cli, String> {
    let mut port = None;
    let mut device_id = 0;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--port" => {
                port = Some(args.get(index + 1).cloned().ok_or("--port needs a value")?);
                index += 2;
            }
            "--device-id" => {
                let text = args.get(index + 1).ok_or("--device-id needs a value")?;
                let value: u8 = text
                    .parse()
                    .map_err(|_| format!("--device-id expects 0-127, got {text:?}"))?;
                if value > 0x7F {
                    return Err("--device-id expects 0-127".to_string());
                }
                device_id = value;
                index += 2;
            }
            "-h" | "--help" => {
                return Ok(Cli {
                    port,
                    device_id,
                    command: Command::Help,
                });
            }
            "-V" | "--version" => {
                return Ok(Cli {
                    port,
                    device_id,
                    command: Command::Version,
                });
            }
            flag if flag.starts_with('-') => {
                return Err(format!("unknown global flag {flag:?}"));
            }
            _ => break,
        }
    }

    if index >= args.len() {
        return Ok(Cli {
            port,
            device_id,
            command: Command::Gui,
        });
    }

    let name = args[index].as_str();
    let rest = &args[index + 1..];
    let mut positional: Vec<&str> = Vec::new();
    let mut json_flag = false;
    let mut dry_run = false;
    let mut duration_ms = None;
    let mut cursor = 0;
    while cursor < rest.len() {
        match rest[cursor].as_str() {
            "--json" => json_flag = true,
            "--dry-run" => dry_run = true,
            "--duration-ms" => {
                let text = rest.get(cursor + 1).ok_or("--duration-ms needs a value")?;
                let value: u64 = text
                    .parse()
                    .map_err(|_| format!("--duration-ms expects milliseconds, got {text:?}"))?;
                duration_ms = Some(value);
                cursor += 1;
            }
            "-h" | "--help" => {
                return Ok(Cli {
                    port,
                    device_id,
                    command: Command::Help,
                });
            }
            flag if flag.starts_with('-') => {
                return Err(format!("unknown flag {flag:?} for {name}"));
            }
            value => positional.push(value),
        }
        cursor += 1;
    }

    let command = match name {
        "gui" => Command::Gui,
        "help" => Command::Help,
        "version" => Command::Version,
        "list-ports" => Command::ListPorts,
        "params" => Command::Params { json: json_flag },
        "read" => Command::Read { json: json_flag },
        "get" => Command::Get {
            id: positional.first().copied().unwrap_or_default().to_string(),
            json: json_flag,
        },
        "set" => Command::Set {
            id: positional.first().copied().unwrap_or_default().to_string(),
            value: positional.get(1).copied().unwrap_or_default().to_string(),
            dry_run,
        },
        "send" => Command::Send {
            hex: positional.concat(),
        },
        "monitor" => Command::Monitor {
            json: json_flag,
            duration_ms,
        },
        "mcp" => Command::Mcp,
        other => return Err(format!("unknown command {other:?}")),
    };

    // Validate required positionals early so callers get a usage error.
    match &command {
        Command::Get { id, .. } if id.is_empty() => {
            return Err("get needs a parameter id (see `params`)".to_string());
        }
        Command::Set { id, .. } if id.is_empty() => {
            return Err("set needs a parameter id and a value (see `params`)".to_string());
        }
        Command::Set { value, .. } if value.is_empty() => {
            return Err("set needs a value (number, label, on/off)".to_string());
        }
        Command::Send { hex } if hex.trim().is_empty() => {
            return Err("send needs a hex string".to_string());
        }
        _ => {}
    }

    validate_ids(device, &command)?;
    Ok(Cli {
        port,
        device_id,
        command,
    })
}

fn validate_ids(device: &Device, command: &Command) -> Result<(), String> {
    let id = match command {
        Command::Get { id, .. } | Command::Set { id, .. } => id,
        _ => return Ok(()),
    };
    if device.param(id).is_none() {
        return Err(format!(
            "unknown parameter {id:?}; ids for {}: {}",
            device.name,
            device
                .params
                .iter()
                .map(|param| param.id)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}

fn help(device: &'static Device) -> String {
    let bin = format!("opentribe-{}", device.id);
    format!(
        "{name} {version} — opentribe controller\n\
         \n\
         Usage:\n  \
         {bin} [--port <substring>] [--device-id <0-127>] [command]\n\
         \n\
         With no command the GUI starts.\n\
         \n\
         Commands:\n  \
         gui                        start the GUI\n  \
         list-ports                 MIDI outputs and inputs\n  \
         params [--json]            parameter ids, kinds and defaults\n  \
         read [--json]              read all settings from the device\n  \
         get <id> [--json]          one parameter\n  \
         set <id> <value>           write one parameter (label, number, on/off)\n  \
         set <id> <value> --dry-run print the message without opening the device\n  \
         send <hex>                 raw SysEx; a body without F0 is wrapped\n  \
         monitor [--json] [--duration-ms N]\n  \
         mcp                        MCP server on stdio\n  \
         version | help\n\
         \n\
         Flags:\n  \
         --port <substring>         pick a port by name (default: {hint})\n  \
         --device-id <0-127>        SysEx device id (default 0)\n",
        name = device.name,
        version = VERSION,
        bin = bin,
        hint = device.port_hint,
    )
}

// ------------------------------------------------------------------ session

struct Session {
    device: &'static Device,
    device_id: u8,
    hub: MidiHub,
    port: String,
    style: WriteStyle,
}

impl Session {
    fn open(device: &'static Device, device_id: u8, hint: Option<&str>) -> Result<Self, String> {
        let ports = midi::output_ports().map_err(|err| format!("MIDI init failed: {err}"))?;
        let wanted = hint.unwrap_or(device.port_hint);
        let name = midi::guess_port(&ports, wanted).ok_or_else(|| {
            format!(
                "no MIDI output matching {wanted:?}; available: {}",
                if ports.is_empty() {
                    "none".to_string()
                } else {
                    ports.join(" | ")
                }
            )
        })?;
        let mut hub = MidiHub::default();
        hub.connect_output(&name)?;
        hub.connect_input(&name, || {})?;
        Ok(Self {
            device,
            device_id,
            hub,
            port: name,
            style: device
                .styles
                .first()
                .copied()
                .unwrap_or(WriteStyle::PacketDirect),
        })
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.hub.send(bytes)
    }

    fn read_values(&mut self, timeout: Duration) -> Result<Values, String> {
        let request = descriptor::read_request(self.device, self.device_id)
            .ok_or("this device has no documented settings read")?;
        self.send(&request)?;
        let expected = self.device.read.map(|read| read.block_len).unwrap_or(0);
        let deadline = Instant::now() + timeout;
        loop {
            for message in self.hub.poll() {
                let Some(parsed) = descriptor::parse(self.device, &message) else {
                    continue;
                };
                if let Some(block) = parsed.body.strip_prefix(&[sysex::op::SETTINGS_BLOCK]) {
                    if block.len() != expected {
                        return Err(format!(
                            "short settings block: {} of {expected} bytes",
                            block.len()
                        ));
                    }
                    let mut values = Values::defaults(self.device);
                    descriptor::apply_block(self.device, &mut values, block);
                    return Ok(values);
                }
            }
            if Instant::now() >= deadline {
                return Err("no settings block received (is the device wedged?)".to_string());
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    /// Wait briefly for an ACK and return its payload.
    fn await_ack(&mut self, timeout: Duration) -> Option<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        loop {
            for message in self.hub.poll() {
                let Some(parsed) = descriptor::parse(self.device, &message) else {
                    continue;
                };
                if descriptor::ack_code(parsed.body).is_some() {
                    return Some(parsed.body.to_vec());
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn poll_messages(&mut self) -> Vec<(Vec<u8>, String)> {
        self.hub
            .poll()
            .into_iter()
            .map(|bytes| {
                let label = descriptor::parse(self.device, &bytes)
                    .map(|message| {
                        descriptor::decode_reply(self.device, message.body)
                            .unwrap_or_else(|| "unknown reply".to_string())
                    })
                    .unwrap_or_else(|| match bytes.first() {
                        Some(0xF0) => "SysEx (other device)".to_string(),
                        Some(status) => {
                            format!("MIDI channel message (ch {})", (status & 0x0F) + 1)
                        }
                        None => "empty".to_string(),
                    });
                (bytes, label)
            })
            .collect()
    }
}

fn with_session<F>(
    device: &'static Device,
    hint: Option<&str>,
    device_id: u8,
    run: F,
) -> Result<(), String>
where
    F: FnOnce(&mut Session) -> Result<(), String>,
{
    let mut session = Session::open(device, device_id, hint)?;
    run(&mut session)
}

// ----------------------------------------------------------------- commands

fn list_ports() -> Result<(), String> {
    let outputs = midi::output_ports().map_err(|err| format!("MIDI init failed: {err}"))?;
    let inputs = midi::input_ports().map_err(|err| format!("MIDI init failed: {err}"))?;
    outln!("outputs:");
    if outputs.is_empty() {
        outln!("  (none)");
    }
    for name in outputs {
        outln!("  {name}");
    }
    outln!("inputs:");
    if inputs.is_empty() {
        outln!("  (none)");
    }
    for name in inputs {
        outln!("  {name}");
    }
    Ok(())
}

fn list_params(device: &'static Device, as_json: bool) -> Result<(), String> {
    let values = Values::defaults(device);
    if as_json {
        let params: Vec<Value> = values
            .entries()
            .map(|(param, value)| param_json(param, value))
            .collect();
        outln!(
            "{}",
            json!({
                "device": device.id,
                "name": device.name,
                "read": device.read.is_some(),
                "params": params,
            })
        );
        return Ok(());
    }
    outln!(
        "{:<24} {:<20} {:<12} {:<6} evidence",
        "id",
        "title",
        "kind",
        "default"
    );
    for (param, value) in values.entries() {
        outln!(
            "{:<24} {:<20} {:<12} {:<6} {}",
            param.id,
            param.title,
            kind_text(param.kind),
            param.kind.label(value),
            param.evidence.tag()
        );
    }
    Ok(())
}

fn read_all(device: &'static Device, session: &mut Session, as_json: bool) -> Result<(), String> {
    let values = session.read_values(READ_TIMEOUT)?;
    if as_json {
        outln!("{}", read_json(device, session, &values));
        return Ok(());
    }
    outln!(
        "{} on {} ({} block-backed parameters)",
        device.name,
        session.port,
        values.entries().count()
    );
    for (param, value) in values.entries() {
        outln!("{:<24} {}", param.id, param.kind.label(value));
    }
    Ok(())
}

fn read_json(device: &Device, session: &Session, values: &Values) -> Value {
    let entries: serde_json::Map<String, Value> = values
        .entries()
        .map(|(param, value)| (param.id.to_string(), json!(value)))
        .collect();
    json!({
        "device": device.id,
        "name": device.name,
        "port": session.port,
        "values": entries,
    })
}

fn get_param(
    device: &'static Device,
    session: &mut Session,
    id: &str,
    as_json: bool,
) -> Result<(), String> {
    let param = device
        .param(id)
        .ok_or_else(|| format!("unknown parameter {id:?}"))?;
    let values = session.read_values(READ_TIMEOUT)?;
    let value = values.get_or(param.id, param.default);
    if as_json {
        outln!(
            "{}",
            json!({
                "id": param.id,
                "value": value,
                "display": param.kind.label(value),
            })
        );
    } else {
        outln!("{} = {}", param.id, param.kind.label(value));
    }
    Ok(())
}

fn set_param(
    device: &'static Device,
    session: &mut Session,
    id: &str,
    text: &str,
) -> Result<(), String> {
    let mut values = session.read_values(READ_TIMEOUT)?;
    let outcome = apply_value(device, &mut values, id, text)?;
    let message = descriptor::write_message(
        device,
        &values,
        session.style,
        session.device_id,
        channel_for(device, &values),
        id,
    )
    .ok_or_else(|| format!("{id} has no write group"))?;
    session.send(&message)?;
    let ack = session.await_ack(ACK_TIMEOUT);
    print_set_result(device, id, outcome, &message, ack.as_deref());
    Ok(())
}

fn set_param_dry(device: &'static Device, id: &str, text: &str) -> Result<(), String> {
    let mut values = Values::defaults(device);
    let outcome = apply_value(device, &mut values, id, text)?;
    let style = device
        .styles
        .first()
        .copied()
        .unwrap_or(WriteStyle::PacketDirect);
    let message =
        descriptor::write_message(device, &values, style, 0, channel_for(device, &values), id)
            .ok_or_else(|| format!("{id} has no write group"))?;
    print_set_result(device, id, outcome, &message, None);
    Ok(())
}

/// Performance channel: the descriptor's channel parameter, else 1.
fn channel_for(device: &Device, values: &Values) -> u8 {
    match device.channel_param.and_then(|id| values.get(id)) {
        Some(channel) if (1..=16).contains(&channel) => channel,
        _ => 1,
    }
}

fn apply_value(device: &Device, values: &mut Values, id: &str, text: &str) -> Result<u8, String> {
    let param = device
        .param(id)
        .ok_or_else(|| format!("unknown parameter {id:?}"))?;
    let value = parse_value(param, text)?;
    values.set(param.id, value);
    Ok(value)
}

fn print_set_result(device: &Device, id: &str, value: u8, message: &[u8], ack: Option<&[u8]>) {
    let param = device.param(id);
    let display = param
        .map(|param| param.kind.label(value))
        .unwrap_or_default();
    outln!("{} = {value} ({display})", id);
    outln!("tx  {}", sysex::hex(message));
    match ack {
        Some(body) => outln!("rx  {}", sysex::hex(body)),
        None => outln!("rx  (no ACK within timeout)"),
    }
}

fn send_hex(device: &'static Device, session: &mut Session, hex: &str) -> Result<(), String> {
    let mut bytes = sysex::parse_hex(hex)?;
    if bytes.first() == Some(&0xF0) {
        if bytes.last() != Some(&0xF7) {
            bytes.push(0xF7);
        }
    } else {
        bytes = sysex::envelope(device.model, device.family, session.device_id, &bytes);
    }
    session.send(&bytes)?;
    let ack = session.await_ack(ACK_TIMEOUT);
    outln!("tx  {}", sysex::hex(&bytes));
    match ack {
        Some(body) => outln!("rx  {}", sysex::hex(&body)),
        None => outln!("rx  (no ACK within timeout)"),
    }
    Ok(())
}

fn monitor(
    device: &'static Device,
    session: &mut Session,
    as_json: bool,
    duration_ms: Option<u64>,
) -> Result<(), String> {
    let deadline = duration_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
    loop {
        for (bytes, label) in session.poll_messages() {
            if as_json {
                outln!(
                    "{}",
                    json!({
                        "label": label,
                        "bytes": sysex::hex(&bytes),
                        "device": device.id,
                    })
                );
            } else {
                outln!("{:<40} {}", label, sysex::hex(&bytes));
            }
            let _ = std::io::stdout().flush();
        }
        if let Some(deadline) = deadline
            && Instant::now() >= deadline
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ------------------------------------------------------------------- values

fn parse_value(param: &Param, text: &str) -> Result<u8, String> {
    let trimmed = text.trim();
    match param.kind {
        Kind::Enum(labels) => {
            if let Some(index) = labels
                .iter()
                .position(|label| label.eq_ignore_ascii_case(trimmed))
            {
                return Ok(index as u8);
            }
            if let Ok(value) = trimmed.parse::<u8>()
                && (value as usize) < labels.len()
            {
                return Ok(value);
            }
            Err(format!(
                "{} expects one of [{}] or 0..{}",
                param.id,
                labels.join(", "),
                labels.len().saturating_sub(1)
            ))
        }
        Kind::Bool => match trimmed.to_ascii_lowercase().as_str() {
            "on" | "true" | "yes" | "1" => Ok(1),
            "off" | "false" | "no" | "0" => Ok(0),
            other => Err(format!("{} expects on/off, got {other:?}", param.id)),
        },
        Kind::Byte { min, max } => {
            let value: u8 = trimmed
                .parse()
                .map_err(|_| format!("{} expects a number, got {trimmed:?}", param.id))?;
            if value < min || value > max {
                Err(format!("{} expects {min}..={max}, got {value}", param.id))
            } else {
                Ok(value)
            }
        }
    }
}

fn kind_text(kind: Kind) -> String {
    match kind {
        Kind::Byte { min, max } => format!("{min}..{max}"),
        Kind::Bool => "on/off".to_string(),
        Kind::Enum(labels) => labels.join("|"),
    }
}

fn kind_json(kind: Kind) -> Value {
    match kind {
        Kind::Byte { min, max } => json!({ "byte": [min, max] }),
        Kind::Bool => json!("bool"),
        Kind::Enum(labels) => json!({ "enum": labels }),
    }
}

fn param_json(param: &Param, value: u8) -> Value {
    json!({
        "id": param.id,
        "title": param.title,
        "kind": kind_json(param.kind),
        "default": param.default,
        "value": value,
        "tab": param.tab,
        "section": param.section,
        "evidence": param.evidence.tag(),
        "read_index": param.read_index,
    })
}

// ---------------------------------------------------------------------- MCP

struct McpServer {
    device: &'static Device,
    device_id: u8,
    hint: Option<String>,
    session: Option<Session>,
}

impl McpServer {
    fn new(device: &'static Device, device_id: u8, hint: Option<String>) -> Self {
        Self {
            device,
            device_id,
            hint,
            session: None,
        }
    }

    fn ensure_session(&mut self) -> Result<&mut Session, String> {
        if self.session.is_none() {
            self.session = Some(Session::open(
                self.device,
                self.device_id,
                self.hint.as_deref(),
            )?);
        }
        self.session
            .as_mut()
            .ok_or_else(|| "session unavailable".to_string())
    }

    fn run(&mut self) -> Result<(), String> {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        for line in stdin.lock().lines() {
            let line = line.map_err(|err| format!("stdin: {err}"))?;
            if line.trim().is_empty() {
                continue;
            }
            let request: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(err) => {
                    let response =
                        error_response(Value::Null, -32700, &format!("parse error: {err}"));
                    write_line(&mut stdout, &response)?;
                    continue;
                }
            };
            if let Some(response) = self.handle(request) {
                write_line(&mut stdout, &response)?;
            }
        }
        Ok(())
    }

    fn handle(&mut self, request: Value) -> Option<Value> {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        match method {
            "initialize" => Some(result_response(
                id,
                json!({
                    "protocolVersion": request
                        .get("params")
                        .and_then(|params| params.get("protocolVersion"))
                        .and_then(|value| value.as_str())
                        .unwrap_or(MCP_PROTOCOL),
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "opentribe", "version": VERSION },
                }),
            )),
            "notifications/initialized" | "notifications/cancelled" => None,
            "ping" => Some(result_response(id, json!({}))),
            "tools/list" => Some(result_response(id, json!({ "tools": tools() }))),
            "tools/call" => Some(self.call_tool(id, request.get("params").cloned())),
            other => Some(error_response(
                id,
                -32601,
                &format!("unknown method {other:?}"),
            )),
        }
    }

    fn call_tool(&mut self, id: Value, params: Option<Value>) -> Value {
        let name = params
            .as_ref()
            .and_then(|params| params.get("name"))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let arguments = params
            .as_ref()
            .and_then(|params| params.get("arguments"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        match self.dispatch(&name, &arguments) {
            Ok(value) => result_response(
                id,
                json!({
                    "content": [{ "type": "text", "text": value.to_string() }],
                    "isError": false,
                }),
            ),
            Err(err) => result_response(
                id,
                json!({
                    "content": [{ "type": "text", "text": err }],
                    "isError": true,
                }),
            ),
        }
    }

    fn dispatch(&mut self, name: &str, arguments: &Value) -> Result<Value, String> {
        match name {
            "list_params" => {
                let values = Values::defaults(self.device);
                Ok(json!({
                    "device": self.device.id,
                    "params": values
                        .entries()
                        .map(|(param, value)| param_json(param, value))
                        .collect::<Vec<_>>(),
                }))
            }
            "read_all" => {
                let device = self.device;
                let session = self.ensure_session()?;
                let values = session.read_values(READ_TIMEOUT)?;
                let port = session.port.clone();
                let mut out = read_json(device, session, &values);
                out["port"] = json!(port);
                Ok(out)
            }
            "get_param" => {
                let id = argument_str(arguments, "id")?;
                let param = self
                    .device
                    .param(&id)
                    .ok_or_else(|| format!("unknown parameter {id:?}"))?;
                let session = self.ensure_session()?;
                let values = session.read_values(READ_TIMEOUT)?;
                let value = values.get_or(param.id, param.default);
                Ok(json!({
                    "id": param.id,
                    "value": value,
                    "display": param.kind.label(value),
                }))
            }
            "set_param" => {
                let id = argument_str(arguments, "id")?;
                let value = arguments
                    .get("value")
                    .ok_or("set_param needs a value")?
                    .clone();
                let text = match value {
                    Value::String(text) => text,
                    other => other.to_string(),
                };
                let device = self.device;
                let session = self.ensure_session()?;
                let mut values = session.read_values(READ_TIMEOUT)?;
                let applied = apply_value(device, &mut values, &id, &text)?;
                let message = descriptor::write_message(
                    device,
                    &values,
                    session.style,
                    session.device_id,
                    channel_for(device, &values),
                    &id,
                )
                .ok_or_else(|| format!("{id} has no write group"))?;
                session.send(&message)?;
                let ack = session.await_ack(ACK_TIMEOUT);
                Ok(json!({
                    "id": id,
                    "value": applied,
                    "tx": sysex::hex(&message),
                    "rx": ack.as_deref().map(sysex::hex),
                }))
            }
            "send_sysex" => {
                let hex = argument_str(arguments, "hex")?;
                let mut bytes = sysex::parse_hex(&hex)?;
                if bytes.first() == Some(&0xF0) {
                    if bytes.last() != Some(&0xF7) {
                        bytes.push(0xF7);
                    }
                } else {
                    bytes = sysex::envelope(
                        self.device.model,
                        self.device.family,
                        self.device_id,
                        &bytes,
                    );
                }
                let session = self.ensure_session()?;
                session.send(&bytes)?;
                let ack = session.await_ack(ACK_TIMEOUT);
                Ok(json!({
                    "tx": sysex::hex(&bytes),
                    "rx": ack.as_deref().map(sysex::hex),
                }))
            }
            "monitor" => {
                let duration = arguments
                    .get("duration_ms")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(1000);
                let session = self.ensure_session()?;
                let deadline = Instant::now() + Duration::from_millis(duration);
                let mut messages = Vec::new();
                while Instant::now() < deadline {
                    for (bytes, label) in session.poll_messages() {
                        messages.push(json!({
                            "label": label,
                            "bytes": sysex::hex(&bytes),
                        }));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Ok(json!({ "messages": messages }))
            }
            other => Err(format!("unknown tool {other:?}")),
        }
    }
}

fn tools() -> Value {
    json!([
        {
            "name": "list_params",
            "description": "List parameter ids, kinds, defaults and read-back offsets.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "read_all",
            "description": "Read the settings block from the device and return every value.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "get_param",
            "description": "Read one parameter from the device.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "set_param",
            "description": "Write one parameter. Accepts a number, an enum label, or on/off.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "value": { "type": ["string", "integer"] }
                },
                "required": ["id", "value"],
                "additionalProperties": false
            }
        },
        {
            "name": "send_sysex",
            "description": "Send raw SysEx. A body without F0 is wrapped in the device envelope.",
            "inputSchema": {
                "type": "object",
                "properties": { "hex": { "type": "string" } },
                "required": ["hex"],
                "additionalProperties": false
            }
        },
        {
            "name": "monitor",
            "description": "Collect messages for a window and return them with decoded labels.",
            "inputSchema": {
                "type": "object",
                "properties": { "duration_ms": { "type": "integer", "minimum": 0 } },
                "additionalProperties": false
            }
        }
    ])
}

fn argument_str(arguments: &Value, key: &str) -> Result<String, String> {
    arguments
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("missing string argument {key:?}"))
}

fn result_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

fn write_line(out: &mut impl Write, value: &Value) -> Result<(), String> {
    writeln!(out, "{value}").map_err(|err| format!("stdout: {err}"))?;
    out.flush().map_err(|err| format!("stdout: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    static PARAMS: [Param; 3] = [
        Param {
            id: "clock.source",
            title: "Clock source",
            kind: Kind::Enum(&["Internal", "MIDI DIN", "MIDI USB", "TRIG"]),
            tab: "Routing",
            section: "Clock",
            write: "clock",
            default: 0,
            read_index: Some(5),
            evidence: crate::descriptor::Evidence::VendorNamed,
            hint: "",
        },
        Param {
            id: "autosave",
            title: "Auto save",
            kind: Kind::Bool,
            tab: "Routing",
            section: "Switches",
            write: "autosave",
            default: 1,
            read_index: Some(2),
            evidence: crate::descriptor::Evidence::VendorNamed,
            hint: "",
        },
        Param {
            id: "accent",
            title: "Accent threshold",
            kind: Kind::Byte { min: 0, max: 127 },
            tab: "Dynamics",
            section: "Dynamics",
            write: "accent",
            default: 96,
            read_index: Some(36),
            evidence: crate::descriptor::Evidence::Verified,
            hint: "",
        },
    ];

    static WRITES: [descriptor::WriteGroup; 3] = [
        descriptor::WriteGroup {
            id: "clock",
            addr: descriptor::Addr::Packet(0x1B),
            slots: &[descriptor::Slot::Param("clock.source")],
        },
        descriptor::WriteGroup {
            id: "autosave",
            addr: descriptor::Addr::PacketSub(0x74, 0x0B),
            slots: &[descriptor::Slot::Param("autosave")],
        },
        descriptor::WriteGroup {
            id: "accent",
            addr: descriptor::Addr::Packet(0x1C),
            slots: &[descriptor::Slot::Param("accent")],
        },
    ];

    static DEVICE: Device = Device {
        id: "test",
        name: "TEST",
        model: [0x00, 0x01, 0x28],
        family: 3,
        port_hint: "TEST",
        styles: &[WriteStyle::PacketDirect],
        read: Some(descriptor::ReadSpec {
            opcode: 0x75,
            reply: 0x76,
            block_len: 40,
        }),
        restore: None,
        channel_param: None,
        params: &PARAMS,
        writes: &WRITES,
        rejected: &[],
        reject_codes: &[],
        notes: &[],
        limits: &[],
    };

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn no_arguments_means_gui() {
        let cli = parse(&DEVICE, &[]).unwrap_or_else(|_| panic!("parses"));
        assert_eq!(cli.command, Command::Gui);
        assert_eq!(cli.device_id, 0);
    }

    #[test]
    fn global_flags_and_commands_parse() {
        let cli = parse(
            &DEVICE,
            &args(&[
                "--port",
                "LM",
                "--device-id",
                "7",
                "set",
                "clock.source",
                "usb",
            ]),
        )
        .unwrap_or_else(|_| panic!("parses"));
        assert_eq!(cli.port.as_deref(), Some("LM"));
        assert_eq!(cli.device_id, 7);
        assert_eq!(
            cli.command,
            Command::Set {
                id: "clock.source".to_string(),
                value: "usb".to_string(),
                dry_run: false
            }
        );

        let cli = parse(&DEVICE, &args(&["read", "--json"])).unwrap_or_else(|_| panic!("parses"));
        assert_eq!(cli.command, Command::Read { json: true });
    }

    #[test]
    fn usage_errors_are_reported() {
        assert!(parse(&DEVICE, &args(&["get"])).is_err());
        assert!(parse(&DEVICE, &args(&["set", "clock.source"])).is_err());
        assert!(parse(&DEVICE, &args(&["set", "nope", "1"])).is_err());
        assert!(parse(&DEVICE, &args(&["nope"])).is_err());
        assert!(parse(&DEVICE, &args(&["--device-id", "200", "read"])).is_err());
        assert!(parse(&DEVICE, &args(&["read", "--bogus"])).is_err());
    }

    #[test]
    fn values_accept_labels_numbers_and_booleans() {
        assert_eq!(parse_value(&PARAMS[0], "MIDI USB"), Ok(2));
        assert_eq!(parse_value(&PARAMS[0], "midi usb"), Ok(2));
        assert_eq!(parse_value(&PARAMS[0], "3"), Ok(3));
        assert!(parse_value(&PARAMS[0], "4").is_err());
        assert_eq!(parse_value(&PARAMS[1], "on"), Ok(1));
        assert_eq!(parse_value(&PARAMS[1], "off"), Ok(0));
        assert_eq!(parse_value(&PARAMS[2], "127"), Ok(127));
        assert!(parse_value(&PARAMS[2], "128").is_err());
    }

    #[test]
    fn param_json_carries_kind_and_evidence() {
        let value = param_json(&PARAMS[0], 2);
        assert_eq!(value["id"], "clock.source");
        assert_eq!(value["kind"]["enum"][2], "MIDI USB");
        assert_eq!(value["evidence"], "vendor-named");
        let value = param_json(&PARAMS[2], 96);
        assert_eq!(value["kind"]["byte"][1], 127);
        assert_eq!(value["evidence"], "verified");
    }

    #[test]
    fn mcp_initialize_and_tool_list() {
        let mut server = McpServer::new(&DEVICE, 0, None);
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": MCP_PROTOCOL, "capabilities": {}, "clientInfo": { "name": "test" } }
            }))
            .unwrap_or_else(|| panic!("initialize has a response"));
        assert_eq!(response["result"]["protocolVersion"], MCP_PROTOCOL);
        assert_eq!(response["result"]["serverInfo"]["name"], "opentribe");

        let response = server
            .handle(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
            .unwrap_or_else(|| panic!("tools/list has a response"));
        let names: Vec<&str> = response["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools array"))
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "list_params",
                "read_all",
                "get_param",
                "set_param",
                "send_sysex",
                "monitor"
            ]
        );
    }

    #[test]
    fn mcp_unknown_method_and_notifications() {
        let mut server = McpServer::new(&DEVICE, 0, None);
        assert!(
            server
                .handle(json!({ "jsonrpc": "2.0", "id": 3, "method": "nope" }))
                .unwrap_or_else(|| panic!("error response"))
                .get("error")
                .is_some()
        );
        assert!(
            server
                .handle(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
                .is_none()
        );
    }

    #[test]
    fn mcp_list_params_needs_no_device() {
        let mut server = McpServer::new(&DEVICE, 0, None);
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": { "name": "list_params", "arguments": {} }
            }))
            .unwrap_or_else(|| panic!("tool response"));
        assert_eq!(response["result"]["isError"], false);
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("text content"));
        let parsed: Value =
            serde_json::from_str(text).unwrap_or_else(|_| panic!("payload is JSON"));
        assert_eq!(
            parsed["params"]
                .as_array()
                .unwrap_or_else(|| panic!("params"))
                .len(),
            3
        );
    }

    #[test]
    fn help_mentions_every_command() {
        let text = help(&DEVICE);
        for needle in [
            "gui",
            "list-ports",
            "params",
            "read",
            "get",
            "set",
            "send",
            "monitor",
            "mcp",
            "opentribe-test",
        ] {
            assert!(text.contains(needle), "help is missing {needle}");
        }
    }
}
