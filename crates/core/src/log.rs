//! Rolling message log shown at the bottom of the window.

use std::collections::VecDeque;
use std::time::Instant;

use eframe::egui::Color32;

use crate::sysex;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Tx,
    Rx,
    Info,
    Error,
}

impl Dir {
    pub fn tag(self) -> &'static str {
        match self {
            Dir::Tx => "TX",
            Dir::Rx => "RX",
            Dir::Info => "info",
            Dir::Error => "err",
        }
    }

    pub fn color(self) -> Color32 {
        match self {
            Dir::Tx => Color32::from_rgb(0x79, 0xB8, 0xFF),
            Dir::Rx => Color32::from_rgb(0x7F, 0xD7, 0x8A),
            Dir::Info => Color32::from_gray(0x9A),
            Dir::Error => Color32::from_rgb(0xE5, 0x7A, 0x7A),
        }
    }
}

struct Entry {
    dir: Dir,
    label: String,
    hex: String,
    at: f64,
}

pub struct Log {
    entries: VecDeque<Entry>,
    start: Instant,
    pub show_rx: bool,
    pub show_info: bool,
}

impl Default for Log {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            start: Instant::now(),
            show_rx: true,
            show_info: true,
        }
    }
}

impl Log {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn push(&mut self, dir: Dir, label: impl Into<String>, bytes: &[u8]) {
        if self.entries.len() >= 600 {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry {
            dir,
            label: label.into(),
            hex: sysex::hex(bytes),
            at: self.start.elapsed().as_secs_f64(),
        });
    }

    pub fn info(&mut self, label: impl Into<String>) {
        self.push(Dir::Info, label, &[]);
    }

    pub fn error(&mut self, label: impl Into<String>) {
        self.push(Dir::Error, label, &[]);
    }

    /// Formatted lines honouring the RX/info filters.
    pub fn visible(&self) -> Vec<(Dir, String)> {
        self.entries
            .iter()
            .filter(|entry| match entry.dir {
                Dir::Rx => self.show_rx,
                Dir::Info => self.show_info,
                _ => true,
            })
            .map(|entry| {
                let minutes = (entry.at / 60.0) as u64;
                let seconds = entry.at % 60.0;
                (
                    entry.dir,
                    format!(
                        "{:02}:{:05.2}  {:<4}  {:<32}  {}",
                        minutes,
                        seconds,
                        entry.dir.tag(),
                        entry.label,
                        entry.hex
                    ),
                )
            })
            .collect()
    }
}
