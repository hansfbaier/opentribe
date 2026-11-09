//! Note-map text files: load and save the MIDI note slots of a device's
//! `note_map` write group.
//!
//! A file is plain text, one slot per line; `#` starts a comment:
//!
//! ```text
//! # opentribe note map · LM DRUM · slot = MIDI note (name)
//! slot01 = 36   # C2
//! slot02 = 40   # E2
//! ```
//!
//! Loading accepts `slotNN = <value>` and `note.slotNN = <value>` lines, bare
//! values (assigned in descriptor order, whitespace-separated values are fine),
//! decimal or `0x`-prefixed numbers, and note names (`C2`, `C#2`, `Db2`).
//! Every slot must be present exactly once; unknown, duplicate and missing
//! slots are errors, as are values outside 0-127.

use std::path::{Path, PathBuf};

use crate::descriptor::{Device, Param, Values};
use crate::widgets::note_name;

/// Id of the write group that carries the note map.
pub const WRITE_GROUP: &str = "note_map";

/// Parameters that travel in the device's note-map write group, in descriptor
/// order. Empty when the device has no note map.
pub fn slots(device: &Device) -> Vec<&'static Param> {
    device
        .params
        .iter()
        .filter(|param| param.write == WRITE_GROUP)
        .collect()
}

/// A note map loaded from a file, resolved against the device's slots.
#[derive(Clone, Debug)]
pub struct NoteMapFile {
    pub path: PathBuf,
    /// One MIDI note per slot, in descriptor order.
    pub notes: Vec<u8>,
}

/// Load a note-map text file. Nothing is sent to the device; the caller decides
/// when the values are written.
pub fn load_file(device: &Device, path: &Path) -> Result<NoteMapFile, String> {
    let text = std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let notes = parse(device, &text).map_err(|err| format!("{}:{err}", path.display()))?;
    Ok(NoteMapFile {
        path: path.to_path_buf(),
        notes,
    })
}

/// Save the current values as a note-map text file. Returns the slot count.
pub fn save_file(device: &Device, values: &Values, path: &Path) -> Result<usize, String> {
    let text = render(device, values)?;
    std::fs::write(path, text).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(slots(device).len())
}

/// Text written by [`save_file`].
fn render(device: &Device, values: &Values) -> Result<String, String> {
    let slots = slots(device);
    if slots.is_empty() {
        return Err("this device has no note map".to_string());
    }
    let mut text = format!(
        "# opentribe note map · {} · slot = MIDI note (name)\n",
        device.name
    );
    for (index, param) in slots.iter().enumerate() {
        let value = values.get_or(param.id, param.default).min(127);
        text.push_str(&format!(
            "slot{:02} = {value:>3}   # {}\n",
            index + 1,
            note_name(value)
        ));
    }
    Ok(text)
}

/// Slot values from the text, in descriptor order.
fn parse(device: &Device, text: &str) -> Result<Vec<u8>, String> {
    let slots = slots(device);
    if slots.is_empty() {
        return Err("this device has no note map".to_string());
    }
    let mut notes: Vec<Option<u8>> = vec![None; slots.len()];
    let mut next_bare = 0usize;
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = strip_comment(raw.trim()).trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let slot = slot_index(slots.len(), key).ok_or_else(|| {
                format!(
                    "{number}: unknown slot {:?} (expected slot01..slot{:02})",
                    key.trim(),
                    slots.len()
                )
            })?;
            if notes[slot].is_some() {
                return Err(format!("{number}: slot{:02} appears twice", slot + 1));
            }
            notes[slot] = Some(parse_value(value).map_err(|err| format!("{number}: {err}"))?);
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() == 2
            && let Some(slot) = slot_index(slots.len(), tokens[0])
        {
            if notes[slot].is_some() {
                return Err(format!("{number}: slot{:02} appears twice", slot + 1));
            }
            notes[slot] = Some(parse_value(tokens[1]).map_err(|err| format!("{number}: {err}"))?);
            continue;
        }
        for token in tokens {
            while next_bare < slots.len() && notes[next_bare].is_some() {
                next_bare += 1;
            }
            if next_bare >= slots.len() {
                return Err(format!("{number}: more than {} note values", slots.len()));
            }
            notes[next_bare] = Some(parse_value(token).map_err(|err| format!("{number}: {err}"))?);
            next_bare += 1;
        }
    }

    let missing: Vec<String> = notes
        .iter()
        .enumerate()
        .filter(|(_, note)| note.is_none())
        .map(|(index, _)| format!("slot{:02}", index + 1))
        .collect();
    if !missing.is_empty() {
        return Err(format!("missing {}", missing.join(", ")));
    }
    Ok(notes.into_iter().flatten().collect())
}

/// Remove a comment: `#` alone or preceded by whitespace. A sharp sign in a
/// note name (`C#2`) is not a comment.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'#' && (index == 0 || bytes[index - 1].is_ascii_whitespace()) {
            return line[..index].trim_end();
        }
    }
    line
}

/// Resolve `slotNN` / `note.slotNN` (1-based) to a zero-based slot index.
fn slot_index(count: usize, key: &str) -> Option<usize> {
    let key = key.trim().to_ascii_lowercase();
    let key = key.strip_prefix("note.").unwrap_or(&key);
    let digits = key.strip_prefix("slot")?;
    let slot: usize = digits.parse().ok()?;
    (1..=count).contains(&slot).then_some(slot - 1)
}

/// Parse a decimal / `0x` number or a note name.
fn parse_value(text: &str) -> Result<u8, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("empty value".to_string());
    }
    let number = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else if text.bytes().all(|byte| byte.is_ascii_digit()) {
        text.parse::<u32>().ok()
    } else {
        None
    };
    if let Some(number) = number {
        return u8::try_from(number)
            .ok()
            .filter(|note| *note <= 127)
            .ok_or_else(|| format!("{text} is outside the MIDI note range 0-127"));
    }
    parse_note_name(text)
        .ok_or_else(|| format!("{text} is not a MIDI note (0-127, 0x24, C2, C#2 or Db2)"))
}

/// `C-1` is 0, `G9` is 127. Sharps and flats are both accepted.
fn parse_note_name(text: &str) -> Option<u8> {
    let mut chars = text.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let mut semitone = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let mut rest = chars.as_str();
    if let Some(after) = rest.strip_prefix('#') {
        semitone += 1;
        rest = after;
    } else if let Some(after) = rest.strip_prefix('b') {
        semitone -= 1;
        rest = after;
    }
    let octave: i16 = rest.parse().ok()?;
    let note = (octave + 1) * 12 + semitone;
    u8::try_from(note).ok().filter(|note| *note <= 127)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{Addr, Evidence, Kind, Param, Slot, WriteGroup};
    use crate::sysex::WriteStyle;

    static PARAMS: [Param; 3] = [
        Param {
            id: "note.slot01",
            title: "Note slot 1",
            kind: Kind::Byte { min: 0, max: 127 },
            tab: "Note map",
            section: "Slots",
            write: WRITE_GROUP,
            default: 36,
            read_index: Some(0),
            evidence: Evidence::Verified,
            hint: "",
        },
        Param {
            id: "note.slot02",
            title: "Note slot 2",
            kind: Kind::Byte { min: 0, max: 127 },
            tab: "Note map",
            section: "Slots",
            write: WRITE_GROUP,
            default: 40,
            read_index: Some(1),
            evidence: Evidence::Verified,
            hint: "",
        },
        Param {
            id: "note.slot03",
            title: "Note slot 3",
            kind: Kind::Byte { min: 0, max: 127 },
            tab: "Note map",
            section: "Slots",
            write: WRITE_GROUP,
            default: 41,
            read_index: Some(2),
            evidence: Evidence::Verified,
            hint: "",
        },
    ];

    static SLOTS: [Slot; 3] = [
        Slot::Param("note.slot01"),
        Slot::Param("note.slot02"),
        Slot::Param("note.slot03"),
    ];

    static WRITES: [WriteGroup; 1] = [WriteGroup {
        id: WRITE_GROUP,
        addr: Addr::Packet(0x29),
        slots: &SLOTS,
    }];

    static DEVICE: Device = Device {
        id: "test",
        name: "Test",
        model: [0x00, 0x00, 0x00],
        family: 3,
        port_hint: "test",
        styles: &[WriteStyle::PacketDirect],
        read: None,
        restore: None,
        channel_param: None,
        params: &PARAMS,
        writes: &WRITES,
        samples: None,
        rejected: &[],
        reject_codes: &[],
        notes: &[],
        limits: &[],
    };

    #[test]
    fn slots_come_from_the_note_map_write_group() {
        let ids: Vec<&str> = slots(&DEVICE).iter().map(|param| param.id).collect();
        assert_eq!(ids, ["note.slot01", "note.slot02", "note.slot03"]);
    }

    #[test]
    fn keyed_file_parses_with_comments() {
        let text = "\
# LM DRUM note map
slot01 = 36   # C2
slot02 = 0x28 # E2
note.slot03 = C#2
";
        assert_eq!(parse(&DEVICE, text), Ok(vec![36, 40, 37]));
    }

    #[test]
    fn bare_values_fill_slots_in_order() {
        assert_eq!(parse(&DEVICE, "36 C2\nDb2\n"), Ok(vec![36, 36, 37]));
        assert_eq!(parse(&DEVICE, "slot01 36\n40 41\n"), Ok(vec![36, 40, 41]));
    }

    #[test]
    fn note_names_cover_the_midi_range() {
        assert_eq!(parse_value("C-1"), Ok(0));
        assert_eq!(parse_value("G9"), Ok(127));
        assert_eq!(parse_value("Cb1"), Ok(23));
        assert_eq!(parse_value("0x7F"), Ok(127));
        assert!(parse_value("G#9").is_err());
        assert!(parse_value("128").is_err());
        assert!(parse_value("H2").is_err());
    }

    #[test]
    fn partial_files_are_rejected() {
        let err = parse(&DEVICE, "slot01 = 36\nslot02 = 40\n").unwrap_err();
        assert!(err.contains("missing slot03"), "{err}");
    }

    #[test]
    fn unknown_and_duplicate_slots_are_rejected() {
        let err = parse(&DEVICE, "slot01 = 36\nslot02 = 40\nslot04 = 41\n").unwrap_err();
        assert!(err.contains("unknown slot"), "{err}");
        let text = "slot01 = 36\nslot01 = 40\nslot02 = 40\nslot03 = 41\n";
        let err = parse(&DEVICE, text).unwrap_err();
        assert!(err.contains("appears twice"), "{err}");
        let text = "slot01 = 36\nslot02 = 40\nslot03 = 41\n36\n";
        let err = parse(&DEVICE, text).unwrap_err();
        assert!(err.contains("more than 3"), "{err}");
    }

    #[test]
    fn saved_file_loads_back_to_the_same_notes() {
        let mut values = Values::defaults(&DEVICE);
        values.set("note.slot01", 0);
        values.set("note.slot02", 60);
        values.set("note.slot03", 127);
        let text = render(&DEVICE, &values).unwrap_or_else(|err| panic!("{err}"));
        assert!(text.contains("slot02 =  60   # C4"), "{text}");
        assert_eq!(parse(&DEVICE, &text), Ok(vec![0, 60, 127]));
    }

    #[test]
    fn file_round_trip_writes_and_reads_text() {
        let path =
            std::env::temp_dir().join(format!("opentribe-test-{}-notemap.txt", std::process::id()));
        let mut values = Values::defaults(&DEVICE);
        values.set("note.slot01", 1);
        values.set("note.slot02", 2);
        values.set("note.slot03", 3);
        assert_eq!(save_file(&DEVICE, &values, &path), Ok(3));
        let file = load_file(&DEVICE, &path).unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(file.notes, [1, 2, 3]);
        let _ = std::fs::remove_file(&path);
    }
}
