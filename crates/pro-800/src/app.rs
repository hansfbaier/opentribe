//! `egui` port of the Swumpf PRO-800 editor.
//!
//! One edit buffer, four tabs: the Swumpf control surface, a 400-slot
//! librarian, the 40-byte global block and a raw SysEx console. Live edits send
//! the same MIDI CCs the browser editor sends; storing a patch is always a
//! whole-patch read-modify-write, read back and compared.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use tribe_core::log::{Dir, Log};
use tribe_core::midi::{self, MidiHub};
use tribe_core::sysex;
use tribe_core::widgets::{self, DANGER, HEADING, MUTED, OK, WARN};

use crate::librarian::{self, Librarian, SlotState};
use crate::params::{self, Chord, GlobalParam, Kind, Param};
use crate::patch::{self, FAMILY, MODEL, Patch};
use crate::transport::{self, Status};

const PACE: Duration = Duration::from_millis(30);
const REPLY_TIMEOUT: Duration = Duration::from_millis(1200);
const MAX_ATTEMPTS: u8 = 3;

const PALETTE: [Color32; 7] = [
    Color32::from_rgb(0x8A, 0xB4, 0xF8),
    Color32::from_rgb(0x7F, 0xD7, 0x8A),
    Color32::from_rgb(0xE8, 0xB3, 0x4A),
    Color32::from_rgb(0xD9, 0x8B, 0xE0),
    Color32::from_rgb(0x74, 0xC7, 0xD4),
    Color32::from_rgb(0xE5, 0x7A, 0x7A),
    Color32::from_rgb(0xB6, 0xC4, 0x8A),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Patch,
    Library,
    Global,
    Console,
}

impl Tab {
    const ALL: [(Tab, &'static str); 4] = [
        (Tab::Patch, "Patch"),
        (Tab::Library, "Librarian"),
        (Tab::Global, "Global"),
        (Tab::Console, "Console"),
    ];
}

#[derive(Clone, Debug)]
enum PendingKind {
    Load(u16),
    Scan,
    Verify { slot: u16, expected: Vec<u8> },
    Clear(u16),
    Globals,
    GlobalWrite(Vec<u8>),
    Status,
    Mode,
}

struct Pending {
    kind: PendingKind,
    sent: Instant,
    attempts: u8,
}

struct Scan {
    current: u16,
    last: u16,
}

pub struct Pro800App {
    hub: MidiHub,
    out_ports: Vec<String>,
    in_ports: Vec<String>,
    selected_out: Option<String>,
    selected_in: Option<String>,
    channel: u8,
    live_cc: bool,
    status: Status,
    log: Log,
    tab: Tab,

    slot: u16,
    slot_buf: String,
    slot_error: Option<String>,
    patch: Option<Patch>,
    snapshot: Option<Patch>,
    name_buf: String,
    dirty: bool,

    pending: Option<Pending>,
    next_send: Option<Instant>,
    write_wait: Option<(u16, Vec<u8>)>,
    clear_sent: Option<(u16, Instant)>,
    write_queue: VecDeque<Patch>,
    scan: Option<Scan>,
    backup_dir: Option<PathBuf>,
    globals: Option<Vec<u8>>,
    globals_dirty: bool,
    confirm_globals: bool,
    confirm_clear: bool,
    hex_input: String,
    hex_error: Option<String>,
    lib: Librarian,
    bank: u8,
    staged_import: Option<(Vec<Patch>, String)>,
    clipboard: Option<Patch>,
    booted: bool,
}

impl Pro800App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self {
            hub: MidiHub::default(),
            out_ports: Vec::new(),
            in_ports: Vec::new(),
            selected_out: None,
            selected_in: None,
            channel: 1,
            live_cc: true,
            status: Status::default(),
            log: Log::default(),
            tab: Tab::Patch,
            slot: 0,
            slot_buf: patch::slot_label(0),
            slot_error: None,
            patch: None,
            snapshot: None,
            name_buf: String::new(),
            dirty: false,
            pending: None,
            next_send: None,
            write_wait: None,
            clear_sent: None,
            write_queue: VecDeque::new(),
            scan: None,
            backup_dir: None,
            globals: None,
            globals_dirty: false,
            confirm_globals: false,
            confirm_clear: false,
            hex_input: String::new(),
            hex_error: None,
            lib: Librarian::new(),
            bank: 0,
            staged_import: None,
            clipboard: None,
            booted: false,
        };
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = egui::vec2(6.0, 5.0);
            style.spacing.button_padding = egui::vec2(8.0, 3.0);
            style.visuals.panel_fill = Color32::from_rgb(0x17, 0x17, 0x17);
            style.visuals.window_fill = Color32::from_rgb(0x1E, 0x1E, 0x1E);
        });
        app.log.info(format!(
            "{} · editor port of Swumpf v114 · model {} · live CC on channel {}",
            crate::DEVICE_NAME,
            sysex::hex(&MODEL),
            app.channel
        ));
        app.log.info(
            "offsets are the editor's map; only the name field and pitch range are hardware-verified",
        );
        app.refresh_ports();
        app.auto_connect(cc.egui_ctx.clone());
        app
    }

    /// Select a program slot and keep the editable field in sync.
    fn set_slot(&mut self, index: u16) {
        let index = index.min(patch::MAX_SLOT);
        self.slot = index;
        self.slot_buf = patch::slot_label(index);
        self.slot_error = None;
    }

    // ------------------------------------------------------------ connection

    fn refresh_ports(&mut self) {
        match midi::output_ports() {
            Ok(ports) => self.out_ports = ports,
            Err(err) => {
                self.out_ports.clear();
                self.log.error(format!("MIDI output init failed: {err}"));
            }
        }
        match midi::input_ports() {
            Ok(ports) => self.in_ports = ports,
            Err(err) => {
                self.in_ports.clear();
                self.log.error(format!("MIDI input init failed: {err}"));
            }
        }
        if self.selected_out.is_none() {
            self.selected_out = midi::guess_port(&self.out_ports, crate::PORT_HINT);
        }
        if self.selected_in.is_none() {
            self.selected_in = midi::guess_port(&self.in_ports, crate::PORT_HINT);
        }
    }

    fn auto_connect(&mut self, ctx: egui::Context) {
        if self
            .selected_out
            .clone()
            .is_none_or(|name| !self.out_ports.contains(&name))
        {
            self.log.info(format!(
                "no {} MIDI output found ({} visible); pick a port and connect",
                crate::DEVICE_NAME,
                self.out_ports.len()
            ));
            return;
        }
        if self.selected_in.clone().as_deref() != self.selected_out.clone().as_deref() {
            self.selected_in = self.selected_out.clone();
        }
        let notify = move || ctx.request_repaint();
        let out = self.selected_out.clone().unwrap_or_default();
        let inp = self.selected_in.clone().unwrap_or_default();
        match self.hub.connect_output(&out) {
            Ok(()) => self.log.info(format!("output opened: {out}")),
            Err(err) => {
                self.log.error(format!("{out}: {err}"));
                return;
            }
        }
        match self.hub.connect_input(&inp, notify) {
            Ok(()) => self.log.info(format!("input opened: {inp}")),
            Err(err) => self.log.error(format!("{inp}: {err}")),
        }
        self.send_status();
    }

    fn connect(&mut self, ctx: egui::Context) {
        if let Some(out) = self.selected_out.clone() {
            match self.hub.connect_output(&out) {
                Ok(()) => {
                    self.log.info(format!("output opened: {out}"));
                    self.pending = None;
                    self.status = Status::default();
                    self.globals = None;
                    self.send_status();
                }
                Err(err) => self.log.error(format!("{out}: {err}")),
            }
        }
        if let Some(inp) = self.selected_in.clone() {
            match self.hub.connect_input(&inp, move || ctx.request_repaint()) {
                Ok(()) => self.log.info(format!("input opened: {inp}")),
                Err(err) => self.log.error(format!("{inp}: {err}")),
            }
        }
    }

    fn connected(&self) -> bool {
        self.hub.output_open()
    }

    // ---------------------------------------------------------------- traffic

    fn send_now(&mut self, label: &str, bytes: Vec<u8>) {
        self.next_send = Some(Instant::now() + PACE);
        if !self.connected() {
            self.log
                .push(Dir::Info, format!("dry run · {label}"), &bytes);
            return;
        }
        match self.hub.send(&bytes) {
            Ok(()) => self.log.push(Dir::Tx, label, &bytes),
            Err(err) => self.log.error(format!("{label}: {err}")),
        }
    }

    fn send_request(&mut self, kind: PendingKind, label: &str, bytes: Vec<u8>) {
        self.send_now(label, bytes);
        self.pending = Some(Pending {
            kind,
            sent: Instant::now(),
            attempts: 1,
        });
    }

    fn send_status(&mut self) {
        if self.pending.is_some() || !self.connected() {
            return;
        }
        self.send_request(
            PendingKind::Status,
            "query firmware",
            transport::firmware_request(),
        );
    }

    fn send_mode(&mut self) {
        if self.pending.is_some() {
            return;
        }
        self.send_request(PendingKind::Mode, "query mode", transport::mode_request());
    }

    fn send_globals(&mut self) {
        if self.pending.is_some()
            || self.scan.is_some()
            || self.write_wait.is_some()
            || self.clear_sent.is_some()
            || !self.write_queue.is_empty()
            || !self.connected()
        {
            return;
        }
        self.send_request(
            PendingKind::Globals,
            "read globals",
            patch::globals_read_frame(),
        );
    }

    fn queue_load(&mut self, index: u16) {
        if self.pending.is_some()
            || self.scan.is_some()
            || self.write_wait.is_some()
            || self.clear_sent.is_some()
            || !self.write_queue.is_empty()
            || !self.connected()
        {
            return;
        }
        self.send_request(
            PendingKind::Load(index),
            &format!("read {}", patch::slot_label(index)),
            patch::read_frame(index),
        );
    }

    fn send_cc(&mut self, param: &Param, value: u32) {
        if !self.live_cc {
            return;
        }
        let Some(cc) = param.cc else { return };
        let Some(cc_value) = params::cc_value(param, value) else {
            return;
        };
        let status = 0xB0 | (self.channel.clamp(1, 16) - 1);
        let bytes = [status, cc, cc_value];
        if self.connected()
            && let Err(err) = self.hub.send(&bytes)
        {
            self.log.error(format!("live CC {cc}: {err}"));
        }
        self.log.push(
            Dir::Tx,
            format!("live CC {cc} = {cc_value} ({})", param.title),
            &bytes,
        );
    }

    // ---------------------------------------------------------------- polling

    fn tick(&mut self, ctx: &egui::Context) {
        for message in self.hub.poll() {
            self.handle_incoming(&message);
        }
        self.handle_timeout();
        self.pump();
        let busy = self.pending.is_some()
            || self.scan.is_some()
            || self.write_wait.is_some()
            || self.clear_sent.is_some()
            || !self.write_queue.is_empty();
        ctx.request_repaint_after(if busy {
            Duration::from_millis(20)
        } else {
            Duration::from_millis(250)
        });
    }

    fn handle_incoming(&mut self, message: &[u8]) {
        if transport::is_empty_reply(message) {
            if let Some(index) = self.pending_slot() {
                let state = SlotState::Empty;
                self.store_slot(index, state.clone());
                self.finish_slot_request(index, &state, message);
            } else {
                self.log.push(Dir::Rx, "empty slot reply", message);
            }
            return;
        }
        if let Some((index, reply)) = patch::patch_reply(message) {
            let state = match reply {
                Some(ref patch) => SlotState::Patch(patch.clone()),
                None => SlotState::Empty,
            };
            self.store_slot(index, state.clone());
            self.finish_slot_request(index, &state, message);
            return;
        }
        if let Some(block) = patch::global_reply(message) {
            let expected = match self.pending.as_ref().map(|pending| &pending.kind) {
                Some(PendingKind::GlobalWrite(expected)) => Some(expected.clone()),
                _ => None,
            };
            self.log.push(
                Dir::Rx,
                format!("global block · {} bytes", block.len()),
                message,
            );
            self.globals = Some(block.clone());
            self.globals_dirty = false;
            self.pending = None;
            if let Some(expected) = expected {
                if patch::equivalent(&block, &expected) {
                    self.log
                        .push(Dir::Rx, "global write verified by read-back", message);
                } else {
                    self.log
                        .error("global write read-back differs (unverified frame)");
                }
            } else if !self.booted {
                // The global block names the program the panel has selected;
                // load that one, as the browser editor does.
                self.booted = true;
                let selected = block
                    .get(5)
                    .copied()
                    .zip(block.get(6).copied())
                    .map(|(lsb, msb)| patch::global_index(lsb, msb).min(patch::MAX_SLOT))
                    .unwrap_or(0);
                self.set_slot(selected);
                self.queue_load(selected);
            }
            return;
        }
        if let Some(body) = sysex::parse(MODEL, FAMILY, message).map(|parsed| parsed.body) {
            if body.first() == Some(&0x09) {
                if let Some((major, minor, patch)) = transport::parse_firmware(body) {
                    self.status.firmware = Some((major, minor, patch));
                    self.log.info(format!(
                        "firmware {major}.{minor}.{patch} · known build {}",
                        crate::KNOWN_FIRMWARE
                    ));
                    if format!("{major}.{minor}.{patch}") != crate::KNOWN_FIRMWARE {
                        self.log.error(format!(
                            "firmware differs from {} — offsets may not apply",
                            crate::KNOWN_FIRMWARE
                        ));
                    }
                }
                self.pending = None;
                self.send_mode();
                return;
            }
            if body.first() == Some(&0x03) {
                self.status.mode = body.get(1).copied();
                self.log.info(format!(
                    "mode · {}",
                    match self.status.mode {
                        Some(0) => "normal".to_string(),
                        Some(mode) => format!("{mode:#04X}"),
                        None => "unknown".to_string(),
                    }
                ));
                self.pending = None;
                self.send_globals();
                return;
            }
        }
        self.log.push(Dir::Rx, "SysEx", message);
    }

    fn pending_slot(&self) -> Option<u16> {
        match self.pending.as_ref().map(|pending| &pending.kind) {
            Some(PendingKind::Load(slot)) => Some(*slot),
            Some(PendingKind::Scan) => self.scan.as_ref().map(|scan| scan.current),
            Some(PendingKind::Verify { slot, .. }) => Some(*slot),
            Some(PendingKind::Clear(slot)) => Some(*slot),
            _ => None,
        }
    }

    fn finish_slot_request(&mut self, index: u16, state: &SlotState, message: &[u8]) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let matches = match &pending.kind {
            PendingKind::Load(slot) => *slot == index,
            PendingKind::Scan => self.scan.as_ref().is_some_and(|scan| scan.current == index),
            PendingKind::Verify { slot, .. } => *slot == index,
            PendingKind::Clear(slot) => *slot == index,
            _ => false,
        };
        if !matches {
            return;
        }
        let kind = pending.kind.clone();
        self.pending = None;
        match kind {
            PendingKind::Load(slot) => match state {
                SlotState::Patch(patch) => {
                    self.log.push(
                        Dir::Rx,
                        format!("{} · {}", patch::slot_label(slot), patch.name()),
                        message,
                    );
                    self.load_patch(patch.clone());
                }
                SlotState::Empty => {
                    self.log.push(
                        Dir::Rx,
                        format!("{} · empty", patch::slot_label(slot)),
                        message,
                    );
                    self.patch = None;
                    self.snapshot = None;
                    self.name_buf.clear();
                    self.dirty = false;
                }
                SlotState::Unknown => {}
            },
            PendingKind::Scan => self.advance_scan(),
            PendingKind::Clear(slot) => match state {
                SlotState::Empty => {
                    self.log.push(
                        Dir::Rx,
                        format!("{} · cleared", patch::slot_label(slot)),
                        message,
                    );
                    if self.slot == slot {
                        self.patch = None;
                        self.snapshot = None;
                        self.name_buf.clear();
                        self.dirty = false;
                    }
                }
                SlotState::Patch(patch) => self.log.error(format!(
                    "{} · still holds {:?} after the delete",
                    patch::slot_label(slot),
                    patch.name()
                )),
                SlotState::Unknown => {}
            },
            PendingKind::Verify { slot, expected } => {
                let ok = state
                    .patch()
                    .is_some_and(|patch| patch::equivalent(&patch.bytes, &expected));
                if ok {
                    self.log.push(
                        Dir::Rx,
                        format!("{} · write verified", patch::slot_label(slot)),
                        message,
                    );
                    if let Some(read_back) = state.patch()
                        && self.slot == slot
                        && self
                            .patch
                            .as_ref()
                            .is_some_and(|patch| patch::equivalent(&patch.bytes, &expected))
                    {
                        self.patch = Some(read_back.clone());
                        self.snapshot = Some(read_back.clone());
                        self.name_buf = read_back.name();
                        self.dirty = false;
                    }
                } else {
                    self.log.error(format!(
                        "{} · read-back differs; patch not stored",
                        patch::slot_label(slot)
                    ));
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    fn store_slot(&mut self, index: u16, state: SlotState) {
        if let Some(dir) = &self.backup_dir {
            let label = patch::slot_label(index);
            let frame = match &state {
                SlotState::Patch(patch) => patch.record_frame(),
                _ => patch::empty_frame(index),
            };
            let path = dir.join(format!("{label}.syx"));
            if let Err(err) = std::fs::write(&path, frame) {
                self.log.error(format!("{}: {err}", path.display()));
                self.backup_dir = None;
            }
        }
        self.lib.set(index, state);
    }

    fn advance_scan(&mut self) {
        let Some(scan) = self.scan.as_mut() else {
            return;
        };
        if scan.current >= scan.last {
            self.log.info("scan finished");
            self.scan = None;
            if self.backup_dir.is_some() {
                self.finish_backup();
            }
        } else {
            scan.current += 1;
            self.next_send = Some(Instant::now() + PACE);
        }
    }

    fn finish_backup(&mut self) {
        let Some(dir) = self.backup_dir.clone() else {
            return;
        };
        match std::fs::create_dir_all(&dir) {
            Ok(()) => {}
            Err(err) => {
                self.log.error(format!("{}: {err}", dir.display()));
                return;
            }
        }
        let path = dir.join("manifest.csv");
        let manifest = librarian::manifest_csv(&self.lib);
        match std::fs::write(&path, manifest) {
            Ok(()) => self.log.info(format!(
                "backup · {} programs in {}",
                self.lib.filled(),
                dir.display()
            )),
            Err(err) => self.log.error(format!("{}: {err}", path.display())),
        }
        self.backup_dir = None;
    }

    fn handle_timeout(&mut self) {
        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        if pending.sent.elapsed() < REPLY_TIMEOUT {
            return;
        }
        let kind = pending.kind.clone();
        if pending.attempts < MAX_ATTEMPTS {
            pending.attempts += 1;
            pending.sent = Instant::now();
            let bytes = match &kind {
                PendingKind::Load(slot) => patch::read_frame(*slot),
                PendingKind::Scan => {
                    patch::read_frame(self.scan.as_ref().map(|scan| scan.current).unwrap_or(0))
                }
                PendingKind::Verify { slot, .. } | PendingKind::Clear(slot) => {
                    patch::read_frame(*slot)
                }
                PendingKind::Globals | PendingKind::GlobalWrite(_) => patch::globals_read_frame(),
                PendingKind::Status => transport::firmware_request(),
                PendingKind::Mode => transport::mode_request(),
            };
            self.log.error(format!(
                "no reply; retry {}/{MAX_ATTEMPTS}",
                pending.attempts
            ));
            self.send_now("retry", bytes);
            return;
        }
        self.log.error("no reply; giving up");
        self.pending = None;
        if matches!(kind, PendingKind::Scan) {
            if let Some(scan) = self.scan.as_ref() {
                let index = scan.current;
                self.lib.set(index, SlotState::Unknown);
            }
            self.advance_scan();
        }
    }

    fn pump(&mut self) {
        if self.pending.is_some() {
            return;
        }
        if let Some(at) = self.next_send
            && Instant::now() < at
        {
            return;
        }
        if let Some((index, expected)) = self.write_wait.take() {
            self.send_request(
                PendingKind::Verify {
                    slot: index,
                    expected,
                },
                &format!("verify {}", patch::slot_label(index)),
                patch::read_frame(index),
            );
            return;
        }
        if let Some((index, at)) = self.clear_sent {
            if Instant::now() < at {
                return;
            }
            self.clear_sent = None;
            self.send_request(
                PendingKind::Clear(index),
                &format!("verify empty {}", patch::slot_label(index)),
                patch::read_frame(index),
            );
            return;
        }
        if let Some(next) = self.write_queue.pop_front() {
            let index = next.index;
            let expected = next.bytes.clone();
            let frame = next.write_frame();
            self.log.push(
                Dir::Tx,
                format!("write {} · {}", patch::slot_label(index), next.name()),
                &frame,
            );
            self.send_now(&format!("write {}", patch::slot_label(index)), frame);
            self.write_wait = Some((index, expected));
            return;
        }
        if let Some(scan) = self.scan.as_ref() {
            let frame = patch::read_frame(scan.current);
            let label = format!("read {}", patch::slot_label(scan.current));
            self.send_request(PendingKind::Scan, &label, frame);
        }
    }

    // ------------------------------------------------------------------ patch

    fn load_patch(&mut self, patch: Patch) {
        self.set_slot(patch.index);
        self.name_buf = patch.name();
        self.snapshot = Some(patch.clone());
        self.patch = Some(patch);
        self.dirty = false;
    }

    fn read_slot(&mut self) {
        if !self.connected() {
            self.log.error("connect a MIDI output first");
            return;
        }
        self.queue_load(self.slot);
    }

    fn write_slot(&mut self) {
        if !self.connected() {
            self.log.error("connect a MIDI output first");
            return;
        }
        let Some(mut patch) = self.patch.clone() else {
            self.log.error("no patch loaded");
            return;
        };
        patch.index = self.slot;
        if self.name_buf != patch.name()
            && let Err(err) = patch.set_name(&self.name_buf)
        {
            self.log.error(err);
            return;
        }
        self.patch = Some(patch.clone());
        self.write_queue.push_back(patch);
        self.next_send = Some(Instant::now());
    }

    fn init_patch(&mut self) {
        self.load_patch(Patch::init(self.slot));
        self.dirty = true;
    }

    fn revert(&mut self) {
        if let Some(snapshot) = self.snapshot.clone() {
            self.name_buf = snapshot.name();
            self.patch = Some(snapshot);
            self.dirty = false;
        }
    }

    fn ensure_patch(&mut self) -> Option<&mut Patch> {
        if self.patch.is_none() {
            self.patch = Some(Patch::init(self.slot));
            self.name_buf = "Init Program".to_string();
            self.snapshot = self.patch.clone();
        }
        self.patch.as_mut()
    }

    fn start_scan(&mut self, first: u16, last: u16) {
        if !self.connected() {
            self.log.error("connect a MIDI output first");
            return;
        }
        if self.scan.is_some()
            || self.pending.is_some()
            || self.write_wait.is_some()
            || self.clear_sent.is_some()
            || !self.write_queue.is_empty()
        {
            self.log.info("already busy");
            return;
        }
        self.scan = Some(Scan {
            current: first,
            last,
        });
        self.log.info(format!(
            "scan {} … {}",
            patch::slot_label(first),
            patch::slot_label(last)
        ));
    }

    fn start_backup(&mut self) {
        let Some(dir) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        if let Err(err) = std::fs::create_dir_all(&dir) {
            self.log.error(format!("{}: {err}", dir.display()));
            return;
        }
        self.backup_dir = Some(dir);
        self.start_scan(0, patch::MAX_SLOT);
        if self.scan.is_some() {
            self.log
                .info("backup: reading all 400 slots and writing .syx files");
        }
    }

    fn import_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("SysEx", &["syx"])
            .pick_file()
        else {
            return;
        };
        match patch::load_syx(&path) {
            Ok(mut patches) => {
                patches.sort_by_key(|patch| patch.index);
                let count = patches.len();
                let mut index = self.slot;
                for patch in &mut patches {
                    patch.index = index;
                    index = (index + 1).min(patch::MAX_SLOT);
                }
                let source = path.display().to_string();
                self.staged_import = Some((patches, source.clone()));
                self.log.info(format!(
                    "staged {count} program(s) from {source}; confirm before writing"
                ));
            }
            Err(err) => self.log.error(err),
        }
    }

    fn confirm_import(&mut self) {
        let Some((patches, source)) = self.staged_import.take() else {
            return;
        };
        let count = patches.len();
        let first = patches.first().map(|patch| patch.index).unwrap_or(0);
        for patch in patches {
            self.write_queue.push_back(patch);
        }
        self.next_send = Some(Instant::now());
        self.log.info(format!(
            "importing {count} program(s) from {source} starting at {}",
            patch::slot_label(first)
        ));
    }

    fn export_slot(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("SysEx", &["syx"])
            .set_file_name(format!("{}.syx", patch::slot_label(self.slot)))
            .save_file()
        else {
            return;
        };
        let patch = self
            .patch
            .clone()
            .or_else(|| self.lib.get(self.slot).patch().cloned());
        let Some(patch) = patch else {
            self.log.error("nothing to export");
            return;
        };
        match std::fs::write(&path, patch.record_frame()) {
            Ok(()) => self.log.info(format!(
                "exported {} to {}",
                patch::slot_label(patch.index),
                path.display()
            )),
            Err(err) => self.log.error(format!("{}: {err}", path.display())),
        }
    }

    fn request_clear(&mut self, index: u16) {
        if !self.connected() {
            self.log.error("connect a MIDI output first");
            return;
        }
        if self.pending.is_some()
            || self.scan.is_some()
            || self.write_wait.is_some()
            || self.clear_sent.is_some()
            || !self.write_queue.is_empty()
        {
            self.log.info("already busy");
            return;
        }
        self.send_now(
            &format!("delete {}", patch::slot_label(index)),
            patch::empty_frame(index),
        );
        self.clear_sent = Some((index, Instant::now() + PACE));
        self.log.info(format!(
            "delete {} · read-back will confirm the slot is empty",
            patch::slot_label(index)
        ));
    }

    fn copy_patch(&mut self) {
        let source = self
            .lib
            .get(self.slot)
            .patch()
            .cloned()
            .or_else(|| self.patch.clone());
        match source {
            Some(patch) => {
                self.log.info(format!(
                    "copied {} · {}",
                    patch::slot_label(patch.index),
                    patch.name()
                ));
                self.clipboard = Some(patch);
            }
            None => self.log.error("nothing to copy"),
        }
    }

    fn paste_patch(&mut self) {
        let Some(mut patch) = self.clipboard.clone() else {
            return;
        };
        if !self.connected() {
            self.log.error("connect a MIDI output first");
            return;
        }
        let from = patch.index;
        patch.index = self.slot;
        self.log.info(format!(
            "pasting {} · {} into {} (overwrites)",
            patch::slot_label(from),
            patch.name(),
            patch::slot_label(self.slot)
        ));
        self.write_queue.push_back(patch);
        self.next_send = Some(Instant::now());
    }

    fn load_cached_or_read(&mut self, index: u16) {
        self.set_slot(index);
        if let Some(patch) = self.lib.get(index).patch().cloned() {
            self.load_patch(patch);
        } else {
            self.queue_load(index);
        }
    }

    // --------------------------------------------------------------------- UI

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(crate::DEVICE_NAME).strong().size(16.0));
            widgets::status_dot(ui, if self.connected() { OK } else { DANGER });
            ui.weak(if self.connected() {
                "connected"
            } else {
                "not connected"
            });
            if let Some((major, minor, patch)) = self.status.firmware {
                ui.label(
                    RichText::new(format!("firmware {major}.{minor}.{patch}"))
                        .color(
                            if format!("{major}.{minor}.{patch}") == crate::KNOWN_FIRMWARE {
                                OK
                            } else {
                                WARN
                            },
                        )
                        .size(11.5),
                );
            }
            ui.separator();
            ui.label("out");
            let out_width = 240.0;
            egui::ComboBox::from_id_salt("out_port")
                .width(out_width)
                .selected_text(
                    self.selected_out
                        .as_deref()
                        .map(widgets::short_port)
                        .unwrap_or_else(|| "select output".to_string()),
                )
                .show_ui(ui, |ui| {
                    for port in &self.out_ports {
                        let label = widgets::short_port(port);
                        if ui
                            .selectable_label(self.selected_out.as_ref() == Some(port), label)
                            .clicked()
                        {
                            self.selected_out = Some(port.clone());
                        }
                    }
                });
            ui.label("in");
            egui::ComboBox::from_id_salt("in_port")
                .width(200.0)
                .selected_text(
                    self.selected_in
                        .as_deref()
                        .map(widgets::short_port)
                        .unwrap_or_else(|| "select input".to_string()),
                )
                .show_ui(ui, |ui| {
                    for port in &self.in_ports {
                        let label = widgets::short_port(port);
                        if ui
                            .selectable_label(self.selected_in.as_ref() == Some(port), label)
                            .clicked()
                        {
                            self.selected_in = Some(port.clone());
                        }
                    }
                });
            if ui.button("Refresh").clicked() {
                self.refresh_ports();
            }
            if ui.button("Connect").clicked() {
                let ctx = ui.ctx().clone();
                self.connect(ctx);
            }
            ui.separator();
            ui.checkbox(&mut self.live_cc, "Live CC");
            ui.add(
                egui::DragValue::new(&mut self.channel)
                    .range(1..=16)
                    .prefix("ch "),
            );
        });
        ui.horizontal(|ui| {
            for (tab, title) in Tab::ALL {
                if ui.selectable_label(self.tab == tab, title).clicked() {
                    self.tab = tab;
                }
            }
            if self.dirty {
                ui.label(RichText::new("pending edits").color(WARN).size(11.0));
            }
        });
    }

    fn patch_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Slot").color(MUTED).size(11.0));
            let text_color = if self.slot_error.is_some() {
                DANGER
            } else {
                Color32::from_gray(0xE8)
            };
            let edit = ui.add_sized(
                [72.0, 22.0],
                egui::TextEdit::singleline(&mut self.slot_buf)
                    .font(egui::TextStyle::Monospace)
                    .align(egui::Align2::CENTER_CENTER)
                    .hint_text("B099")
                    .text_color(text_color),
            );
            if edit.lost_focus() {
                match patch::parse_slot(&self.slot_buf) {
                    Ok(index) => {
                        self.set_slot(index);
                        if self.connected() {
                            self.read_slot();
                        }
                    }
                    Err(err) => self.slot_error = Some(err),
                }
            }
            match &self.slot_error {
                Some(err) => {
                    edit.on_hover_text(err);
                    ui.label(RichText::new("invalid slot").color(DANGER).size(10.5));
                }
                None => {
                    edit.on_hover_text("Program slot: B099, b94, 194 or 194/0xC2");
                }
            }
            let slot_ok = self.slot_error.is_none();
            let name_width = 190.0;
            let response = ui.add_sized(
                [name_width, 22.0],
                egui::TextEdit::singleline(&mut self.name_buf)
                    .hint_text("patch name")
                    .char_limit(patch::NAME_LEN),
            );
            if response.changed() {
                self.dirty = true;
            }
            if ui.button("Set name").clicked() {
                let name = self.name_buf.clone();
                let outcome = match self.ensure_patch() {
                    Some(patch) => patch.set_name(&name).map(|()| patch.name()),
                    None => Ok(String::new()),
                };
                match outcome {
                    Ok(stored) => {
                        self.dirty = true;
                        self.log.info(format!("name set to {stored:?}"));
                    }
                    Err(err) => self.log.error(err),
                }
            }
            ui.separator();
            if ui.add_enabled(slot_ok, egui::Button::new("Read")).clicked() {
                self.read_slot();
            }
            let editor_matches_slot = self
                .patch
                .as_ref()
                .is_some_and(|patch| patch.index == self.slot);
            if ui
                .add_enabled(
                    slot_ok && self.connected() && editor_matches_slot,
                    egui::Button::new("Write"),
                )
                .clicked()
            {
                self.write_slot();
            }
            if let Some(patch) = &self.patch
                && patch.index != self.slot
            {
                ui.label(
                    RichText::new(format!("editor holds {}", patch::slot_label(patch.index)))
                        .color(WARN)
                        .size(10.5),
                );
            }
            if ui
                .add_enabled(self.snapshot.is_some(), egui::Button::new("Revert"))
                .clicked()
            {
                self.revert();
            }
            if ui.button("Init").clicked() {
                self.init_patch();
            }
            if self.dirty {
                ui.label(RichText::new("pending edits").color(WARN).size(11.0));
            }
        });
        ui.label(
            RichText::new(
                "live CC preview follows the knobs; Write stores the whole patch, padded to 173 bytes",
            )
            .color(MUTED)
            .size(10.5),
        );
        ui.separator();
        if self.patch.is_none() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label("No patch loaded.");
                ui.label(
                    RichText::new("Read a slot, press Init, or import a .syx file.").color(MUTED),
                );
                if ui.button("Read slot").clicked() {
                    self.read_slot();
                }
            });
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, section) in params::SECTIONS.iter().enumerate() {
                    let accent = PALETTE[index % PALETTE.len()];
                    self.section_ui(ui, section, accent);
                }
                self.chord_ui(ui);
            });
    }

    fn section_ui(&mut self, ui: &mut egui::Ui, section: &'static str, accent: Color32) {
        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(0x1D, 0x1D, 0x1D))
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(section.to_uppercase())
                        .strong()
                        .color(accent)
                        .size(12.0),
                );
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    for param in params::section(section) {
                        self.param_ui(ui, param);
                    }
                });
            });
        ui.add_space(4.0);
    }

    fn param_ui(&mut self, ui: &mut egui::Ui, param: &'static Param) {
        let Some(patch) = self.patch.as_ref() else {
            return;
        };
        let mut value = params::read(patch, param);
        let mut changed = false;
        ui.vertical(|ui| {
            ui.set_width(82.0);
            let tooltip = format!(
                "{} · offset {} · CC {} · {}",
                param.id,
                param.offset,
                param
                    .cc
                    .map(|cc| cc.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                param.evidence.tag()
            );
            ui.label(RichText::new(param.title).size(10.5).color(MUTED))
                .on_hover_text(tooltip);
            match param.kind {
                Kind::Knob => {
                    let response = knob_widget(ui, &mut value);
                    changed = response;
                    let drag = ui.add_sized(
                        [56.0, 16.0],
                        egui::DragValue::new(&mut value).range(0..=999).speed(2.0),
                    );
                    changed |= drag.changed();
                }
                Kind::Byte => {
                    changed = ui
                        .add(egui::DragValue::new(&mut value).range(0..=255))
                        .changed();
                }
                Kind::Bool | Kind::Bit => {
                    let mut flag = value != 0;
                    let checkbox_label = if flag { "on" } else { "off" };
                    if ui.checkbox(&mut flag, checkbox_label).changed() {
                        value = u32::from(flag);
                        changed = true;
                    }
                }
                Kind::Enum(choices) => {
                    let current = params::label(param, value);
                    egui::ComboBox::from_id_salt(param.id)
                        .width(96.0)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for choice in choices {
                                if ui
                                    .selectable_label(value == u32::from(choice.wire), choice.label)
                                    .clicked()
                                {
                                    value = u32::from(choice.wire);
                                    changed = true;
                                }
                            }
                        });
                }
            }
        });
        if changed {
            if let Some(patch) = self.patch.as_mut() {
                params::write(patch, param, value);
            }
            self.dirty = true;
            self.send_cc(param, value);
        }
    }

    fn chord_ui(&mut self, ui: &mut egui::Ui) {
        let Some(patch) = self.patch.as_ref() else {
            return;
        };
        let chord = params::chord(patch);
        let mut edited: Option<Vec<Option<u8>>> = None;
        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(0x1D, 0x1D, 0x1D))
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("CHORD MEMORY")
                        .strong()
                        .color(PALETTE[4])
                        .size(12.0),
                );
                ui.horizontal(|ui| {
                    let mut mode = match chord {
                        Chord::FullStack => 0u8,
                        Chord::Mono => 1,
                        Chord::Memory(_) => 2,
                    };
                    if ui.selectable_value(&mut mode, 0, "8 voice").clicked() {
                        self.set_chord(Chord::FullStack);
                    }
                    if ui.selectable_value(&mut mode, 1, "1 voice").clicked() {
                        self.set_chord(Chord::Mono);
                    }
                    if ui.selectable_value(&mut mode, 2, "Chord").clicked()
                        && matches!(chord, Chord::FullStack | Chord::Mono)
                    {
                        self.set_chord(Chord::Memory(vec![Some(0); params::CHORD_LEN]));
                    }
                    if let Chord::Memory(intervals) = &chord {
                        let mut next = intervals.clone();
                        ui.horizontal(|ui| {
                            for (slot, interval) in next.iter_mut().enumerate() {
                                let mut on = interval.is_some();
                                if ui.checkbox(&mut on, format!("{}", slot + 2)).changed() {
                                    *interval = if on { Some(0) } else { None };
                                }
                                if let Some(value) = interval {
                                    ui.add(
                                        egui::DragValue::new(value)
                                            .range(0..=params::CHORD_NONE - 1)
                                            .speed(0.2),
                                    );
                                }
                            }
                        });
                        if next != *intervals {
                            edited = Some(next);
                        }
                    }
                });
                ui.label(
                    RichText::new(
                        "bytes 87-93: 8 voice = 00, 1 voice = FF, chord = interval per extra voice",
                    )
                    .color(MUTED)
                    .size(10.5),
                );
            });
        if let Some(intervals) = edited {
            self.set_chord(Chord::Memory(intervals));
        }
        ui.add_space(4.0);
    }

    fn set_chord(&mut self, chord: Chord) {
        if let Some(patch) = self.patch.as_mut() {
            params::set_chord(patch, &chord);
            self.dirty = true;
        }
    }

    fn library_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for bank in 0..4u8 {
                if ui
                    .selectable_label(self.bank == bank, format!("Bank {}", (b'A' + bank) as char))
                    .clicked()
                {
                    self.bank = bank;
                }
            }
            ui.separator();
            if ui.button("Read bank").clicked() {
                let first = u16::from(self.bank) * 100;
                self.start_scan(first, first + 99);
            }
            if ui.button("Read all 400").clicked() {
                self.start_scan(0, patch::MAX_SLOT);
            }
            if ui.button("Backup…").clicked() {
                self.start_backup();
            }
            if ui.button("Import .syx…").clicked() {
                self.import_file();
            }
            if ui.button("Export slot…").clicked() {
                self.export_slot();
            }
            if ui.button("Copy").clicked() {
                self.copy_patch();
            }
            if ui
                .add_enabled(
                    self.connected() && self.clipboard.is_some(),
                    egui::Button::new("Paste"),
                )
                .clicked()
            {
                self.paste_patch();
            }
            if self.confirm_clear {
                if ui
                    .button(RichText::new("Confirm clear").color(DANGER))
                    .clicked()
                {
                    self.confirm_clear = false;
                    self.request_clear(self.slot);
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_clear = false;
                }
            } else if ui
                .add_enabled(self.connected(), egui::Button::new("Clear slot…"))
                .clicked()
            {
                self.confirm_clear = true;
            }
            ui.separator();
            if ui.button("Load selected").clicked() {
                self.load_cached_or_read(self.slot);
                self.tab = Tab::Patch;
            }
            ui.weak(format!(
                "{} of 400 known · {} populated",
                self.lib.slots().len() - self.unknown_slots(),
                self.lib.filled()
            ));
        });
        if let Some((patches, source)) = self.staged_import.clone() {
            let first = patches.first().map(|patch| patch.index).unwrap_or(0);
            let last = patches.last().map(|patch| patch.index).unwrap_or(0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "Import {} program(s) from {source} into {} … {}?",
                        patches.len(),
                        patch::slot_label(first),
                        patch::slot_label(last)
                    ))
                    .color(WARN),
                );
                if ui.button("Write").clicked() {
                    self.confirm_import();
                }
                if ui.button("Cancel").clicked() {
                    self.staged_import = None;
                }
            });
        }
        if let Some(scan) = &self.scan {
            let label = format!(
                "scanning {} / {}",
                patch::slot_label(scan.current),
                patch::slot_label(scan.last)
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).color(WARN));
                if ui.button("Cancel").clicked() {
                    self.scan = None;
                    self.pending = None;
                    if self.backup_dir.is_some() {
                        self.finish_backup();
                    }
                    self.log.info("scan cancelled; partial results kept");
                }
            });
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("slot_grid")
                    .spacing([4.0, 4.0])
                    .show(ui, |ui| {
                        for row in 0..10u16 {
                            for column in 0..10u16 {
                                let index = u16::from(self.bank) * 100 + row * 10 + column;
                                self.slot_cell(ui, index);
                            }
                            ui.end_row();
                        }
                    });
            });
    }

    fn unknown_slots(&self) -> usize {
        self.lib
            .slots()
            .iter()
            .filter(|state| matches!(state, SlotState::Unknown))
            .count()
    }

    fn slot_cell(&mut self, ui: &mut egui::Ui, index: u16) {
        let state = self.lib.get(index);
        let name = match state {
            SlotState::Patch(patch) => short_name(&patch.name(), 14),
            SlotState::Empty => "(empty)".to_string(),
            SlotState::Unknown => "(not read)".to_string(),
        };
        let selected = self.slot == index;
        let text = RichText::new(format!("{}\n{}", patch::slot_label(index), name))
            .monospace()
            .size(10.5)
            .color(match state {
                SlotState::Patch(_) => {
                    if selected {
                        Color32::BLACK
                    } else {
                        Color32::from_gray(0xDC)
                    }
                }
                SlotState::Empty => MUTED,
                SlotState::Unknown => Color32::from_gray(0x66),
            });
        let fill = if selected {
            HEADING
        } else if matches!(state, SlotState::Patch(_)) {
            Color32::from_rgb(0x2A, 0x2A, 0x2A)
        } else {
            Color32::from_rgb(0x20, 0x20, 0x20)
        };
        let button = egui::Button::new(text)
            .fill(fill)
            .min_size(egui::vec2(102.0, 32.0));
        let response = ui.add(button);
        if response.clicked() {
            self.set_slot(index);
        }
        if response.double_clicked() {
            self.load_cached_or_read(index);
            self.tab = Tab::Patch;
        }
    }

    fn global_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Read globals").clicked() {
                self.send_globals();
            }
            if self.globals_dirty {
                ui.label(RichText::new("pending global edits").color(WARN));
            }
            if ui
                .add_enabled(
                    self.globals.is_some() && self.globals_dirty,
                    egui::Button::new(if self.confirm_globals {
                        "Confirm global write"
                    } else {
                        "Write globals…"
                    }),
                )
                .clicked()
            {
                if !self.confirm_globals {
                    self.confirm_globals = true;
                    self.log
                        .info("global write is unverified (gate G3); click again to send");
                } else {
                    self.confirm_globals = false;
                    self.write_globals();
                }
            }
            if self.confirm_globals && ui.button("Cancel").clicked() {
                self.confirm_globals = false;
            }
            ui.weak("write frame 78 7E 03 is the editor's, not firmware-verified");
        });
        ui.separator();
        let Some(mut block) = self.globals.clone() else {
            ui.label("No global block read yet.");
            if ui.button("Read globals").clicked() {
                self.send_globals();
            }
            return;
        };
        let mut changed = false;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("globals")
                    .num_columns(3)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        ui.strong("Active voices");
                        let mut mask = block.get(9).copied().unwrap_or(0);
                        let mut next = 0u8;
                        ui.horizontal(|ui| {
                            for voice in 0..8 {
                                let mut on = mask & (1 << voice) != 0;
                                if ui.checkbox(&mut on, format!("{}", voice + 1)).changed() {
                                    if on {
                                        next |= 1 << voice;
                                    }
                                } else if on {
                                    next |= 1 << voice;
                                }
                            }
                        });
                        if next != mask {
                            mask = next;
                            block[9] = mask;
                            changed = true;
                        }
                        ui.label("bit 0-7 · voice 1-8");
                        ui.end_row();

                        ui.strong("Note priority");
                        let current = block.get(26).copied().unwrap_or(0);
                        let mut chosen = current;
                        ui.horizontal(|ui| {
                            for (index, label) in ["Last", "Low", "High"].iter().enumerate() {
                                if ui
                                    .selectable_label(current == index as u8, *label)
                                    .clicked()
                                {
                                    chosen = index as u8;
                                }
                            }
                        });
                        if chosen != current {
                            block[26] = chosen;
                            changed = true;
                        }
                        ui.label("offset 26");
                        ui.end_row();

                        ui.strong("Transpose");
                        let mut semitones = block.get(37).copied().unwrap_or(0) as i8;
                        if ui
                            .add(egui::DragValue::new(&mut semitones).range(-12..=12))
                            .changed()
                        {
                            block[37] = semitones as u8;
                            changed = true;
                        }
                        ui.label("offset 37 · two's complement");
                        ui.end_row();

                        for param in params::GLOBALS {
                            if matches!(
                                param.id,
                                "global.active_voices"
                                    | "global.note_priority"
                                    | "global.transpose"
                            ) {
                                continue;
                            }
                            self.global_row(ui, param, &block);
                        }
                    });
                ui.add_space(8.0);
                ui.strong("Program");
                ui.label(RichText::new(program_text(&block)).monospace());
                ui.add_space(8.0);
                ui.strong("Raw block");
                ui.label(
                    RichText::new(sysex::hex(&block))
                        .monospace()
                        .size(11.0)
                        .color(MUTED),
                );
            });
        if changed {
            self.globals = Some(block);
            self.globals_dirty = true;
        }
    }

    fn global_row(&mut self, ui: &mut egui::Ui, param: &GlobalParam, block: &[u8]) {
        let value = block.get(param.offset).copied().unwrap_or(0);
        ui.strong(param.title);
        let text = match param.kind {
            Kind::Enum(choices) => choices
                .iter()
                .find(|choice| choice.wire == value)
                .map(|choice| choice.label.to_string())
                .unwrap_or_else(|| value.to_string()),
            _ => value.to_string(),
        };
        ui.label(RichText::new(format!("{text:<12} {value}")).monospace());
        ui.label(
            RichText::new(format!(
                "offset {} · {}",
                param.offset,
                if param.writable {
                    param.evidence.tag()
                } else {
                    "read-only until G3"
                }
            ))
            .color(MUTED)
            .size(10.5),
        );
        ui.end_row();
    }

    fn write_globals(&mut self) {
        if self.scan.is_some() || self.pending.is_some() {
            self.log.info("already busy");
            return;
        }
        let Some(block) = self.globals.clone() else {
            return;
        };
        let frame = patch::globals_write_frame(&block);
        self.send_now("write globals", frame);
        self.pending = Some(Pending {
            kind: PendingKind::GlobalWrite(block),
            sent: Instant::now(),
            attempts: 1,
        });
    }

    fn console_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("SysEx");
            ui.add(
                egui::TextEdit::singleline(&mut self.hex_input)
                    .hint_text("F0 00 20 32 00 01 24 00 77 7E 03 F7")
                    .desired_width(520.0)
                    .font(egui::TextStyle::Monospace),
            );
            if ui.button("Send").clicked() {
                match transport::raw_frame(&self.hex_input) {
                    Ok(bytes) => {
                        self.hex_error = None;
                        self.send_now("raw SysEx", bytes);
                    }
                    Err(err) => self.hex_error = Some(err),
                }
            }
            if ui.button("Read globals").clicked() {
                self.send_globals();
            }
            if ui.button("Read slot").clicked() {
                self.read_slot();
            }
        });
        if let Some(err) = &self.hex_error {
            ui.label(RichText::new(err).color(DANGER));
        }
        ui.separator();
        self.log_panel(ui);
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Log");
            ui.weak(format!("{} entries", self.log.len()));
            if ui.button("Clear").clicked() {
                self.log.clear();
            }
            ui.checkbox(&mut self.log.show_rx, "RX");
            ui.checkbox(&mut self.log.show_info, "info");
            ui.weak("dry run = message built but no output port is open");
        });
        let lines = self.log.visible();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .max_height(160.0)
            .show(ui, |ui| {
                for (dir, line) in lines {
                    ui.label(
                        RichText::new(line)
                            .monospace()
                            .size(11.0)
                            .color(dir.color()),
                    );
                }
            });
    }
}

impl eframe::App for Pro800App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.tick(ui.ctx());
        egui::Panel::top("top_bar").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(170.0)
            .min_size(80.0)
            .show(ui, |ui| {
                if self.tab != Tab::Console {
                    self.log_panel(ui);
                }
            });
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Patch => self.patch_tab(ui),
            Tab::Library => self.library_tab(ui),
            Tab::Global => self.global_tab(ui),
            Tab::Console => self.console_tab(ui),
        });
    }
}

/// Round knob painted as a 280° arc. Returns `true` when the value changed.
fn knob_widget(ui: &mut egui::Ui, value: &mut u32) -> bool {
    let size = egui::vec2(56.0, 50.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let mut changed = false;
    if response.dragged() {
        let delta = ui.input(|input| input.pointer.delta().y);
        if delta.abs() > 0.0 {
            let next = (*value as f32 - delta * 1.5).clamp(0.0, 999.0).round() as u32;
            if next != *value {
                *value = next;
                changed = true;
            }
        }
    }
    if response.double_clicked() {
        *value = 0;
        changed = true;
    }
    let center = rect.center();
    let radius = 19.0;
    let pointer = ui.painter();
    pointer.circle_filled(center, radius + 2.0, Color32::from_gray(0x0D));
    pointer.circle_filled(center, radius, Color32::from_gray(0x38));
    pointer.circle_stroke(
        center,
        radius,
        egui::Stroke::new(1.0, Color32::from_gray(0x55)),
    );
    let fraction = f32::from(*value as u16) / 999.0;
    let start = -135.0_f32.to_radians();
    let end = (-135.0 + 270.0 * fraction).to_radians();
    let arc: Vec<egui::Pos2> = (0..=28)
        .map(|step| {
            let angle = start + (end - start) * step as f32 / 28.0;
            center + egui::vec2(angle.sin(), -angle.cos()) * radius
        })
        .collect();
    pointer.add(egui::Shape::line(arc, egui::Stroke::new(2.5, HEADING)));
    let direction = egui::vec2(end.sin(), -end.cos());
    pointer.line_segment(
        [center, center + direction * (radius - 4.0)],
        egui::Stroke::new(2.5, Color32::from_gray(0xE0)),
    );
    response.on_hover_text("drag up/down · double-click resets to 0");
    changed
}

fn short_name(name: &str, width: usize) -> String {
    if name.chars().count() <= width {
        name.to_string()
    } else {
        let mut text: String = name.chars().take(width.saturating_sub(1)).collect();
        text.push('…');
        text
    }
}

fn program_text(block: &[u8]) -> String {
    let lsb = block.get(5).copied().unwrap_or(0);
    let msb = block.get(6).copied().unwrap_or(0);
    let bank = block.get(20).copied().unwrap_or(0);
    let index = patch::global_index(lsb, msb);
    format!(
        "{} · bank {} · index {}",
        patch::slot_label(index),
        (b'A' + bank.min(3)) as char,
        index
    )
}

/// `--screenshot` wrapper: draw, save a PNG, close.
pub struct ShotApp {
    inner: Pro800App,
    path: PathBuf,
    frames: u64,
    requested: bool,
}

const SHOT_FRAME: u64 = 120;

impl ShotApp {
    pub fn new(cc: &eframe::CreationContext<'_>, path: PathBuf) -> Self {
        let mut inner = Pro800App::new(cc);
        // The screenshot path runs without hardware: show captured programs and
        // a captured global block so the picture documents the real surface.
        let hardware = inner.connected();
        for fixture in [
            &include_bytes!("../../../verified/pro-800-a000.syx")[..],
            &include_bytes!("../../../verified/pro-800-b094.syx")[..],
            &include_bytes!("../../../verified/pro-800-c054.syx")[..],
        ] {
            for patch in patch::patches_from_syx(fixture) {
                if patch.index == 0 && !hardware {
                    inner.load_patch(patch.clone());
                }
                inner.lib.put(patch);
            }
        }
        if !hardware
            && let Some(block) =
                patch::global_reply(include_bytes!("../../../verified/pro-800-globals.syx"))
        {
            inner.globals = Some(block);
        }
        inner.tab = match std::env::var("PRO800_TAB").as_deref() {
            Ok("library") => Tab::Library,
            Ok("global") => Tab::Global,
            Ok("console") => Tab::Console,
            _ => Tab::Patch,
        };
        Self {
            inner,
            path,
            frames: 0,
            requested: false,
        }
    }
}

impl eframe::App for ShotApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.inner.ui(ui, frame);
        self.frames += 1;
        if !self.requested && self.frames >= SHOT_FRAME {
            self.requested = true;
            let ctx = ui.ctx().clone();
            let path = self.path.clone();
            let closer = ctx.clone();
            ctx.request_screenshot(move |image| {
                match save_png(&path, &image) {
                    Ok(()) => eprintln!("screenshot saved to {}", path.display()),
                    Err(err) => eprintln!("screenshot: {err}"),
                }
                closer.send_viewport_cmd(egui::ViewportCommand::Close);
            });
        }
        ui.ctx().request_repaint();
    }
}

fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), String> {
    let [width, height] = image.size;
    if width == 0 || height == 0 {
        return Err("empty image".to_string());
    }
    let mut pixels = Vec::with_capacity(width * height * 4);
    for pixel in &image.pixels {
        let [r, g, b, a] = pixel.to_array();
        pixels.extend_from_slice(&[r, g, b, a]);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    image::save_buffer(
        path,
        &pixels,
        width as u32,
        height as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|err| format!("{}: {err}", path.display()))
}
