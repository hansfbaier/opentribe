//! Descriptor generator: validates `verified/<device>.toml` and writes
//! `crates/<device>/src/descriptors.rs`.
//!
//! Usage:
//!   cargo run -p xtask              # regenerate
//!   cargo run -p xtask -- --check   # verify the committed file is current
//!
//! The overlay is the only input. It is hand-written from the vendor QSG data
//! sheet and from measurements made on the instrument, so the build never reads
//! vendor software or data files. See PROVENANCE.md.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Deserialize;

#[derive(Deserialize)]
struct Overlay {
    device: DeviceCfg,
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default)]
    limits: Vec<String>,
    #[serde(default)]
    rejected: Vec<RejectedCfg>,
    #[serde(default)]
    labels: Vec<LabelsCfg>,
    #[serde(default)]
    write: Vec<WriteCfg>,
    #[serde(default)]
    param: Vec<ParamCfg>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceCfg {
    id: String,
    name: String,
    model: Vec<u8>,
    family: u8,
    port_hint: String,
    styles: Vec<String>,
    channel_param: Option<String>,
    restore: Option<u8>,
    #[serde(default)]
    reject_codes: Vec<u8>,
    read: Option<ReadCfg>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadCfg {
    opcode: u8,
    reply: u8,
    block_len: usize,
}

#[derive(Deserialize)]
struct RejectedCfg {
    name: String,
    packet: u8,
}

#[derive(Deserialize)]
struct LabelsCfg {
    name: String,
    values: Vec<String>,
}

#[derive(Deserialize)]
struct WriteCfg {
    id: String,
    packet: Option<u8>,
    sub: Option<u8>,
    cc: Option<u8>,
    slots: Vec<SlotCfg>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SlotCfg {
    Lit(u8),
    Param(String),
}

#[derive(Deserialize)]
struct ParamCfg {
    id: String,
    title: String,
    kind: KindCfg,
    tab: String,
    section: String,
    write: String,
    default: u8,
    read_index: Option<u8>,
    evidence: String,
    #[serde(default)]
    hint: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum KindCfg {
    Byte([u8; 2]),
    Bool(bool),
    #[serde(rename = "enum")]
    Enum(String),
}

fn main() -> ExitCode {
    let root = workspace_root();
    let check = std::env::args().any(|arg| arg == "--check");

    let mut overlays: Vec<PathBuf> = match std::fs::read_dir(root.join("verified")) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .collect(),
        Err(err) => {
            eprintln!("xtask: cannot read verified/: {err}");
            return ExitCode::FAILURE;
        }
    };
    overlays.sort();
    if overlays.is_empty() {
        eprintln!("xtask: no overlays in verified/");
        return ExitCode::FAILURE;
    }

    for overlay_path in overlays {
        if let Err(err) = build_device(&root, &overlay_path, check) {
            eprintln!("xtask: {err}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn build_device(root: &Path, overlay_path: &Path, check: bool) -> Result<(), String> {
    let stem = overlay_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("bad overlay name {}", overlay_path.display()))?;
    let out_path = root.join("crates").join(stem).join("src/descriptors.rs");

    let text = std::fs::read_to_string(overlay_path)
        .map_err(|err| format!("read {}: {err}", overlay_path.display()))?;
    let overlay: Overlay =
        toml::from_str(&text).map_err(|err| format!("parse {}: {err}", overlay_path.display()))?;
    validate(&overlay)?;

    let generated = emit(&overlay, stem)?;
    if check {
        let existing = std::fs::read_to_string(&out_path)
            .map_err(|err| format!("cannot read {}: {err}", out_path.display()))?;
        if existing == generated {
            println!("xtask: {} is up to date", out_path.display());
            Ok(())
        } else {
            Err(format!(
                "{} is stale; run `cargo run -p xtask`",
                out_path.display()
            ))
        }
    } else {
        std::fs::write(&out_path, &generated)
            .map_err(|err| format!("cannot write {}: {err}", out_path.display()))?;
        println!(
            "xtask: wrote {} ({} params, {} write groups)",
            out_path.display(),
            overlay.param.len(),
            overlay.write.len()
        );
        Ok(())
    }
}

fn workspace_root() -> PathBuf {
    // xtask lives in <root>/xtask, so the parent of CARGO_MANIFEST_DIR is the root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn validate(overlay: &Overlay) -> Result<(), String> {
    if overlay.device.model.len() != 3 {
        return Err("model must have exactly three bytes".to_string());
    }
    if overlay.param.is_empty() {
        return Err("no parameters defined".to_string());
    }

    let mut param_ids = HashSet::new();
    for param in &overlay.param {
        if !param_ids.insert(param.id.as_str()) {
            return Err(format!("duplicate parameter id {}", param.id));
        }
        if param.tab.trim().is_empty() || param.section.trim().is_empty() {
            return Err(format!("{} has an empty tab or section", param.id));
        }
        if !matches!(
            param.evidence.as_str(),
            "qsg" | "verified" | "vendor-named" | "unsupported"
        ) {
            return Err(format!(
                "{}: unknown evidence {:?}",
                param.id, param.evidence
            ));
        }
        check_kind(&param.kind)?;
        if let Some(index) = param.read_index {
            let block_len = overlay
                .device
                .read
                .as_ref()
                .map_or(0, |read| read.block_len);
            if usize::from(index) >= block_len {
                return Err(format!(
                    "{}: read_index {index} is outside the {block_len}-byte block",
                    param.id
                ));
            }
        }
        match &param.kind {
            KindCfg::Byte([min, max]) => {
                if param.default < *min || param.default > *max {
                    return Err(format!(
                        "{}: default {} outside {min}..={max}",
                        param.id, param.default
                    ));
                }
            }
            KindCfg::Bool(flag) => {
                if !flag {
                    return Err(format!(
                        "{}: bool kind must be written as {{ bool = true }}",
                        param.id
                    ));
                }
                if param.default > 1 {
                    return Err(format!("{}: bool default must be 0 or 1", param.id));
                }
            }
            KindCfg::Enum(name) => {
                let set = overlay
                    .labels
                    .iter()
                    .find(|set| set.name == *name)
                    .ok_or_else(|| format!("{}: unknown label set {name}", param.id))?;
                if usize::from(param.default) >= set.values.len() {
                    return Err(format!(
                        "{}: default {} outside label set {name}",
                        param.id, param.default
                    ));
                }
            }
        }
    }

    let labels: HashSet<&str> = overlay.labels.iter().map(|set| set.name.as_str()).collect();
    for param in &overlay.param {
        if let KindCfg::Enum(name) = &param.kind
            && !labels.contains(name.as_str())
        {
            return Err(format!("{}: unknown label set {name}", param.id));
        }
    }

    let mut write_ids = HashSet::new();
    for group in &overlay.write {
        if !write_ids.insert(group.id.as_str()) {
            return Err(format!("duplicate write group {}", group.id));
        }
        if group.packet.is_some() == group.cc.is_some() {
            return Err(format!(
                "write {}: set exactly one of `packet` or `cc`",
                group.id
            ));
        }
        if group.cc.is_some() && group.sub.is_some() {
            return Err(format!(
                "write {}: `sub` is only valid for packets",
                group.id
            ));
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for slot in &group.slots {
            if let SlotCfg::Param(id) = slot {
                if !param_ids.contains(id.as_str()) {
                    return Err(format!(
                        "write {}: slot references unknown parameter {id}",
                        group.id
                    ));
                }
                if !seen.insert(id.as_str()) {
                    return Err(format!("write {}: parameter {id} used twice", group.id));
                }
                let param = overlay
                    .param
                    .iter()
                    .find(|param| param.id == *id)
                    .ok_or_else(|| format!("write {}: unknown parameter {id}", group.id))?;
                if param.write != group.id {
                    return Err(format!(
                        "{}: write is {:?} but appears in group {:?}",
                        id, param.write, group.id
                    ));
                }
            }
        }
    }
    for param in &overlay.param {
        if !write_ids.contains(param.write.as_str()) {
            return Err(format!(
                "{}: unknown write group {:?}",
                param.id, param.write
            ));
        }
    }

    if let Some(channel) = &overlay.device.channel_param
        && !param_ids.contains(channel.as_str())
    {
        return Err(format!("channel_param {channel} is not a parameter"));
    }
    for style in &overlay.device.styles {
        if !matches!(style.as_str(), "packet-direct" | "long-envelope") {
            return Err(format!("unknown write style {style}"));
        }
    }
    if overlay.device.styles.is_empty() {
        return Err("device lists no write styles".to_string());
    }
    Ok(())
}

fn check_kind(kind: &KindCfg) -> Result<(), String> {
    if let KindCfg::Byte([min, max]) = kind
        && min > max
    {
        return Err(format!("byte kind has min {min} above max {max}"));
    }
    Ok(())
}

fn emit(overlay: &Overlay, overlay_name: &str) -> Result<String, String> {
    let mut out = String::new();
    let device = &overlay.device;
    let _ = writeln!(
        out,
        "// @generated by `cargo run -p xtask` from verified/{overlay_name}.toml — do not edit by hand."
    );
    let _ = writeln!(out, "//");
    let _ = writeln!(
        out,
        "// Sources: the vendor QSG data sheet and measurements on this"
    );
    let _ = writeln!(
        out,
        "// instrument. See PROVENANCE.md for the data lineage."
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "use tribe_core::descriptor::{{Addr, Device, Evidence, Kind, Param, ReadSpec, Slot, WriteGroup}};"
    );
    let _ = writeln!(out, "use tribe_core::sysex::WriteStyle;");
    let _ = writeln!(out);

    for labels in &overlay.labels {
        let _ = writeln!(
            out,
            "static LABELS_{}: [&str; {}] = [{}];",
            ident(&labels.name),
            labels.values.len(),
            labels
                .values
                .iter()
                .map(|value| format!("{value:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if !overlay.labels.is_empty() {
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "static PARAMS: [Param; {}] = [", overlay.param.len());
    for param in &overlay.param {
        let kind = match &param.kind {
            KindCfg::Byte([min, max]) => format!("Kind::Byte {{ min: {min}, max: {max} }}"),
            KindCfg::Bool(_) => "Kind::Bool".to_string(),
            KindCfg::Enum(name) => format!("Kind::Enum(&LABELS_{})", ident(name)),
        };
        let evidence = match param.evidence.as_str() {
            "verified" => "Evidence::Verified",
            "vendor-named" => "Evidence::VendorNamed",
            "unsupported" => "Evidence::Unsupported",
            _ => "Evidence::Qsg",
        };
        let read_index = match param.read_index {
            Some(index) => format!("Some({index})"),
            None => "None".to_string(),
        };
        let _ = writeln!(
            out,
            "    Param {{ id: {:?}, title: {:?}, kind: {kind}, tab: {:?}, section: {:?}, write: {:?}, default: {}, read_index: {read_index}, evidence: {evidence}, hint: {:?} }},",
            param.id, param.title, param.tab, param.section, param.write, param.default, param.hint
        );
    }
    let _ = writeln!(out, "];");
    let _ = writeln!(out);

    let _ = writeln!(
        out,
        "static WRITES: [WriteGroup; {}] = [",
        overlay.write.len()
    );
    for group in &overlay.write {
        let addr = match (group.cc, group.packet, group.sub) {
            (Some(cc), _, _) => format!("Addr::Cc({cc})"),
            (None, Some(packet), Some(sub)) => {
                format!("Addr::PacketSub({packet:#04X}, {sub:#04X})")
            }
            (None, Some(packet), None) => format!("Addr::Packet({packet:#04X})"),
            (None, None, _) => return Err(format!("write {}: missing packet or cc", group.id)),
        };
        let slots = group
            .slots
            .iter()
            .map(|slot| match slot {
                SlotCfg::Lit(byte) => format!("Slot::Lit({byte:#04X})"),
                SlotCfg::Param(id) => format!("Slot::Param({id:?})"),
            })
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "    WriteGroup {{ id: {:?}, addr: {addr}, slots: &[{slots}] }},",
            group.id
        );
    }
    let _ = writeln!(out, "];");
    let _ = writeln!(out);

    let styles = device
        .styles
        .iter()
        .map(|style| match style.as_str() {
            "long-envelope" => "WriteStyle::LongEnvelope",
            _ => "WriteStyle::PacketDirect",
        })
        .collect::<Vec<_>>()
        .join(", ");
    let read = match &device.read {
        Some(read) => format!(
            "Some(ReadSpec {{ opcode: {:#04X}, reply: {:#04X}, block_len: {} }})",
            read.opcode, read.reply, read.block_len
        ),
        None => "None".to_string(),
    };
    let model = device
        .model
        .iter()
        .map(|byte| format!("{byte:#04X}"))
        .collect::<Vec<_>>()
        .join(", ");
    let restore = match device.restore {
        Some(packet) => format!("Some({packet:#04X})"),
        None => "None".to_string(),
    };
    let channel = match &device.channel_param {
        Some(id) => format!("Some({id:?})"),
        None => "None".to_string(),
    };
    let reject_codes = device
        .reject_codes
        .iter()
        .map(|code| format!("{code:#04X}"))
        .collect::<Vec<_>>()
        .join(", ");
    let rejected = if overlay.rejected.is_empty() {
        "&[]".to_string()
    } else {
        format!(
            "&[{}]",
            overlay
                .rejected
                .iter()
                .map(|entry| format!("({:?}, {:#04X})", entry.name, entry.packet))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let _ = writeln!(out, "pub static DEVICE: &Device = &Device {{");
    let _ = writeln!(out, "    id: {:?},", device.id);
    let _ = writeln!(out, "    name: {:?},", device.name);
    let _ = writeln!(out, "    model: [{model}],");
    let _ = writeln!(out, "    family: {},", device.family);
    let _ = writeln!(out, "    port_hint: {:?},", device.port_hint);
    let _ = writeln!(out, "    styles: &[{styles}],");
    let _ = writeln!(out, "    read: {read},");
    let _ = writeln!(out, "    restore: {restore},");
    let _ = writeln!(out, "    channel_param: {channel},");
    let _ = writeln!(out, "    params: &PARAMS,");
    let _ = writeln!(out, "    writes: &WRITES,");
    let _ = writeln!(out, "    rejected: {rejected},");
    let _ = writeln!(out, "    reject_codes: &[{}],", reject_codes);
    let _ = writeln!(out, "    notes: &[{}],", string_slice(&overlay.notes));
    let _ = writeln!(out, "    limits: &[{}],", string_slice(&overlay.limits));
    let _ = writeln!(out, "}};");
    Ok(out)
}

fn string_slice(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("{value:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn ident(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}
