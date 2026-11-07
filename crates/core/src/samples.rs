//! Sample surface: import/export of WAV files into the instrument's sample
//! slots, and the grid that shows them.
//!
//! The grid is read-first, like the original app: slot contents are unknown
//! until the device answers a slot-list read, then each slot shows either
//! *empty* or *filled*. Imported WAV audio lives on top of that status.
//!
//! What is implemented, honestly:
//! - Import decodes a WAV file (any encoding `hound` reads), converts it to
//!   mono 16-bit and stores it in the selected slot.
//! - Export writes a slot's audio back to a 16-bit mono WAV file.
//! - The sample-directory read (`77 30` -> `78 30`) is implemented. The
//!   directory is a variable-length list of named samples, so the placeholder
//!   row count is replaced by the device's answer.
//! - Transfer to and from the instrument is **not** implemented here; the
//!   separate `opentribe-lm-drum-sample` binary performs uploads.

use std::path::Path;

use eframe::egui;
use egui::RichText;

use crate::descriptor::{SampleEntry, SampleSpec};
use crate::log::Log;
use crate::widgets::{MUTED, OK};

/// What the device reports for a slot (or what we know locally).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotStatus {
    /// The slot list has not been read from the device yet.
    Unknown,
    /// The device reports this slot as empty.
    Empty,
    /// The device reports this slot as holding a sample.
    Filled,
}

#[derive(Clone)]
pub struct SampleSlot {
    /// Device sample name, or the imported file name when audio is local.
    pub name: String,
    /// Device name, kept when a local import replaces `name`.
    pub device_name: Option<String>,
    /// The device's answer to the directory read, or [`SlotStatus::Unknown`].
    pub on_device: SlotStatus,
    /// Length reported by the device, in 16-bit samples.
    pub length_samples: Option<u32>,
    pub sample_rate: u32,
    /// Mono 16-bit audio; `None` unless imported locally this session.
    pub audio: Option<Vec<i16>>,
}

impl SampleSlot {
    fn blank(index: usize) -> Self {
        Self {
            name: format!("Slot {:02}", index + 1),
            device_name: None,
            on_device: SlotStatus::Unknown,
            length_samples: None,
            sample_rate: 0,
            audio: None,
        }
    }

    /// Shown status: local audio wins, otherwise the device's answer.
    pub fn status(&self) -> SlotStatus {
        if self.audio.is_some() {
            SlotStatus::Filled
        } else {
            self.on_device
        }
    }

    pub fn duration(&self) -> Option<f32> {
        self.audio
            .as_ref()
            .map(|audio| audio.len() as f32 / self.sample_rate.max(1) as f32)
    }

    fn summary(&self) -> String {
        match (&self.audio, self.duration()) {
            (Some(audio), Some(seconds)) => format!(
                "{:.1}s · {} Hz · {} samples",
                seconds,
                self.sample_rate,
                audio.len()
            ),
            (None, _) if self.on_device == SlotStatus::Filled => match self.length_samples {
                Some(length) => format!("{length} samples · audio not readable yet"),
                None => "filled on device · audio not readable yet".to_string(),
            },
            _ => "-".to_string(),
        }
    }
}

/// The sample slots of one device, plus the currently selected slot.
pub struct SampleBank {
    pub slots: Vec<SampleSlot>,
    pub selected: usize,
    /// Set by the UI; the app shell performs the read attempt and clears it.
    pub read_pending: bool,
}

impl SampleBank {
    pub fn new(slot_count: usize) -> Self {
        Self {
            slots: (0..slot_count).map(SampleSlot::blank).collect(),
            selected: 0,
            read_pending: false,
        }
    }

    /// Replace the placeholder rows with the device's sample directory.
    /// Locally imported audio is kept by row index, so a re-read does not
    /// discard edits made in this session.
    pub fn mark_read(&mut self, entries: &[SampleEntry]) {
        let mut old = std::mem::take(&mut self.slots);
        self.slots = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let (sample_rate, audio) = match old.get_mut(index) {
                    Some(slot) => (slot.sample_rate, slot.audio.take()),
                    None => (0, None),
                };
                SampleSlot {
                    name: entry.name.clone(),
                    device_name: Some(entry.name.clone()),
                    on_device: SlotStatus::Filled,
                    length_samples: Some(entry.length_samples),
                    sample_rate,
                    audio,
                }
            })
            .collect();
        if self.selected >= self.slots.len() {
            self.selected = 0;
        }
    }

    /// Decode a WAV file into the selected slot.
    pub fn import_wav(&mut self, path: &Path, log: &mut Log) {
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        match decode_wav(path) {
            Ok((rate, mono)) => {
                let seconds = mono.len() as f32 / rate.max(1) as f32;
                let index = self.selected;
                if self.slots[index].status() == SlotStatus::Filled {
                    log.info(format!(
                        "note: slot {} is not empty — importing overwrites it",
                        index + 1
                    ));
                }
                let previous = &self.slots[index];
                self.slots[index] = SampleSlot {
                    name: file_name.clone(),
                    device_name: previous.device_name.clone(),
                    on_device: previous.on_device,
                    length_samples: previous.length_samples,
                    sample_rate: rate,
                    audio: Some(mono),
                };
                log.info(format!(
                    "imported {file_name} into slot {} · {seconds:.1}s · {rate} Hz · mono",
                    index + 1
                ));
            }
            Err(err) => log.error(format!("import {file_name}: {err}")),
        }
    }

    /// Drop locally imported audio; slots return to their device-read status.
    pub fn clear_imported(&mut self, log: &mut Log) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.audio.is_some() {
                slot.audio = None;
                slot.sample_rate = 0;
                slot.name = slot
                    .device_name
                    .clone()
                    .unwrap_or_else(|| format!("Slot {:02}", index + 1));
            }
        }
        log.info("imported samples cleared");
    }

    /// Write the selected slot's audio to a WAV file.
    pub fn export_wav(&self, slot_index: usize, path: &Path, log: &mut Log) {
        let Some(slot) = self.slots.get(slot_index) else {
            log.error(format!("slot {} does not exist", slot_index + 1));
            return;
        };
        let Some(audio) = &slot.audio else {
            log.error(format!(
                "slot {} has no local audio: samples on the device cannot be read back yet",
                slot_index + 1
            ));
            return;
        };
        match encode_wav(path, slot.sample_rate, audio) {
            Ok(()) => log.info(format!(
                "exported slot {} to {}",
                slot_index + 1,
                path.display()
            )),
            Err(err) => log.error(format!("export {}: {err}", path.display())),
        }
    }
}

// -------------------------------------------------------------------- wav io

/// Decode any `hound`-readable WAV into mono 16-bit samples.
fn decode_wav(path: &Path) -> Result<(u32, Vec<i16>), String> {
    let mut reader = hound::WavReader::open(path).map_err(|err| err.to_string())?;
    let spec = reader.spec();
    let rate = spec.sample_rate;
    let channels = usize::from(spec.channels).max(1);
    let frames = reader.len() as usize / channels;
    let mut mono = Vec::with_capacity(frames);

    match spec.sample_format {
        hound::SampleFormat::Float => {
            let mut sums: Vec<f32> = vec![0.0; channels];
            for (index, sample) in reader.samples::<f32>().enumerate() {
                let sample = sample.map_err(|err| err.to_string())?;
                sums[index % channels] += sample;
                if index % channels == channels - 1 {
                    let mean = sums.iter().sum::<f32>() / channels as f32;
                    mono.push((mean.clamp(-1.0, 1.0) * 32767.0) as i16);
                    sums.fill(0.0);
                }
            }
        }
        hound::SampleFormat::Int => match spec.bits_per_sample {
            8 => {
                let mut sums: Vec<i32> = vec![0; channels];
                for (index, sample) in reader.samples::<i8>().enumerate() {
                    let sample = sample.map_err(|err| err.to_string())?;
                    sums[index % channels] += i32::from(sample) << 8;
                    if index % channels == channels - 1 {
                        mono.push((sums.iter().sum::<i32>() / channels as i32) as i16);
                        sums.fill(0);
                    }
                }
            }
            16 => {
                let mut sums: Vec<i32> = vec![0; channels];
                for (index, sample) in reader.samples::<i16>().enumerate() {
                    let sample = sample.map_err(|err| err.to_string())?;
                    sums[index % channels] += i32::from(sample);
                    if index % channels == channels - 1 {
                        mono.push((sums.iter().sum::<i32>() / channels as i32) as i16);
                        sums.fill(0);
                    }
                }
            }
            24 | 32 => {
                let shift = if spec.bits_per_sample == 24 { 8 } else { 16 };
                let mut sums: Vec<i64> = vec![0; channels];
                for (index, sample) in reader.samples::<i32>().enumerate() {
                    let sample = sample.map_err(|err| err.to_string())?;
                    sums[index % channels] += i64::from(sample >> shift);
                    if index % channels == channels - 1 {
                        mono.push((sums.iter().sum::<i64>() / channels as i64) as i16);
                        sums.fill(0);
                    }
                }
            }
            bits => return Err(format!("unsupported WAV bit depth {bits}")),
        },
    }

    if mono.is_empty() {
        return Err("no audio in file".to_string());
    }
    Ok((rate, mono))
}

/// Encode mono 16-bit audio as a WAV file.
fn encode_wav(path: &Path, rate: u32, audio: &[i16]) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|err| err.to_string())?;
    for sample in audio {
        writer
            .write_sample(*sample)
            .map_err(|err| err.to_string())?;
    }
    writer.finalize().map_err(|err| err.to_string())?;
    Ok(())
}

// ----------------------------------------------------------------------- ui

/// The sample grid plus import/export controls.
pub fn samples_tab(
    ui: &mut egui::Ui,
    spec: &'static SampleSpec,
    bank: &mut SampleBank,
    log: &mut Log,
) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            crate::widgets::section(ui, "Sample import / export");
            ui.horizontal(|ui| {
                if ui
                    .button("Read slot contents")
                    .on_hover_text(
                        "Reads the device's sample directory (names and lengths) with `77 30`.",
                    )
                    .clicked()
                {
                    bank.read_pending = true;
                }
                ui.separator();
                if ui
                    .button("Import WAV into selected slot…")
                    .on_hover_text("Decodes a WAV file into the selected slot (mono 16-bit)")
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("WAV audio", &["wav"])
                        .pick_file()
                {
                    bank.import_wav(&path, log);
                }
                if ui
                    .add_enabled(
                        bank.slots
                            .get(bank.selected)
                            .is_some_and(|slot| slot.audio.is_some()),
                        egui::Button::new("Export selected slot…"),
                    )
                    .on_hover_text("Writes the selected slot's audio to a WAV file")
                    .clicked()
                {
                    let index = bank.selected;
                    if let Some(slot) = bank.slots.get(index)
                        && let Some(path) = rfd::FileDialog::new()
                            .set_file_name(&slot.name)
                            .add_filter("WAV audio", &["wav"])
                            .save_file()
                    {
                        bank.export_wav(index, &path, log);
                    }
                }
                ui.separator();
                ui.weak(format!("{} samples", bank.slots.len()));
                ui.separator();
                if ui.button("Clear imported").clicked() {
                    bank.clear_imported(log);
                }
            });

            ui.add_space(6.0);
            egui::Grid::new("sample_slots")
                .num_columns(4)
                .spacing([12.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    for header in ["slot", "name", "status", "length"] {
                        ui.label(RichText::new(header).strong().size(12.0));
                    }
                    ui.end_row();
                    for (index, slot) in bank.slots.iter().enumerate() {
                        let selected = index == bank.selected;
                        if ui
                            .selectable_label(selected, format!("{:02}", index + 1))
                            .clicked()
                        {
                            bank.selected = index;
                        }
                        let name = if selected {
                            RichText::new(&slot.name).color(OK)
                        } else {
                            RichText::new(&slot.name)
                        };
                        ui.label(name);
                        match slot.status() {
                            SlotStatus::Unknown => {
                                ui.label(RichText::new("not read").color(MUTED).size(12.0));
                            }
                            SlotStatus::Empty => {
                                ui.label(RichText::new("empty").color(OK).size(12.0));
                            }
                            SlotStatus::Filled => {
                                let text = if slot.audio.is_some() {
                                    "imported"
                                } else {
                                    "filled"
                                };
                                ui.label(RichText::new(text).color(OK).size(12.0));
                            }
                        }
                        ui.label(RichText::new(slot.summary()).size(12.0));
                        ui.end_row();
                    }
                });

            ui.add_space(8.0);
            for line in spec.notes {
                ui.weak(format!("• {line}"));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::Log;
    use std::path::PathBuf;

    /// One temp path per call so parallel tests never share a file.
    fn temp_wav() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("opentribe-test-{}-{n}.wav", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn wav_roundtrip_preserves_mono_samples() {
        let path = temp_wav();
        let audio: Vec<i16> = (0..48000i64)
            .map(|i| ((i % 1000) - 500) * 10)
            .map(|v| v as i16)
            .collect();
        encode_wav(&path, 48000, &audio).unwrap_or_else(|_| panic!("encodes"));
        let (rate, decoded) = decode_wav(&path).unwrap_or_else(|_| panic!("decodes"));
        assert_eq!(rate, 48000);
        assert_eq!(decoded, audio);
        let _ = std::fs::remove_file(&path);
    }

    fn directory(names: &[(&str, u32)]) -> Vec<SampleEntry> {
        names
            .iter()
            .map(|(name, length_samples)| SampleEntry {
                name: (*name).to_string(),
                length_samples: *length_samples,
            })
            .collect()
    }

    #[test]
    fn slots_start_unread_until_a_read_answers() {
        let bank = SampleBank::new(4);
        assert!(
            bank.slots
                .iter()
                .all(|slot| slot.status() == SlotStatus::Unknown)
        );

        let mut bank = bank;
        bank.mark_read(&directory(&[("LM1-BASS", 4458), ("LM1-SNARE", 4458)]));
        assert_eq!(
            bank.slots.len(),
            2,
            "the directory replaces the placeholder"
        );
        assert_eq!(bank.slots[0].name, "LM1-BASS");
        assert_eq!(bank.slots[0].length_samples, Some(4458));
        assert_eq!(bank.slots[0].status(), SlotStatus::Filled);
        assert_eq!(bank.slots[1].status(), SlotStatus::Filled);
    }

    #[test]
    fn import_fills_the_selected_slot_and_clear_restores_the_read_status() {
        let path = temp_wav();
        encode_wav(&path, 44100, &[100, -100, 50, -50]).unwrap_or_else(|_| panic!("encodes"));
        let mut bank = SampleBank::new(2);
        bank.mark_read(&directory(&[("LM1-BASS", 4458), ("LM1-SNARE", 4458)]));
        bank.selected = 1;
        let mut log = Log::default();
        bank.import_wav(&path, &mut log);
        let slot = &bank.slots[1];
        assert_eq!(slot.status(), SlotStatus::Filled);
        assert_eq!(slot.sample_rate, 44100);
        assert_eq!(slot.audio.as_deref(), Some(&[100, -100, 50, -50][..]));

        // Clearing drops the local audio and restores the device's name.
        bank.clear_imported(&mut log);
        assert_eq!(bank.slots[1].status(), SlotStatus::Filled);
        assert_eq!(bank.slots[1].name, "LM1-SNARE");
        assert!(bank.slots[1].audio.is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn export_refuses_slots_without_local_audio() {
        let mut bank = SampleBank::new(2);
        bank.mark_read(&directory(&[("LM1-BASS", 4458)]));
        let path = temp_wav();
        let mut log = Log::default();
        bank.export_wav(0, &path, &mut log);
        assert!(!path.exists());
    }
}
