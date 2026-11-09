//! The PRO-800 patch control surface, transcribed from the Swumpf editor
//! (`vendor/swumpf/swumpf_114.html`, v114) and its CC map.
//!
//! Every entry keeps the unpacked payload offset the editor uses, the live MIDI
//! CC number it sends, and the value kind. Offsets are *not* firmware-verified:
//! they are tagged [`Evidence::VendorNamed`] and the two facts that were measured
//! on hardware (the name field and the pitch-bend range) are tagged
//! [`Evidence::Verified`]. See `docs/spec-pro-800.md` gate G2.

use tribe_core::descriptor::Evidence;

use crate::patch::{NAME_LEN, NAME_OFFSET, Patch};

/// One selectable value of an enum parameter.
#[derive(Clone, Copy, Debug)]
pub struct Choice {
    /// Byte stored in the patch payload.
    pub wire: u8,
    pub label: &'static str,
    /// CC value the Swumpf editor sends for a live preview.
    pub cc: u8,
}

/// How a parameter is encoded in the patch payload.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// 16-bit little-endian, shown on a 0–999 knob (the editor's scale).
    Knob,
    /// One plain byte, shown numerically.
    Byte,
    /// One byte, 0 or 1.
    Bool,
    /// One bit of a shared byte; `Param::mask` selects it.
    Bit,
    /// One byte chosen from a fixed list.
    Enum(&'static [Choice]),
}

/// One patch control.
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub id: &'static str,
    pub title: &'static str,
    pub section: &'static str,
    /// Unpacked payload offset.
    pub offset: usize,
    pub kind: Kind,
    /// Bits this control owns when writing (`0` means the whole byte).
    pub mask: u8,
    /// Live MIDI CC number.
    pub cc: Option<u8>,
    pub evidence: Evidence,
}

const NAMED: Evidence = Evidence::VendorNamed;
const VERIFIED: Evidence = Evidence::Verified;

// ---------------------------------------------------------------- label sets

const POT_MODE: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Free",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "Semi",
        cc: 32,
    },
    Choice {
        wire: 2,
        label: "Octave",
        cc: 64,
    },
    Choice {
        wire: 3,
        label: "Fixed",
        cc: 96,
    },
];
const KEY_TRACK: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Off",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "Half",
        cc: 43,
    },
    Choice {
        wire: 2,
        label: "Full",
        cc: 86,
    },
];
const REF_NOTE: &[Choice] = &[
    Choice {
        wire: 0,
        label: "C1",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "C2",
        cc: 32,
    },
    Choice {
        wire: 2,
        label: "C3",
        cc: 64,
    },
    Choice {
        wire: 3,
        label: "C4",
        cc: 96,
    },
];
const SLOW_FAST: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Fast",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "Slow",
        cc: 64,
    },
];
const FAST_SLOW: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Slow",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "Fast",
        cc: 64,
    },
];
const EXP_LIN: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Exp",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "Lin",
        cc: 64,
    },
];
const LFO_SHAPE: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Pulse",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "Triangle",
        cc: 22,
    },
    Choice {
        wire: 2,
        label: "Random",
        cc: 44,
    },
    Choice {
        wire: 3,
        label: "Sine",
        cc: 66,
    },
    Choice {
        wire: 4,
        label: "Noise",
        cc: 88,
    },
    Choice {
        wire: 5,
        label: "Saw",
        cc: 110,
    },
];
/// Main LFO destination. The low three bits of the same byte are separate
/// switches ([`Param::LFO_FREQ_BIT`] and friends), so this control owns `0x38`.
const LFO_TARGET: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Freq A/B",
        cc: 0,
    },
    Choice {
        wire: 8,
        label: "Freq A",
        cc: 33,
    },
    Choice {
        wire: 16,
        label: "Freq B",
        cc: 66,
    },
    Choice {
        wire: 32,
        label: "VCA",
        cc: 99,
    },
];
const GLIDE_MODE: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Time",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "Speed",
        cc: 64,
    },
];
const WHEEL_TARGET: &[Choice] = &[
    Choice {
        wire: 0,
        label: "LFO",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "Vibrato",
        cc: 64,
    },
];
const WHEEL_AMOUNT: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Min",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "Low",
        cc: 32,
    },
    Choice {
        wire: 2,
        label: "High",
        cc: 64,
    },
    Choice {
        wire: 3,
        label: "Full",
        cc: 96,
    },
];
const PITCH_TARGET: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Off",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "VCO",
        cc: 43,
    },
    Choice {
        wire: 2,
        label: "VCF",
        cc: 86,
    },
];
const ARP_MODE: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Off",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "Up",
        cc: 19,
    },
    Choice {
        wire: 2,
        label: "Down",
        cc: 37,
    },
    Choice {
        wire: 3,
        label: "Up/Down",
        cc: 55,
    },
    Choice {
        wire: 4,
        label: "Up+Down",
        cc: 74,
    },
    Choice {
        wire: 5,
        label: "Random",
        cc: 92,
    },
    Choice {
        wire: 6,
        label: "Assigned",
        cc: 110,
    },
];

// ------------------------------------------------------------------ sections

pub const OSC_A: &str = "Oscillator A";
pub const NOISE: &str = "Noise";
pub const OSC_B: &str = "Oscillator B";
pub const FILTER: &str = "Filter";
pub const FILTER_ENV: &str = "Filter envelope";
pub const POLY_MOD: &str = "Poly-mod";
pub const AMP_ENV: &str = "Amplifier envelope";
pub const AFTERTOUCH: &str = "Aftertouch";
pub const LFO: &str = "LFO-mod";
pub const UNISON: &str = "Unison/detune";
pub const GLIDE: &str = "Glide";
pub const MOD_WHEEL: &str = "Mod wheel/vibrato";
pub const PITCH_WHEEL: &str = "Pitch wheel";
pub const ARP: &str = "Arpeggiator";

/// Section order in the Patch tab.
pub const SECTIONS: [&str; 14] = [
    OSC_A,
    NOISE,
    OSC_B,
    FILTER,
    FILTER_ENV,
    POLY_MOD,
    AMP_ENV,
    AFTERTOUCH,
    LFO,
    UNISON,
    GLIDE,
    MOD_WHEEL,
    PITCH_WHEEL,
    ARP,
];

// ------------------------------------------------------------ patch controls

macro_rules! knob {
    ($id:literal, $title:literal, $section:expr, $offset:expr, $cc:expr) => {
        Param {
            id: $id,
            title: $title,
            section: $section,
            offset: $offset,
            kind: Kind::Knob,
            mask: 0,
            cc: Some($cc),
            evidence: NAMED,
        }
    };
    ($id:literal, $title:literal, $section:expr, $offset:expr, $cc:expr, $evidence:expr) => {
        Param {
            id: $id,
            title: $title,
            section: $section,
            offset: $offset,
            kind: Kind::Knob,
            mask: 0,
            cc: Some($cc),
            evidence: $evidence,
        }
    };
}

macro_rules! toggle {
    ($id:literal, $title:literal, $section:expr, $offset:expr, $cc:expr) => {
        Param {
            id: $id,
            title: $title,
            section: $section,
            offset: $offset,
            kind: Kind::Bool,
            mask: 0,
            cc: Some($cc),
            evidence: NAMED,
        }
    };
    ($id:literal, $title:literal, $section:expr, $offset:expr, $cc:expr, $mask:expr) => {
        Param {
            id: $id,
            title: $title,
            section: $section,
            offset: $offset,
            kind: Kind::Bit,
            mask: $mask,
            cc: Some($cc),
            evidence: NAMED,
        }
    };
}

macro_rules! select {
    ($id:literal, $title:literal, $section:expr, $offset:expr, $cc:expr, $choices:expr) => {
        Param {
            id: $id,
            title: $title,
            section: $section,
            offset: $offset,
            kind: Kind::Enum($choices),
            mask: 0,
            cc: Some($cc),
            evidence: NAMED,
        }
    };
    ($id:literal, $title:literal, $section:expr, $offset:expr, $cc:expr, $choices:expr, $mask:expr) => {
        Param {
            id: $id,
            title: $title,
            section: $section,
            offset: $offset,
            kind: Kind::Enum($choices),
            mask: $mask,
            cc: Some($cc),
            evidence: NAMED,
        }
    };
}

/// Every patch control, in the Swumpf editor's order.
pub const PARAMS: &[Param] = &[
    // OSC A
    knob!("osc_a.freq", "Frequency", OSC_A, 5, 8),
    knob!("osc_a.pw", "Pulse width", OSC_A, 9, 10),
    knob!("osc_a.level", "Level", OSC_A, 7, 9),
    toggle!("osc_a.saw", "Saw", OSC_A, 55, 48),
    toggle!("osc_a.tri", "Triangle", OSC_A, 56, 49),
    toggle!("osc_a.square", "Square", OSC_A, 57, 50),
    toggle!("osc_a.sync", "Sync", OSC_A, 61, 54),
    select!("osc_a.pot_mode", "Freq pot mode", OSC_A, 74, 68, POT_MODE),
    // Noise
    knob!("noise.level", "Level", NOISE, 142, 37),
    // OSC B
    knob!("osc_b.freq", "Frequency", OSC_B, 11, 11),
    knob!("osc_b.fine", "Fine", OSC_B, 17, 14),
    knob!("osc_b.pw", "Pulse width", OSC_B, 15, 13),
    knob!("osc_b.level", "Level", OSC_B, 13, 12),
    toggle!("osc_b.saw", "Saw", OSC_B, 58, 51),
    toggle!("osc_b.tri", "Triangle", OSC_B, 59, 52),
    toggle!("osc_b.square", "Square", OSC_B, 60, 53),
    select!("osc_b.pot_mode", "Freq pot mode", OSC_B, 75, 69, POT_MODE),
    // Filter
    knob!("filter.cutoff", "Cutoff", FILTER, 19, 15),
    knob!("filter.resonance", "Resonance", FILTER, 21, 16),
    knob!("filter.env_amount", "Env amount", FILTER, 23, 17),
    select!("filter.keyboard", "Keyboard", FILTER, 67, 60, KEY_TRACK),
    select!("filter.ref_note", "Ref note", FILTER, 169, 78, REF_NOTE),
    // Filter envelope
    knob!("fe.attack", "Attack", FILTER_ENV, 31, 21),
    knob!("fe.decay", "Decay", FILTER_ENV, 29, 20),
    knob!("fe.sustain", "Sustain", FILTER_ENV, 27, 19),
    knob!("fe.release", "Release", FILTER_ENV, 25, 18),
    knob!("fe.velocity", "Velocity", FILTER_ENV, 53, 32),
    select!("fe.speed", "Speed", FILTER_ENV, 69, 62, SLOW_FAST),
    select!("fe.shape", "Shape", FILTER_ENV, 68, 61, EXP_LIN),
    // Poly-mod
    knob!("pm.filter_env", "Filter env", POLY_MOD, 41, 26),
    knob!("pm.osc_b", "OSC B", POLY_MOD, 43, 27),
    toggle!("pm.freq_a", "Freq A", POLY_MOD, 62, 55),
    toggle!("pm.filter", "Filter cutoff", POLY_MOD, 63, 56),
    // Amp envelope
    knob!("ae.attack", "Attack", AMP_ENV, 39, 25),
    knob!("ae.decay", "Decay", AMP_ENV, 37, 24),
    knob!("ae.sustain", "Sustain", AMP_ENV, 35, 23),
    knob!("ae.release", "Release", AMP_ENV, 33, 22),
    knob!("ae.velocity", "Velocity", AMP_ENV, 51, 31),
    select!("ae.speed", "Speed", AMP_ENV, 148, 72, SLOW_FAST),
    select!("ae.shape", "Shape", AMP_ENV, 70, 63, EXP_LIN),
    // Aftertouch
    knob!("at.vca", "VCA amount", AFTERTOUCH, 144, 39),
    knob!("at.vcf", "VCF amount", AFTERTOUCH, 146, 40),
    knob!("at.lfo", "LFO amount", AFTERTOUCH, 166, 41),
    // LFO
    knob!("lfo.delay", "Mod delay", LFO, 76, 33),
    knob!("lfo.freq", "Frequency", LFO, 45, 28),
    knob!("lfo.amount", "Ini amount", LFO, 47, 29),
    select!("lfo.speed", "Speed range", LFO, 65, 58, FAST_SLOW),
    select!("lfo.shape", "LFO shape", LFO, 64, 57, LFO_SHAPE),
    select!("lfo.target", "LFO target", LFO, 66, 59, LFO_TARGET, 0x38),
    toggle!("lfo.to_freq", "Freq", LFO, 66, 74, 0x01),
    toggle!("lfo.to_filter", "Filter", LFO, 66, 75, 0x02),
    toggle!("lfo.to_pw", "PW", LFO, 66, 76, 0x04),
    // Unison
    toggle!("unison.spread", "Voice spread", UNISON, 168, 77),
    toggle!("unison.enabled", "Unison", UNISON, 71, 65),
    knob!("unison.detune", "Detune", UNISON, 82, 36),
    // Glide
    knob!("glide.amount", "Amount", GLIDE, 49, 30),
    select!("glide.mode", "Mode", GLIDE, 170, 79, GLIDE_MODE),
    // Mod wheel / vibrato
    select!(
        "modwheel.target",
        "Wheel target",
        MOD_WHEEL,
        84,
        70,
        WHEEL_TARGET
    ),
    knob!("vibrato.freq", "Vibrato freq", MOD_WHEEL, 78, 34),
    knob!("vibrato.amount", "Vib amount", MOD_WHEEL, 80, 35),
    select!(
        "modwheel.amount",
        "Wheel amount",
        MOD_WHEEL,
        73,
        67,
        WHEEL_AMOUNT
    ),
    // Pitch wheel
    select!(
        "pitchwheel.target",
        "Target",
        PITCH_WHEEL,
        72,
        66,
        PITCH_TARGET
    ),
    knob!(
        "pitchwheel.range",
        "Pitch range",
        PITCH_WHEEL,
        171,
        42,
        VERIFIED
    ),
    // Arpeggiator
    select!("arp.mode", "Mode", ARP, 149, 73, ARP_MODE),
];

/// Parameters of one section, in table order.
pub fn section(section: &str) -> impl Iterator<Item = &'static Param> + '_ {
    PARAMS.iter().filter(move |param| param.section == section)
}

pub fn param(id: &str) -> Option<&'static Param> {
    PARAMS.iter().find(|param| param.id == id)
}

// -------------------------------------------------------------- chord memory

/// First byte of the chord-memory interval list.
pub const CHORD_OFFSET: usize = 87;
/// Seven interval bytes, one per extra voice.
pub const CHORD_LEN: usize = 7;
/// Interval byte that marks an unused voice.
pub const CHORD_NONE: u8 = 255;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chord {
    /// All seven voices in unison (bytes `00`).
    FullStack,
    /// One voice (bytes `FF`).
    Mono,
    /// Intervals above the lowest held key; `None` for an unused voice.
    Memory(Vec<Option<u8>>),
}

/// Decode the chord-memory bytes of a patch.
pub fn chord(patch: &Patch) -> Chord {
    let field = patch_field(patch, CHORD_OFFSET, CHORD_LEN);
    if field.iter().all(|byte| *byte == CHORD_NONE) {
        return Chord::Mono;
    }
    if field.iter().all(|byte| *byte == 0) {
        return Chord::FullStack;
    }
    Chord::Memory(
        field
            .iter()
            .map(|byte| (*byte != CHORD_NONE).then_some(*byte))
            .collect(),
    )
}

/// Encode a chord-memory setting into the patch.
pub fn set_chord(patch: &mut Patch, chord: &Chord) {
    let bytes = match chord {
        Chord::FullStack => [0u8; CHORD_LEN],
        Chord::Mono => [CHORD_NONE; CHORD_LEN],
        Chord::Memory(intervals) => {
            let mut bytes = [CHORD_NONE; CHORD_LEN];
            for (slot, interval) in bytes.iter_mut().zip(intervals.iter()) {
                if let Some(interval) = interval {
                    *slot = (*interval).min(CHORD_NONE - 1);
                }
            }
            bytes
        }
    };
    patch.ensure_for(CHORD_OFFSET + CHORD_LEN);
    patch.bytes[CHORD_OFFSET..CHORD_OFFSET + CHORD_LEN].copy_from_slice(&bytes);
}

// ---------------------------------------------------------------- globals

/// One global-settings control (offsets are inside the 40-byte global block).
#[derive(Clone, Copy, Debug)]
pub struct GlobalParam {
    pub id: &'static str,
    pub title: &'static str,
    pub offset: usize,
    pub kind: Kind,
    pub evidence: Evidence,
    /// Whether the app offers a write. `false` means read-only.
    pub writable: bool,
}

const NOTE_PRIORITY: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Last",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "Low",
        cc: 0,
    },
    Choice {
        wire: 2,
        label: "High",
        cc: 0,
    },
];
const SYNC_MODE: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Internal",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "MIDI",
        cc: 0,
    },
    Choice {
        wire: 2,
        label: "USB",
        cc: 0,
    },
    Choice {
        wire: 3,
        label: "Sync In",
        cc: 0,
    },
];
const OFF_ON: &[Choice] = &[
    Choice {
        wire: 0,
        label: "Off",
        cc: 0,
    },
    Choice {
        wire: 1,
        label: "On",
        cc: 0,
    },
];

/// Global controls. Only the three the Swumpf editor writes are marked
/// writable; the rest are read-only until gate G3.
pub const GLOBALS: &[GlobalParam] = &[
    GlobalParam {
        id: "global.active_voices",
        title: "Active voices",
        offset: 9,
        kind: Kind::Byte,
        evidence: NAMED,
        writable: true,
    },
    GlobalParam {
        id: "global.note_priority",
        title: "Note priority",
        offset: 26,
        kind: Kind::Enum(NOTE_PRIORITY),
        evidence: NAMED,
        writable: true,
    },
    GlobalParam {
        id: "global.transpose",
        title: "Transpose",
        offset: 37,
        kind: Kind::Byte,
        evidence: NAMED,
        writable: true,
    },
    GlobalParam {
        id: "global.sync_mode",
        title: "Sync mode",
        offset: 11,
        kind: Kind::Enum(SYNC_MODE),
        evidence: NAMED,
        writable: false,
    },
    GlobalParam {
        id: "global.local",
        title: "Local",
        offset: 30,
        kind: Kind::Enum(OFF_ON),
        evidence: NAMED,
        writable: false,
    },
    GlobalParam {
        id: "global.soft_thru",
        title: "Soft thru",
        offset: 29,
        kind: Kind::Enum(OFF_ON),
        evidence: NAMED,
        writable: false,
    },
    GlobalParam {
        id: "global.pitchbend_semitones",
        title: "Pitch-bend semitones",
        offset: 24,
        kind: Kind::Byte,
        evidence: NAMED,
        writable: false,
    },
];

// ------------------------------------------------------------------- values

/// Decode a knob value (0–999) for a 16-bit payload field.
pub fn raw_to_knob(raw: u16) -> u32 {
    ((u32::from(raw) * 999 + 32_767) / 65_535).min(999)
}

/// Encode a knob value (0–999) into a 16-bit payload field.
pub fn knob_to_raw(knob: u32) -> u16 {
    ((knob.min(999) * 65_535 + 499) / 999) as u16
}

/// CC value for a knob position, as the Swumpf editor sends it.
pub fn knob_to_cc(knob: u32) -> u8 {
    ((knob.min(999) * 127 + 499) / 999) as u8
}

fn patch_field(patch: &Patch, offset: usize, len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| patch.byte_at(offset + index).unwrap_or(0))
        .collect()
}

/// Read the control's current UI value.
pub fn read(patch: &Patch, param: &Param) -> u32 {
    if patch.bytes.len() <= param.offset {
        return match param.kind {
            Kind::Knob => 0,
            Kind::Enum(choices) => {
                u32::from(choices.first().map(|choice| choice.wire).unwrap_or(0))
            }
            _ => 0,
        };
    }
    match param.kind {
        Kind::Knob => raw_to_knob(patch.u16_at(param.offset).unwrap_or(0)),
        Kind::Byte => u32::from(patch.bytes[param.offset]),
        Kind::Bool => u32::from(patch.bytes[param.offset] != 0),
        Kind::Bit => u32::from(patch.bytes[param.offset] & param.mask != 0),
        Kind::Enum(choices) => {
            let byte = patch.bytes[param.offset] & if param.mask == 0 { 0xFF } else { param.mask };
            u32::from(
                choices
                    .iter()
                    .find(|choice| choice.wire & param.mask == byte)
                    .map(|choice| choice.wire)
                    .unwrap_or(byte),
            )
        }
    }
}

/// Write the control's UI value into the patch payload.
pub fn write(patch: &mut Patch, param: &Param, value: u32) {
    let value = match param.kind {
        Kind::Knob => value.min(999),
        Kind::Byte | Kind::Enum(_) => value.min(u32::from(u8::MAX)),
        Kind::Bool | Kind::Bit => u32::from(value != 0),
    };
    match param.kind {
        Kind::Knob => patch.set_u16(param.offset, knob_to_raw(value)),
        Kind::Byte => patch.set_byte(param.offset, value as u8),
        Kind::Bool => patch.set_byte(param.offset, u8::from(value != 0)),
        Kind::Bit => {
            let old = patch.byte_at(param.offset).unwrap_or(0);
            let byte = if value != 0 {
                old | param.mask
            } else {
                old & !param.mask
            };
            patch.set_byte(param.offset, byte);
        }
        Kind::Enum(_) => {
            let old = patch.byte_at(param.offset).unwrap_or(0);
            let byte = (u32::from(old) & u32::from(!param.mask))
                | (value & u32::from(if param.mask == 0 { 0xFF } else { param.mask }));
            patch.set_byte(param.offset, byte as u8);
        }
    }
}

/// Display text for a control value, using the enum labels where available.
pub fn label(param: &Param, value: u32) -> String {
    match param.kind {
        Kind::Knob => value.to_string(),
        Kind::Byte => value.to_string(),
        Kind::Bool | Kind::Bit => {
            if value == 0 {
                "off".to_string()
            } else {
                "on".to_string()
            }
        }
        Kind::Enum(choices) => choices
            .iter()
            .find(|choice| {
                u32::from(choice.wire & if param.mask == 0 { 0xFF } else { param.mask }) == value
            })
            .map(|choice| choice.label.to_string())
            .unwrap_or_else(|| format!("{value}")),
    }
}

/// CC value for a control value, used for the live preview.
pub fn cc_value(param: &Param, value: u32) -> Option<u8> {
    param.cc.map(|_| match param.kind {
        Kind::Knob => knob_to_cc(value),
        Kind::Byte => value as u8,
        Kind::Bool | Kind::Bit => {
            if value == 0 {
                0
            } else {
                64
            }
        }
        Kind::Enum(choices) => choices
            .iter()
            .find(|choice| {
                u32::from(choice.wire & if param.mask == 0 { 0xFF } else { param.mask }) == value
            })
            .map(|choice| choice.cc)
            .unwrap_or(0),
    })
}

/// Read the name field from a patch (already public as [`Patch::name`]; kept
/// here so the UI has one import for the control surface).
pub fn name(patch: &Patch) -> String {
    patch.name()
}

/// Write the name field, rejecting over-long names.
pub fn set_name(patch: &mut Patch, name: &str) -> Result<(), String> {
    patch.set_name(name)
}

pub const NAME_FIELD: (usize, usize) = (NAME_OFFSET, NAME_LEN);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_and_cc_numbers_are_unique() {
        let mut ids = HashSet::new();
        let mut ccs = HashSet::new();
        for param in PARAMS {
            assert!(ids.insert(param.id), "duplicate id {}", param.id);
            if let Some(cc) = param.cc {
                assert!(ccs.insert(cc), "duplicate CC {cc} on {}", param.id);
            }
        }
    }

    #[test]
    fn every_section_has_controls_and_every_control_is_placed() {
        for name in SECTIONS {
            assert!(section(name).count() > 0, "{name} is empty");
        }
        for param in PARAMS {
            assert!(SECTIONS.contains(&param.section), "{}", param.id);
        }
    }

    #[test]
    fn knob_encoding_round_trips_the_editor_scale() {
        for knob in [0, 1, 250, 500, 999] {
            assert_eq!(raw_to_knob(knob_to_raw(knob)), knob);
        }
        assert_eq!(knob_to_raw(999), 65_535);
        assert_eq!(knob_to_cc(999), 127);
        assert_eq!(knob_to_cc(0), 0);
    }

    #[test]
    fn shared_lfo_target_byte_preserves_the_other_bits() {
        let mut patch = Patch::init(0);
        let target = param("lfo.target").unwrap_or_else(|| panic!("lfo.target"));
        let freq = param("lfo.to_freq").unwrap_or_else(|| panic!("lfo.to_freq"));
        let pw = param("lfo.to_pw").unwrap_or_else(|| panic!("lfo.to_pw"));
        write(&mut patch, freq, 1);
        write(&mut patch, pw, 1);
        write(&mut patch, target, 16);
        assert_eq!(patch.byte_at(66), Some(16 | 1 | 4));
        assert_eq!(read(&patch, target), 16);
        assert_eq!(read(&patch, freq), 1);
        write(&mut patch, freq, 0);
        assert_eq!(patch.byte_at(66), Some(16 | 4));
        assert_eq!(read(&patch, pw), 1);
    }

    #[test]
    fn enums_map_to_wire_values_and_labels() {
        let mut patch = Patch::init(0);
        let arp = param("arp.mode").unwrap_or_else(|| panic!("arp.mode"));
        write(&mut patch, arp, 5);
        assert_eq!(patch.byte_at(149), Some(5));
        assert_eq!(label(arp, read(&patch, arp)), "Random");
        assert_eq!(cc_value(arp, 6), Some(110));
        let shape = param("lfo.shape").unwrap_or_else(|| panic!("lfo.shape"));
        assert_eq!(label(shape, 3), "Sine");
    }

    #[test]
    fn chord_memory_encodes_its_three_modes() {
        let mut patch = Patch::init(0);
        set_chord(&mut patch, &Chord::FullStack);
        assert_eq!(chord(&patch), Chord::FullStack);
        set_chord(&mut patch, &Chord::Mono);
        assert_eq!(chord(&patch), Chord::Mono);
        let memory = Chord::Memory(vec![Some(5), Some(7), None, None, None, None, None]);
        set_chord(&mut patch, &memory);
        assert_eq!(chord(&patch), memory);
        assert_eq!(patch.byte_at(89), Some(CHORD_NONE));
    }

    #[test]
    fn missing_bytes_read_as_zero() {
        let patch = Patch::new(0, vec![0u8; 8]);
        for param in PARAMS {
            let _ = read(&patch, param);
        }
        let mut patch = patch;
        let cutoff = param("filter.cutoff").unwrap_or_else(|| panic!("cutoff"));
        write(&mut patch, cutoff, 500);
        assert_eq!(patch.bytes.len(), 21);
        assert_eq!(read(&patch, cutoff), 500);
    }
}
