//! Headless interface: read, write, back up and compare PRO-800 programs.
//!
//! ```text
//! opentribe-pro-800                                  GUI
//! opentribe-pro-800 list-ports
//! opentribe-pro-800 status [--json]
//! opentribe-pro-800 params [--json]
//! opentribe-pro-800 slots [--bank A] [--json]
//! opentribe-pro-800 dump B094 [--out FILE]
//! opentribe-pro-800 backup --dir DIR [--verify]
//! opentribe-pro-800 inspect --file FILE [--json]
//! opentribe-pro-800 diff A000 B094
//! opentribe-pro-800 store --file FILE --slot B094 [--force]
//! opentribe-pro-800 copy --from A000 --to B094 [--force]
//! opentribe-pro-800 rename --slot B094 --name "Bass 2"
//! opentribe-pro-800 get filter.cutoff --slot B094
//! opentribe-pro-800 set filter.cutoff 640 --slot B094
//! opentribe-pro-800 globals [--json]
//! opentribe-pro-800 globals-set global.note_priority high
//! opentribe-pro-800 monitor [--duration-ms 3000]
//! opentribe-pro-800 send "F0 00 20 32 00 01 24 00 77 7E 03 F7"
//! ```

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use tribe_core::midi;
use tribe_core::sysex;

use crate::librarian::{self, Librarian, SlotState};
use crate::params::{self, GlobalParam, Kind, Param};
use crate::patch::{self, Patch};
use crate::transport::{self, Session};

const VERSION: &str = env!("CARGO_PKG_VERSION");

macro_rules! outln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

/// Failure with the process exit code the spec assigns it.
#[derive(Debug, PartialEq, Eq)]
struct Fail {
    code: u8,
    message: String,
}

impl Fail {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }

    fn device(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            message: message.into(),
        }
    }

    fn verify(message: impl Into<String>) -> Self {
        Self {
            code: 3,
            message: message.into(),
        }
    }
}

type Result<T> = std::result::Result<T, Fail>;

/// Entry point. Returns the process exit code.
pub fn main() -> u8 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("{}", err.message);
            err.code
        }
    }
}

fn dispatch(args: &[String]) -> Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        crate::run(None);
        return Ok(());
    };
    let rest = &args[1..];
    match command {
        "gui" => {
            crate::run(None);
            Ok(())
        }
        "--screenshot" => {
            let path = rest
                .first()
                .ok_or_else(|| Fail::usage("--screenshot needs a file name"))?;
            crate::run(Some(path));
            Ok(())
        }
        "-h" | "--help" | "help" => {
            outln!("{}", help());
            Ok(())
        }
        "-V" | "--version" | "version" => {
            outln!("opentribe-pro-800 {VERSION}");
            Ok(())
        }
        "list-ports" => list_ports(),
        "params" => cmd_params(rest),
        "status" => cmd_status(rest),
        "slots" => cmd_slots(rest),
        "dump" => cmd_dump(rest),
        "backup" => cmd_backup(rest),
        "inspect" => cmd_inspect(rest),
        "diff" => cmd_diff(rest),
        "store" => cmd_store(rest),
        "restore" => cmd_restore(rest),
        "delete" => cmd_delete(rest),
        "copy" => cmd_copy(rest),
        "rename" => cmd_rename(rest),
        "get" => cmd_get(rest),
        "set" => cmd_set(rest),
        "globals" => cmd_globals(rest),
        "globals-set" => cmd_globals_set(rest),
        "monitor" => cmd_monitor(rest),
        "send" => cmd_send(rest),
        other => Err(Fail::usage(format!(
            "unknown command {other:?}\n\n{}",
            help()
        ))),
    }
}

fn help() -> String {
    format!(
        "opentribe-pro-800 {VERSION} · Behringer {name}\n\
         \n\
         Usage:\n\
         \x20 opentribe-pro-800                                  GUI\n\
         \x20 opentribe-pro-800 gui                              GUI (explicit)\n\
         \x20 opentribe-pro-800 --screenshot FILE                draw, save a PNG, exit\n\
         \x20 opentribe-pro-800 list-ports\n\
         \x20 opentribe-pro-800 status [--json] [--port NAME]\n\
         \x20 opentribe-pro-800 params [--json]\n\
         \x20 opentribe-pro-800 slots [--bank A] [--json] [--port NAME]\n\
         \x20 opentribe-pro-800 dump B094 [--out FILE] [--port NAME]\n\
         \x20 opentribe-pro-800 backup --dir DIR [--verify] [--port NAME]\n\
         \x20 opentribe-pro-800 inspect --file FILE [--json]\n\
         \x20 opentribe-pro-800 diff A000 B094 [--port NAME]\n\
         \x20 opentribe-pro-800 store --file FILE --slot B094 [--force]\n\
         \x20 opentribe-pro-800 restore --dir DIR [--force] [--port NAME]\n\
         \x20 opentribe-pro-800 delete --slot B094\n\
         \x20 opentribe-pro-800 copy --from A000 --to B094 [--force]\n\
         \x20 opentribe-pro-800 rename --slot B094 --name \"Bass 2\"\n\
         \x20 opentribe-pro-800 get filter.cutoff --slot B094\n\
         \x20 opentribe-pro-800 set filter.cutoff 640 --slot B094\n\
         \x20 opentribe-pro-800 globals [--json]\n\
         \x20 opentribe-pro-800 globals-set global.note_priority high\n\
         \x20 opentribe-pro-800 monitor [--duration-ms 3000]\n\
         \x20 opentribe-pro-800 send \"F0 00 20 32 00 01 24 00 77 7E 03 F7\"\n\
         \n\
         Slot names: A000-D099, b94, 194 or 194/0xC2.\n\
         Every write is read back and compared; a mismatch exits with code 3.\n\
         Exit codes: 0 ok, 1 device error, 2 usage/validation, 3 verification.",
        name = crate::DEVICE_NAME
    )
}

// ------------------------------------------------------------------ parsing

struct Parsed {
    positional: Vec<String>,
    values: BTreeMap<String, String>,
    switches: HashSet<String>,
}

fn parse_args(args: &[String], value_flags: &[&str], switch_flags: &[&str]) -> Result<Parsed> {
    let mut parsed = Parsed {
        positional: Vec::new(),
        values: BTreeMap::new(),
        switches: HashSet::new(),
    };
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if let Some(name) = arg.strip_prefix("--") {
            if value_flags.contains(&name) {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| Fail::usage(format!("--{name} needs a value")))?;
                parsed.values.insert(name.to_string(), value.clone());
                index += 2;
                continue;
            }
            if switch_flags.contains(&name) {
                parsed.switches.insert(name.to_string());
                index += 1;
                continue;
            }
            return Err(Fail::usage(format!("unknown flag --{name}")));
        }
        parsed.positional.push(arg.clone());
        index += 1;
    }
    Ok(parsed)
}

impl Parsed {
    fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    fn required(&self, name: &str) -> Result<&str> {
        self.value(name)
            .ok_or_else(|| Fail::usage(format!("--{name} is required")))
    }

    fn switch(&self, name: &str) -> bool {
        self.switches.contains(name)
    }

    fn port(&self) -> Option<&str> {
        self.value("port")
    }
}

fn open(parsed: &Parsed) -> Result<Session> {
    Session::open(parsed.port()).map_err(|err| Fail::device(format!("MIDI: {err}")))
}

// ----------------------------------------------------------------- commands

fn list_ports() -> Result<()> {
    let outputs = midi::output_ports().map_err(Fail::device)?;
    let inputs = midi::input_ports().map_err(Fail::device)?;
    outln!("outputs:");
    for port in &outputs {
        outln!("  {port}");
    }
    outln!("inputs:");
    for port in &inputs {
        outln!("  {port}");
    }
    Ok(())
}

fn cmd_params(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &[], &["json"])?;
    if parsed.switch("json") {
        let list: Vec<Value> = params::PARAMS.iter().map(param_json).collect();
        outln!(
            "{}",
            json!({ "device": crate::DEVICE_NAME, "params": list })
        );
        return Ok(());
    }
    for section in params::SECTIONS {
        outln!("{section}");
        for param in params::section(section) {
            outln!(
                "  {:<18} {:<20} off {:<3} cc {:<3}  {}",
                param.id,
                param.title,
                param.offset,
                param.cc.map(|cc| cc.to_string()).unwrap_or_default(),
                param.evidence.tag()
            );
        }
    }
    Ok(())
}

fn param_json(param: &Param) -> Value {
    let kind = match param.kind {
        Kind::Knob => json!({ "kind": "knob", "min": 0, "max": 999 }),
        Kind::Byte => json!({ "kind": "byte", "min": 0, "max": 255 }),
        Kind::Bool | Kind::Bit => json!({ "kind": "bool" }),
        Kind::Enum(choices) => json!({
            "kind": "enum",
            "choices": choices.iter().map(|choice| json!({
                "value": choice.wire,
                "label": choice.label,
            })).collect::<Vec<_>>(),
        }),
    };
    json!({
        "id": param.id,
        "title": param.title,
        "section": param.section,
        "offset": param.offset,
        "cc": param.cc,
        "kind": kind,
        "evidence": param.evidence.tag(),
    })
}

fn cmd_status(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["port"], &["json"])?;
    let mut session = open(&parsed)?;
    let status = session.status().map_err(device_error)?;
    if parsed.switch("json") {
        outln!(
            "{}",
            json!({
                "device": crate::DEVICE_NAME,
                "output": session.out_name,
                "input": session.in_name,
                "firmware": status.firmware.map(|(a, b, c)| format!("{a}.{b}.{c}")),
                "mode": status.mode,
            })
        );
        return Ok(());
    }
    outln!("device   {}", crate::DEVICE_NAME);
    outln!("output   {}", session.out_name);
    outln!("input    {}", session.in_name);
    outln!("firmware {}", status.firmware_text());
    outln!(
        "mode     {}",
        match status.mode {
            Some(0) => "normal".to_string(),
            Some(mode) => format!("{mode:#04X}"),
            None => "unknown".to_string(),
        }
    );
    Ok(())
}

fn cmd_slots(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["bank", "port"], &["json"])?;
    let bank = match parsed.value("bank") {
        Some(text) => Some(bank_index(text)?),
        None => None,
    };
    let mut session = open(&parsed)?;
    let mut lib = Librarian::new();
    lib.scan(&mut session, bank, |_, _| {})
        .map_err(device_error)?;
    if parsed.switch("json") {
        let list: Vec<Value> = lib
            .slots()
            .iter()
            .enumerate()
            .filter(|(_, state)| !matches!(state, SlotState::Unknown))
            .map(|(index, state)| {
                json!({
                    "slot": patch::slot_label(index as u16),
                    "index": index,
                    "name": state.name(),
                    "empty": matches!(state, SlotState::Empty),
                    "bytes": state.patch().map(|patch| patch.bytes.len()),
                })
            })
            .collect();
        outln!("{}", json!({ "slots": list }));
        return Ok(());
    }
    for (index, state) in lib.slots().iter().enumerate() {
        match state {
            SlotState::Unknown => {}
            SlotState::Empty => outln!("{}  (empty)", patch::slot_label(index as u16)),
            SlotState::Patch(patch) => outln!(
                "{}  {}  [{} bytes]",
                patch::slot_label(index as u16),
                patch.name(),
                patch.bytes.len()
            ),
        }
    }
    Ok(())
}

fn cmd_dump(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["out", "port"], &[])?;
    let slot = parsed
        .positional
        .first()
        .ok_or_else(|| Fail::usage("dump needs a slot"))?;
    let index = patch::parse_slot(slot).map_err(Fail::usage)?;
    let mut session = open(&parsed)?;
    let patch = session
        .read_slot(index)
        .map_err(device_error)?
        .ok_or_else(|| Fail::device(format!("slot {} is empty", patch::slot_label(index))))?;
    let path = parsed
        .value("out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{}.syx", patch::slot_label(index))));
    write_bytes(&path, &patch.record_frame())?;
    outln!(
        "{}  {}  [{} bytes] -> {}",
        patch::slot_label(index),
        patch.name(),
        patch.bytes.len(),
        path.display()
    );
    Ok(())
}

fn cmd_backup(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["dir", "port"], &["verify"])?;
    let dir = PathBuf::from(parsed.required("dir")?);
    let mut session = open(&parsed)?;
    let mut lib = Librarian::new();
    let mut last = 0u16;
    let manifest = lib
        .backup(&mut session, &dir, |index, _| {
            if index / 20 != last / 20 {
                last = index;
                eprintln!("read {}", patch::slot_label(index));
            }
        })
        .map_err(device_error)?;
    outln!(
        "backup: {} programs -> {} ({})",
        lib.filled(),
        dir.display(),
        manifest.display()
    );
    if parsed.switch("verify") {
        let mut mismatches = 0;
        for index in 0..=patch::MAX_SLOT {
            let file = dir.join(format!("{}.syx", patch::slot_label(index)));
            let Ok(patches) = patch::load_syx(&file) else {
                continue;
            };
            let Some(stored) = patches.first() else {
                continue;
            };
            let read = session
                .read_slot(index)
                .map_err(device_error)?
                .ok_or_else(|| {
                    Fail::verify(format!("{} read back empty", patch::slot_label(index)))
                })?;
            if !patch::equivalent(&read.bytes, &stored.bytes) {
                mismatches += 1;
                eprintln!("{}: differs", patch::slot_label(index));
            }
        }
        if mismatches > 0 {
            return Err(Fail::verify(format!("{mismatches} slots differ")));
        }
        outln!("verify: all files match the device");
    }
    Ok(())
}

fn cmd_inspect(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["file"], &["json"])?;
    let path = PathBuf::from(parsed.required("file")?);
    let data =
        std::fs::read(&path).map_err(|err| Fail::usage(format!("{}: {err}", path.display())))?;
    let frames = patch::split_frames(&data);
    let patches = patch::patches_from_syx(&data);
    if parsed.switch("json") {
        let list: Vec<Value> = patches
            .iter()
            .map(|patch| {
                json!({
                    "index": patch.index,
                    "slot": patch::slot_label(patch.index),
                    "name": patch.name(),
                    "bytes": patch.bytes.len(),
                    "packed": patch.packed().len(),
                    "sha256": librarian::sha256_hex(&patch.bytes),
                })
            })
            .collect();
        outln!(
            "{}",
            json!({
                "file": path.display().to_string(),
                "frames": frames.len(),
                "patches": list,
            })
        );
        return Ok(());
    }
    outln!(
        "{}: {} bytes, {} SysEx frames, {} programs",
        path.display(),
        data.len(),
        frames.len(),
        patches.len()
    );
    for patch in &patches {
        outln!(
            "  {}  {:<16} [{} unpacked / {} packed]  {}",
            patch::slot_label(patch.index),
            patch.name(),
            patch.bytes.len(),
            patch.packed().len(),
            librarian::sha256_hex(&patch.bytes)
        );
    }
    Ok(())
}

fn cmd_diff(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["port"], &["json"])?;
    if parsed.positional.len() < 2 {
        return Err(Fail::usage("diff needs two slots or files"));
    }
    let needs_device = [&parsed.positional[0], &parsed.positional[1]]
        .iter()
        .any(|source| !looks_like_file(source));
    let mut session = if needs_device {
        Some(open(&parsed)?)
    } else {
        None
    };
    let left = load_source(&parsed.positional[0], session.as_mut())?;
    let right = load_source(&parsed.positional[1], session.as_mut())?;
    let changes = librarian::diff(&left, &right);
    if parsed.switch("json") {
        let list: Vec<Value> = changes
            .iter()
            .map(|change| {
                json!({
                    "offset": change.offset,
                    "left": change.left,
                    "right": change.right,
                })
            })
            .collect();
        outln!(
            "{}",
            json!({
                "left": { "slot": patch::slot_label(left.index), "name": left.name() },
                "right": { "slot": patch::slot_label(right.index), "name": right.name() },
                "changes": list,
            })
        );
        return Ok(());
    }
    outln!("left   {}  {}", patch::slot_label(left.index), left.name());
    outln!(
        "right  {}  {}",
        patch::slot_label(right.index),
        right.name()
    );
    outln!("{} changed byte(s)", changes.len());
    for change in changes {
        outln!(
            "  {:#05x}  {:<6} -> {}",
            change.offset,
            change
                .left
                .map(|byte| format!("{byte:#04x}"))
                .unwrap_or_else(|| "-".to_string()),
            change
                .right
                .map(|byte| format!("{byte:#04x}"))
                .unwrap_or_else(|| "-".to_string())
        );
    }
    Ok(())
}

fn cmd_store(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["file", "slot", "port"], &["force"])?;
    let path = PathBuf::from(parsed.required("file")?);
    let index = patch::parse_slot(parsed.required("slot")?).map_err(Fail::usage)?;
    let mut patches = patch::load_syx(&path).map_err(Fail::usage)?;
    let mut patch = patches
        .pop()
        .ok_or_else(|| Fail::usage("file has no program"))?;
    patch.index = index;
    let mut session = open(&parsed)?;
    ensure_empty(&mut session, index, parsed.switch("force"))?;
    let read_back = session.write_slot(&patch).map_err(write_error)?;
    outln!(
        "stored {}  {}  [{} bytes, read back {} bytes]",
        patch::slot_label(index),
        read_back.name(),
        patch.bytes.len(),
        read_back.bytes.len()
    );
    Ok(())
}

fn cmd_restore(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["dir", "port"], &["force"])?;
    let dir = PathBuf::from(parsed.required("dir")?);
    let mut session = open(&parsed)?;
    let mut lib = Librarian::new();
    let written = lib
        .restore(&mut session, &dir, parsed.switch("force"))
        .map_err(device_error)?;
    outln!(
        "restored {written} program(s) from {} and read each one back",
        dir.display()
    );
    Ok(())
}

fn cmd_delete(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["slot", "port"], &[])?;
    let index = patch::parse_slot(parsed.required("slot")?).map_err(Fail::usage)?;
    let mut session = open(&parsed)?;
    session.clear_slot(index).map_err(device_error)?;
    outln!("deleted {} (read back empty)", patch::slot_label(index));
    Ok(())
}

fn cmd_copy(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["from", "to", "port"], &["force"])?;
    let from = patch::parse_slot(parsed.required("from")?).map_err(Fail::usage)?;
    let to = patch::parse_slot(parsed.required("to")?).map_err(Fail::usage)?;
    let mut session = open(&parsed)?;
    let mut patch = session
        .read_slot(from)
        .map_err(device_error)?
        .ok_or_else(|| Fail::device(format!("{} is empty", patch::slot_label(from))))?;
    patch.index = to;
    ensure_empty(&mut session, to, parsed.switch("force"))?;
    let read_back = session.write_slot(&patch).map_err(write_error)?;
    outln!(
        "copied {} -> {}  {}",
        patch::slot_label(from),
        patch::slot_label(to),
        read_back.name()
    );
    Ok(())
}

fn cmd_rename(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["slot", "name", "port"], &[])?;
    let index = patch::parse_slot(parsed.required("slot")?).map_err(Fail::usage)?;
    let name = parsed.required("name")?;
    let mut session = open(&parsed)?;
    let mut patch = session
        .read_slot(index)
        .map_err(device_error)?
        .ok_or_else(|| Fail::device(format!("{} is empty", patch::slot_label(index))))?;
    patch.set_name(name).map_err(Fail::usage)?;
    let read_back = session.write_slot(&patch).map_err(write_error)?;
    outln!(
        "renamed {} -> {:?}",
        patch::slot_label(index),
        read_back.name()
    );
    Ok(())
}

fn cmd_get(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["slot", "port"], &["json"])?;
    let id = parsed
        .positional
        .first()
        .ok_or_else(|| Fail::usage("get needs a parameter id"))?;
    let index = patch::parse_slot(parsed.required("slot")?).map_err(Fail::usage)?;
    let param =
        params::param(id).ok_or_else(|| Fail::usage(format!("unknown parameter {id:?}")))?;
    let mut session = open(&parsed)?;
    let patch = session
        .read_slot(index)
        .map_err(device_error)?
        .ok_or_else(|| Fail::device(format!("{} is empty", patch::slot_label(index))))?;
    let value = params::read(&patch, param);
    if parsed.switch("json") {
        outln!(
            "{}",
            json!({
                "slot": patch::slot_label(index),
                "param": param.id,
                "value": value,
                "label": params::label(param, value),
                "evidence": param.evidence.tag(),
                "raw": raw_json(&patch, param),
            })
        );
        return Ok(());
    }
    outln!(
        "{}  {} = {} ({})",
        param.id,
        param.title,
        value,
        params::label(param, value)
    );
    Ok(())
}

fn cmd_set(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["slot", "port"], &["json"])?;
    if parsed.positional.len() < 2 {
        return Err(Fail::usage("set needs a parameter id and a value"));
    }
    let id = &parsed.positional[0];
    let text = &parsed.positional[1];
    let index = patch::parse_slot(parsed.required("slot")?).map_err(Fail::usage)?;
    let param =
        params::param(id).ok_or_else(|| Fail::usage(format!("unknown parameter {id:?}")))?;
    let value = parse_value(param, text).map_err(Fail::usage)?;
    let mut session = open(&parsed)?;
    let mut patch = session
        .read_slot(index)
        .map_err(device_error)?
        .ok_or_else(|| Fail::device(format!("{} is empty", patch::slot_label(index))))?;
    params::write(&mut patch, param, value);
    let read_back = session.write_slot(&patch).map_err(write_error)?;
    let after = params::read(&read_back, param);
    if after != value {
        return Err(Fail::verify(format!(
            "{} read back {after}, expected {value}",
            param.id
        )));
    }
    if parsed.switch("json") {
        outln!(
            "{}",
            json!({
                "slot": patch::slot_label(index),
                "param": param.id,
                "value": after,
                "label": params::label(param, after),
                "evidence": param.evidence.tag(),
            })
        );
        return Ok(());
    }
    outln!(
        "{}  {} = {} ({})  [{} offset {}]",
        param.id,
        param.title,
        after,
        params::label(param, after),
        param.evidence.tag(),
        param.offset
    );
    Ok(())
}

fn cmd_globals(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["port"], &["json"])?;
    let mut session = open(&parsed)?;
    let block = session.read_globals().map_err(device_error)?;
    if parsed.switch("json") {
        let values: Vec<Value> = params::GLOBALS
            .iter()
            .map(|param| {
                let value = block.get(param.offset).copied().unwrap_or(0);
                json!({
                    "id": param.id,
                    "title": param.title,
                    "offset": param.offset,
                    "value": value,
                    "writable": param.writable,
                    "evidence": param.evidence.tag(),
                })
            })
            .collect();
        outln!(
            "{}",
            json!({
                "hex": sysex::hex(&block),
                "program": program_from_globals(&block),
                "fields": values,
            })
        );
        return Ok(());
    }
    outln!("global block  [{} bytes]", block.len());
    for param in params::GLOBALS {
        let value = block.get(param.offset).copied().unwrap_or(0);
        let text = if param.id == "global.transpose" {
            format!("{}", value as i8)
        } else {
            format!("{value:<4} {:<10}", global_label(param, value))
        };
        outln!(
            "  {:<28} {:<28} off {:<2} {}{}",
            param.id,
            param.title,
            param.offset,
            text,
            if param.writable { "" } else { "  (read-only)" }
        );
    }
    outln!("program  {}", program_from_globals(&block));
    outln!("raw      {}", sysex::hex(&block));
    Ok(())
}

fn cmd_globals_set(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["port"], &["json"])?;
    if parsed.positional.len() < 2 {
        return Err(Fail::usage("globals-set needs a parameter id and a value"));
    }
    let id = &parsed.positional[0];
    let text = &parsed.positional[1];
    let param = params::GLOBALS
        .iter()
        .find(|param| param.id == id)
        .ok_or_else(|| Fail::usage(format!("unknown global parameter {id:?}")))?;
    if !param.writable {
        return Err(Fail::usage(format!(
            "{} is read-only until gate G3",
            param.id
        )));
    }
    let mut session = open(&parsed)?;
    let mut block = session.read_globals().map_err(device_error)?;
    let value = parse_global_value(param, text).map_err(Fail::usage)?;
    if param.offset >= block.len() {
        return Err(Fail::device("global block is too short for this field"));
    }
    block[param.offset] = value;
    let read_back = session.write_globals(&block).map_err(write_error)?;
    if parsed.switch("json") {
        outln!(
            "{}",
            json!({
                "param": param.id,
                "value": read_back.get(param.offset).copied().unwrap_or(0),
                "hex": sysex::hex(&read_back),
            })
        );
        return Ok(());
    }
    outln!(
        "{} = {} ({})  [global write is unverified]",
        param.id,
        read_back.get(param.offset).copied().unwrap_or(0),
        global_label(param, read_back.get(param.offset).copied().unwrap_or(0))
    );
    Ok(())
}

fn cmd_monitor(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["duration-ms", "port"], &[])?;
    let duration = parsed
        .value("duration-ms")
        .map(|text| text.parse::<u64>())
        .transpose()
        .map_err(|_| Fail::usage("--duration-ms needs a number"))?
        .map(Duration::from_millis);
    let inputs = midi::input_ports().map_err(Fail::device)?;
    let hint = parsed.port().unwrap_or(crate::PORT_HINT);
    let name = midi::guess_port(&inputs, hint)
        .or_else(|| inputs.first().cloned())
        .ok_or_else(|| Fail::device("no MIDI input ports visible"))?;
    let mut hub = tribe_core::midi::MidiHub::default();
    hub.connect_input(&name, || {}).map_err(Fail::device)?;
    outln!("listening on {name}");
    let deadline = duration.map(|duration| Instant::now() + duration);
    loop {
        for message in hub.poll() {
            outln!("{}\t{}", message.len(), sysex::hex(&message));
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn cmd_send(args: &[String]) -> Result<()> {
    let parsed = parse_args(args, &["port"], &["json"])?;
    let hex = parsed
        .positional
        .first()
        .ok_or_else(|| Fail::usage("send needs a hex message"))?;
    let bytes = transport::raw_frame(hex).map_err(Fail::usage)?;
    let mut session = open(&parsed)?;
    let replies = session.send_raw(&bytes).map_err(device_error)?;
    if parsed.switch("json") {
        outln!(
            "{}",
            json!({
                "sent": sysex::hex(&bytes),
                "replies": replies.iter().map(|reply| sysex::hex(reply)).collect::<Vec<_>>(),
            })
        );
        return Ok(());
    }
    outln!("sent    {}", sysex::hex(&bytes));
    for reply in &replies {
        outln!("reply   {}", sysex::hex(reply));
    }
    if replies.is_empty() {
        outln!("no reply");
    }
    Ok(())
}

// ------------------------------------------------------------------ helpers

fn device_error(err: String) -> Fail {
    Fail::device(err)
}

fn write_error(err: String) -> Fail {
    if err.contains("read-back") {
        Fail::verify(err)
    } else {
        Fail::device(err)
    }
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|err| Fail::usage(format!("{}: {err}", parent.display())))?;
    }
    std::fs::write(path, bytes).map_err(|err| Fail::usage(format!("{}: {err}", path.display())))
}

fn bank_index(text: &str) -> Result<u8> {
    let letter = text.trim().to_ascii_uppercase();
    let letter = letter.as_bytes().first().copied().unwrap_or(b'?');
    if !(b'A'..=b'D').contains(&letter) {
        return Err(Fail::usage(format!("bank {text:?} is not A-D")));
    }
    Ok(letter - b'A')
}

fn ensure_empty(session: &mut Session, index: u16, force: bool) -> Result<()> {
    if force {
        return Ok(());
    }
    if session.read_slot(index).map_err(device_error)?.is_some() {
        return Err(Fail::usage(format!(
            "{} is not empty; pass --force to overwrite",
            patch::slot_label(index)
        )));
    }
    Ok(())
}

fn looks_like_file(text: &str) -> bool {
    text.ends_with(".syx") || text.contains(std::path::MAIN_SEPARATOR)
}

fn load_source(text: &str, session: Option<&mut Session>) -> Result<Patch> {
    if looks_like_file(text) {
        let path = Path::new(text);
        let mut patches = patch::load_syx(path).map_err(Fail::usage)?;
        patches
            .pop()
            .ok_or_else(|| Fail::usage(format!("{text}: no program")))
    } else {
        let index = patch::parse_slot(text).map_err(Fail::usage)?;
        let session = session.ok_or_else(|| Fail::device("no MIDI session"))?;
        session
            .read_slot(index)
            .map_err(device_error)?
            .ok_or_else(|| Fail::device(format!("{} is empty", patch::slot_label(index))))
    }
}

fn parse_value(param: &Param, text: &str) -> std::result::Result<u32, String> {
    match param.kind {
        Kind::Knob => {
            let value: u32 = text
                .parse()
                .map_err(|_| format!("{} takes a number 0-999", param.id))?;
            if value > 999 {
                return Err(format!("{} takes a number 0-999", param.id));
            }
            Ok(value)
        }
        Kind::Byte => text
            .parse()
            .map_err(|_| format!("{} takes a byte 0-255", param.id)),
        Kind::Bool | Kind::Bit => match text.to_ascii_lowercase().as_str() {
            "on" | "true" | "1" | "yes" => Ok(1),
            "off" | "false" | "0" | "no" => Ok(0),
            _ => Err(format!("{} takes on/off", param.id)),
        },
        Kind::Enum(choices) => {
            if let Some(choice) = choices
                .iter()
                .find(|choice| choice.label.eq_ignore_ascii_case(text))
            {
                return Ok(u32::from(choice.wire));
            }
            if let Ok(number) = text.parse::<u8>()
                && let Some(choice) = choices.iter().find(|choice| choice.wire == number)
            {
                return Ok(u32::from(choice.wire));
            }
            Err(format!(
                "{} takes one of: {}",
                param.id,
                choices
                    .iter()
                    .map(|choice| choice.label)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
    }
}

fn parse_global_value(param: &GlobalParam, text: &str) -> std::result::Result<u8, String> {
    match param.kind {
        Kind::Enum(choices) => choices
            .iter()
            .find(|choice| choice.label.eq_ignore_ascii_case(text))
            .map(|choice| choice.wire)
            .or_else(|| text.parse::<u8>().ok())
            .ok_or_else(|| {
                format!(
                    "{} takes one of: {}",
                    param.id,
                    choices
                        .iter()
                        .map(|choice| choice.label)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }),
        _ => text
            .parse::<i16>()
            .map(|value| value as u8)
            .map_err(|_| format!("{} takes a number", param.id)),
    }
}

fn global_label(param: &GlobalParam, value: u8) -> String {
    match param.kind {
        Kind::Enum(choices) => choices
            .iter()
            .find(|choice| choice.wire == value)
            .map(|choice| choice.label.to_string())
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

fn program_from_globals(block: &[u8]) -> String {
    let lsb = block.get(5).copied().unwrap_or(0);
    let msb = block.get(6).copied().unwrap_or(0);
    let bank = block.get(20).copied().unwrap_or(0);
    let index = patch::global_index(lsb, msb);
    format!(
        "{} bank {} index {}",
        patch::slot_label(index),
        (b'A' + bank.min(3)) as char,
        index
    )
}

fn raw_json(patch: &Patch, param: &Param) -> Value {
    match param.kind {
        Kind::Knob => json!({
            "u16": patch.u16_at(param.offset),
            "bytes": patch.bytes.get(param.offset..param.offset + 2),
        }),
        _ => json!({ "byte": patch.byte_at(param.offset) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn flags_and_positionals_parse() {
        let parsed = parse_args(
            &args(&["filter.cutoff", "640", "--slot", "B094", "--json"]),
            &["slot"],
            &["json"],
        )
        .ok()
        .unwrap_or_else(|| panic!("parse"));
        assert_eq!(parsed.positional, ["filter.cutoff", "640"]);
        assert_eq!(parsed.value("slot"), Some("B094"));
        assert!(parsed.switch("json"));
    }

    #[test]
    fn unknown_flags_are_usage_errors() {
        assert!(parse_args(&args(&["--bogus"]), &[], &[]).is_err());
        assert!(parse_args(&args(&["--slot"]), &["slot"], &[]).is_err());
    }

    #[test]
    fn values_accept_labels_and_numbers() {
        let arp = params::param("arp.mode").unwrap_or_else(|| panic!("arp"));
        assert_eq!(parse_value(arp, "random"), Ok(5));
        assert_eq!(parse_value(arp, "2"), Ok(2));
        assert!(parse_value(arp, "sideways").is_err());
        let cutoff = params::param("filter.cutoff").unwrap_or_else(|| panic!("cutoff"));
        assert_eq!(parse_value(cutoff, "999"), Ok(999));
        assert!(parse_value(cutoff, "1000").is_err());
        let sync = params::param("osc_a.sync").unwrap_or_else(|| panic!("sync"));
        assert_eq!(parse_value(sync, "on"), Ok(1));
    }

    #[test]
    fn bank_letters_map_to_indices() {
        assert_eq!(bank_index("A"), Ok(0));
        assert_eq!(bank_index("d"), Ok(3));
        assert!(bank_index("E").is_err());
    }

    #[test]
    fn program_is_decoded_from_the_global_block() {
        let mut block = [0u8; 40];
        block[5] = 42;
        block[6] = 0;
        block[20] = 0;
        assert_eq!(program_from_globals(&block), "A042 bank A index 42");
        block[5] = 199;
        block[6] = 0;
        block[20] = 1;
        assert_eq!(program_from_globals(&block), "B099 bank B index 199");
    }

    #[test]
    fn help_names_every_command() {
        let text = help();
        for command in [
            "list-ports",
            "status",
            "params",
            "slots",
            "dump",
            "backup",
            "inspect",
            "diff",
            "store",
            "restore",
            "delete",
            "copy",
            "rename",
            "get",
            "set",
            "globals",
            "globals-set",
            "monitor",
            "send",
        ] {
            assert!(text.contains(command), "{command}");
        }
    }
}
