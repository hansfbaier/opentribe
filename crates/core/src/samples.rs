//! Sample surface: import/export of WAV files into the instrument's sample
//! slots, and the grid that shows them.
//!
//! What is implemented, honestly:
//! - Import decodes a WAV file (any encoding `hound` reads), converts it to
//!   mono 16-bit and stores it in the selected slot.
//! - Export writes a slot's audio back to a 16-bit mono WAV file.
//! - Transfer to and from the instrument is **not** implemented: the transfer
//!   frames have not been captured. The tab says so instead of pretending.
//!   Recording the vendor app's USB traffic while it imports or exports a
//!   sample settles the protocol.

use std::path::Path;

use eframe::egui;
use egui::RichText;

use crate::descriptor::SampleSpec;
use crate::log::Log;
use crate::widgets::{MUTED, OK, WARN};

/// Where a slot's audio came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// Shipped with the instrument; we cannot read it back yet.
    Factory,
    /// Imported from a WAV file this session.
    Imported,
}

pub struct SampleSlot {
    pub name: String,
    pub source: Source,
    pub sample_rate: u32,
    /// Mono 16-bit audio; `None` for factory slots (no transfer protocol yet).
    pub audio: Option<Vec<i16>>,
}

impl SampleSlot {
    fn factory(index: usize) -> Self {
        Self {
            name: format!("Slot {:02}", index + 1),
            source: Source::Factory,
            sample_rate: 0,
            audio: None,
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
            _ => "factory sample · not readable yet".to_string(),
        }
    }
}

/// The sample slots of one device, plus the currently selected slot.
pub struct SampleBank {
    pub slots: Vec<SampleSlot>,
    pub selected: usize,
}

impl SampleBank {
    pub fn new(slot_count: usize) -> Self {
        Self {
            slots: (0..slot_count).map(SampleSlot::factory).collect(),
            selected: 0,
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
                self.slots[index] = SampleSlot {
                    name: file_name.clone(),
                    source: Source::Imported,
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

    /// Write the selected slot's audio to a WAV file.
    pub fn export_wav(&self, slot_index: usize, path: &Path, log: &mut Log) {
        let Some(slot) = self.slots.get(slot_index) else {
            log.error(format!("slot {} does not exist", slot_index + 1));
            return;
        };
        let Some(audio) = &slot.audio else {
            log.error(format!(
                "slot {} has no audio: factory samples cannot be read back yet",
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
                ui.weak(format!("{} slots", spec.slots));
                ui.separator();
                if ui.button("Clear imported").clicked() {
                    for (index, slot) in bank.slots.iter_mut().enumerate() {
                        if slot.source == Source::Imported {
                            *slot = SampleSlot::factory(index);
                        }
                    }
                    log.info("imported samples cleared");
                }
            });

            ui.add_space(6.0);
            egui::Grid::new("sample_slots")
                .num_columns(5)
                .spacing([12.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    for header in ["slot", "name", "source", "audio", "transfer"] {
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
                        match slot.source {
                            Source::Factory => {
                                ui.label(RichText::new("factory").color(MUTED).size(12.0));
                            }
                            Source::Imported => {
                                ui.label(RichText::new("imported").color(OK).size(12.0));
                            }
                        }
                        ui.label(RichText::new(slot.summary()).size(12.0));
                        ui.label(RichText::new("not measured yet").color(WARN).size(12.0));
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

    #[test]
    fn import_fills_the_selected_slot() {
        let path = temp_wav();
        encode_wav(&path, 44100, &[100, -100, 50, -50]).unwrap_or_else(|_| panic!("encodes"));
        let mut bank = SampleBank::new(4);
        bank.selected = 2;
        let mut log = Log::default();
        bank.import_wav(&path, &mut log);
        let slot = &bank.slots[2];
        assert_eq!(slot.source, Source::Imported);
        assert_eq!(slot.sample_rate, 44100);
        assert_eq!(slot.audio.as_deref(), Some(&[100, -100, 50, -50][..]));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn export_refuses_factory_slots() {
        let bank = SampleBank::new(2);
        let path = temp_wav();
        let mut log = Log::default();
        bank.export_wav(0, &path, &mut log);
        assert!(!path.exists());
    }
}
