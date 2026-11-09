//! Bank scanning, backups, `.syx` import/export and patch comparison.
//!
//! A bank is 100 programs; the instrument holds four (`A`–`D`). The librarian
//! keeps whatever was read and never infers a name it did not see.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::patch::{self, MAX_SLOT, Patch, slot_label};
use crate::transport::Session;

/// What is known about one program address.
#[derive(Clone, Debug, Default)]
pub enum SlotState {
    /// Not read yet.
    #[default]
    Unknown,
    /// The device answered with no payload.
    Empty,
    /// A stored program.
    Patch(Patch),
}

impl SlotState {
    pub fn name(&self) -> Option<String> {
        match self {
            SlotState::Patch(patch) => Some(patch.name()),
            _ => None,
        }
    }

    pub fn patch(&self) -> Option<&Patch> {
        match self {
            SlotState::Patch(patch) => Some(patch),
            _ => None,
        }
    }
}

/// The 400 program addresses.
#[derive(Clone, Debug, Default)]
pub struct Librarian {
    slots: Vec<SlotState>,
}

impl Librarian {
    pub fn new() -> Self {
        Self {
            slots: vec![SlotState::Unknown; usize::from(MAX_SLOT) + 1],
        }
    }

    pub fn get(&self, index: u16) -> &SlotState {
        self.slots
            .get(usize::from(index))
            .unwrap_or(&SlotState::Unknown)
    }

    pub fn set(&mut self, index: u16, state: SlotState) {
        if let Some(slot) = self.slots.get_mut(usize::from(index)) {
            *slot = state;
        }
    }

    /// Put a patch into the librarian (used by the GUI's edit buffer).
    pub fn put(&mut self, patch: Patch) {
        self.set(patch.index, SlotState::Patch(patch));
    }

    pub fn slots(&self) -> &[SlotState] {
        &self.slots
    }

    /// Number of addresses holding a program.
    pub fn filled(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| matches!(slot, SlotState::Patch(_)))
            .count()
    }

    /// Read every address in a bank (0–3) or all four when `bank` is `None`.
    pub fn scan(
        &mut self,
        session: &mut Session,
        bank: Option<u8>,
        mut progress: impl FnMut(u16, &SlotState),
    ) -> Result<(), String> {
        let (first, last) = match bank {
            Some(bank) if bank < 4 => (u16::from(bank) * 100, u16::from(bank) * 100 + 99),
            Some(bank) => return Err(format!("bank {bank} is not 0-3")),
            None => (0, MAX_SLOT),
        };
        for index in first..=last {
            let state = match session.read_slot(index)? {
                Some(patch) => SlotState::Patch(patch),
                None => SlotState::Empty,
            };
            self.set(index, state.clone());
            progress(index, &state);
            std::thread::sleep(crate::transport::PACE);
        }
        Ok(())
    }

    /// Write every populated address to `dir` as `A000.syx` … plus a manifest.
    pub fn backup(
        &mut self,
        session: &mut Session,
        dir: &Path,
        mut progress: impl FnMut(u16, &SlotState),
    ) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        let mut manifest = String::from("slot,index,name,bytes,sha256\n");
        for index in 0..=MAX_SLOT {
            let state = match session.read_slot(index)? {
                Some(patch) => SlotState::Patch(patch),
                None => SlotState::Empty,
            };
            let label = slot_label(index);
            let frame = match &state {
                SlotState::Patch(patch) => patch.record_frame(),
                _ => patch::empty_frame(index),
            };
            let path = dir.join(format!("{label}.syx"));
            std::fs::write(&path, &frame).map_err(|err| format!("{}: {err}", path.display()))?;
            let (name, bytes, digest) = match &state {
                SlotState::Patch(patch) => {
                    (patch.name(), patch.bytes.len(), sha256_hex(&patch.bytes))
                }
                _ => (String::new(), 0, sha256_hex(&[])),
            };
            manifest.push_str(&format!(
                "{label},{index},{},{bytes},{digest}\n",
                csv_field(&name)
            ));
            self.set(index, state.clone());
            progress(index, &state);
            std::thread::sleep(crate::transport::PACE);
        }
        let path = dir.join("manifest.csv");
        std::fs::write(&path, manifest).map_err(|err| format!("{}: {err}", path.display()))?;
        Ok(path)
    }

    /// Restore a directory of `*.syx` files into their slot labels, or into the
    /// slots recorded in `manifest.csv`.
    pub fn restore(
        &mut self,
        session: &mut Session,
        dir: &Path,
        force: bool,
    ) -> Result<usize, String> {
        let mut entries: Vec<(u16, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(dir).map_err(|err| format!("{}: {err}", dir.display()))? {
            let entry = entry.map_err(|err| err.to_string())?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("syx") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| format!("{}: unreadable file name", path.display()))?;
            let index = patch::parse_slot(stem)?;
            entries.push((index, path));
        }
        entries.sort_by_key(|(index, _)| *index);
        if entries.is_empty() {
            return Err(format!("{}: no .syx files", dir.display()));
        }
        let mut written = 0;
        for (index, path) in entries {
            let mut patches = patch::load_syx(&path)?;
            let Some(mut patch) = patches.pop() else {
                continue;
            };
            patch.index = index;
            if !force
                && matches!(session.read_slot(index), Ok(Some(_)))
                && self.get(index).name().as_deref() != Some(patch.name().as_str())
            {
                return Err(format!(
                    "slot {} is not empty; pass --force to overwrite",
                    slot_label(index)
                ));
            }
            let read_back = session.write_slot(&patch)?;
            self.put(read_back);
            written += 1;
            std::thread::sleep(crate::transport::PACE);
        }
        Ok(written)
    }
}

/// One changed byte between two patches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteDiff {
    pub offset: usize,
    pub left: Option<u8>,
    pub right: Option<u8>,
}

/// Compare two payloads, including fields one of them trimmed away.
pub fn diff(left: &Patch, right: &Patch) -> Vec<ByteDiff> {
    let len = left.bytes.len().max(right.bytes.len());
    (0..len)
        .filter_map(|offset| {
            let a = left.bytes.get(offset).copied();
            let b = right.bytes.get(offset).copied();
            if a.unwrap_or(0) == b.unwrap_or(0) && a.is_some() == b.is_some() {
                None
            } else {
                Some(ByteDiff {
                    offset,
                    left: a,
                    right: b,
                })
            }
        })
        .collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Manifest for the states the librarian already knows. The GUI backup writes
/// this after a scan; [`Librarian::backup`] builds the same rows as it reads.
pub fn manifest_csv(lib: &Librarian) -> String {
    let mut manifest = String::from("slot,index,name,bytes,sha256\n");
    for (index, state) in lib.slots().iter().enumerate() {
        let index = index as u16;
        let label = slot_label(index);
        match state {
            SlotState::Unknown => {}
            SlotState::Empty => {
                manifest.push_str(&format!("{label},{index},,0,{}\n", sha256_hex(&[])))
            }
            SlotState::Patch(patch) => manifest.push_str(&format!(
                "{label},{index},{},{},{}\n",
                csv_field(&patch.name()),
                patch.bytes.len(),
                sha256_hex(&patch.bytes)
            )),
        }
    }
    manifest
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Patch {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../verified")
            .join(name);
        patch::load_syx(&path)
            .unwrap_or_else(|err| panic!("{err}"))
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("{name}: empty"))
    }

    #[test]
    fn diff_finds_changed_and_trimmed_bytes() {
        let left = Patch::new(0, vec![1, 2, 3, 0, 0]);
        let right = Patch::new(0, vec![1, 9, 3]);
        let changes = diff(&left, &right);
        assert_eq!(
            changes,
            vec![
                ByteDiff {
                    offset: 1,
                    left: Some(2),
                    right: Some(9)
                },
                ByteDiff {
                    offset: 3,
                    left: Some(0),
                    right: None
                },
                ByteDiff {
                    offset: 4,
                    left: Some(0),
                    right: None
                },
            ]
        );
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn csv_fields_are_quoted_when_needed() {
        assert_eq!(csv_field("Bass"), "Bass");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn librarian_keeps_one_state_per_address() {
        let mut lib = Librarian::new();
        assert_eq!(lib.slots().len(), 400);
        assert_eq!(lib.filled(), 0);
        lib.put(fixture("pro-800-a000.syx"));
        assert_eq!(lib.filled(), 1);
        assert_eq!(lib.get(0).name().as_deref(), Some("Classical Brass"));
        assert!(matches!(lib.get(1), SlotState::Unknown));
        lib.set(1, SlotState::Empty);
        assert!(matches!(lib.get(1), SlotState::Empty));
    }
}
