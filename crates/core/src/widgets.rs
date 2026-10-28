//! Small egui helpers shared by every device panel.

use eframe::egui::{self, Color32, RichText};

pub const HEADING: Color32 = Color32::from_rgb(0x8A, 0xB4, 0xF8);
pub const OK: Color32 = Color32::from_rgb(0x6F, 0xD0, 0x89);
pub const WARN: Color32 = Color32::from_rgb(0xE8, 0xB3, 0x4A);
pub const DANGER: Color32 = Color32::from_rgb(0xE5, 0x7A, 0x7A);
pub const MUTED: Color32 = Color32::from_gray(0x8C);

pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).strong().color(HEADING));
    ui.add_space(2.0);
}

/// Third grid column: hint text, or an empty cell to keep the grid aligned.
pub fn hint_cell(ui: &mut egui::Ui, hint: &str) {
    if hint.is_empty() {
        ui.label("");
    } else {
        ui.weak(hint);
    }
}

/// Small filled circle; the default egui font has no `●` glyph.
pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// Shorten an ALSA/CoreMIDI port name for status labels, e.g.
/// `POLY D:POLY D MIDI 1 16:0` becomes `POLY D MIDI 1`.
pub fn short_port(name: &str) -> String {
    let after_client = name.split_once(':').map(|(_, rest)| rest).unwrap_or(name);
    let mut parts: Vec<&str> = after_client.split_whitespace().collect();
    let trailing_address = parts.last().is_some_and(|last| {
        last.contains(':') && last.chars().all(|c| c.is_ascii_digit() || c == ':')
    });
    if parts.len() > 1 && trailing_address {
        parts.pop();
    }
    parts.join(" ")
}

pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = i16::from(note) / 12 - 1;
    format!("{}{}", NAMES[(note % 12) as usize], octave)
}

/// Draw a two-octave keyboard. Returns the note under the pointer while the
/// left button is held inside the widget.
pub fn keyboard(ui: &mut egui::Ui, held: Option<u8>, base: u8, octaves: usize) -> Option<u8> {
    const WHITE_SEMITONES: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
    const BLACK_SEMITONES: [(usize, u8); 5] = [(0, 1), (1, 3), (3, 6), (4, 8), (5, 10)];

    let white_count = 7 * octaves;
    let width = white_count as f32 * 27.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, 96.0), egui::Sense::click_and_drag());
    let painter = ui.painter();
    let key_width = rect.width() / white_count as f32;
    let pointer = if response.is_pointer_button_down_on() {
        response.interact_pointer_pos()
    } else {
        None
    };
    let mut hit = None;

    for index in 0..white_count {
        let note = base + (index / 7) as u8 * 12 + WHITE_SEMITONES[index % 7];
        let key_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + index as f32 * key_width, rect.top()),
            egui::vec2(key_width - 1.0, rect.height()),
        );
        let fill = if held == Some(note) {
            Color32::from_rgb(0x5F, 0x8F, 0xD6)
        } else {
            Color32::from_gray(0xE6)
        };
        painter.rect_filled(key_rect, 3.0, fill);
        painter.rect_stroke(
            key_rect,
            3.0,
            egui::Stroke::new(1.0, Color32::from_gray(0x55)),
            egui::StrokeKind::Inside,
        );
        if index % 7 == 0 {
            painter.text(
                egui::pos2(key_rect.center().x, key_rect.bottom() - 6.0),
                egui::Align2::CENTER_BOTTOM,
                note_name(note),
                egui::FontId::monospace(10.0),
                Color32::from_gray(0x77),
            );
        }
        if pointer.is_some_and(|pos| key_rect.contains(pos)) {
            hit = Some(note);
        }
    }

    for octave in 0..octaves {
        for (white_index, semitone) in BLACK_SEMITONES {
            let note = base + octave as u8 * 12 + semitone;
            let x = rect.left() + (octave * 7 + white_index + 1) as f32 * key_width;
            let key_rect = egui::Rect::from_min_size(
                egui::pos2(x - key_width * 0.30, rect.top()),
                egui::vec2(key_width * 0.60, rect.height() * 0.62),
            );
            let fill = if held == Some(note) {
                Color32::from_rgb(0x3E, 0x6A, 0xA8)
            } else {
                Color32::from_gray(0x1D)
            };
            painter.rect_filled(key_rect, 2.0, fill);
            painter.rect_stroke(
                key_rect,
                2.0,
                egui::Stroke::new(1.0, Color32::from_gray(0x0E)),
                egui::StrokeKind::Inside,
            );
            if pointer.is_some_and(|pos| key_rect.contains(pos)) {
                hit = Some(note);
            }
        }
    }

    hit
}
