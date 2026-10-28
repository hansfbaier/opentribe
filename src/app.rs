//! egui front end for the POLY D global-parameter SysEx map.

use std::collections::VecDeque;
use std::time::Instant;

use eframe::egui;
use egui::{Color32, RichText};

use crate::midi::{self, MidiHub};
use crate::model::{
    self, CLOCK_POLARITY_LABELS, CLOCK_RATE_LABELS, CLOCK_SOURCE_LABELS, GlobalParams,
    KEY_PRIORITY_LABELS, MOD_CURVE_LABELS, OUTPUT_LABELS, SPECS, VELOCITY_CURVE_LABELS,
};
use crate::sysex::{self, Framing, packet};

const HEADING: Color32 = Color32::from_rgb(0x8A, 0xB4, 0xF8);
const OK: Color32 = Color32::from_rgb(0x6F, 0xD0, 0x89);
const WARN: Color32 = Color32::from_rgb(0xE8, 0xB3, 0x4A);
const DANGER: Color32 = Color32::from_rgb(0xE5, 0x7A, 0x7A);
const MUTED: Color32 = Color32::from_gray(0x8C);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Global,
    Routing,
    Test,
    Console,
    Reference,
}

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::Global => "Global",
            Tab::Routing => "Clock & routing",
            Tab::Test => "Play & test",
            Tab::Console => "Console",
            Tab::Reference => "Reference",
        }
    }

    const ALL: [Tab; 5] = [
        Tab::Global,
        Tab::Routing,
        Tab::Test,
        Tab::Console,
        Tab::Reference,
    ];
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Tx,
    Rx,
    Info,
    Error,
}

impl Dir {
    fn tag(self) -> &'static str {
        match self {
            Dir::Tx => "TX",
            Dir::Rx => "RX",
            Dir::Info => "info",
            Dir::Error => "err",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Dir::Tx => Color32::from_rgb(0x79, 0xB8, 0xFF),
            Dir::Rx => Color32::from_rgb(0x7F, 0xD7, 0x8A),
            Dir::Info => Color32::from_gray(0x9A),
            Dir::Error => DANGER,
        }
    }
}

struct Entry {
    dir: Dir,
    label: String,
    hex: String,
    at: f64,
}

struct Log {
    entries: VecDeque<Entry>,
    start: Instant,
    show_rx: bool,
    show_info: bool,
}

impl Log {
    fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            start: Instant::now(),
            show_rx: true,
            show_info: true,
        }
    }

    fn push(&mut self, dir: Dir, label: impl Into<String>, bytes: &[u8]) {
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

    fn info(&mut self, label: impl Into<String>) {
        self.push(Dir::Info, label, &[]);
    }

    fn error(&mut self, label: impl Into<String>) {
        self.push(Dir::Error, label, &[]);
    }

    fn visible(&self) -> Vec<(Dir, String)> {
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

struct OutMsg {
    label: String,
    bytes: Vec<u8>,
}

pub struct PolyDApp {
    hub: MidiHub,
    out_ports: Vec<String>,
    in_ports: Vec<String>,
    selected_out: Option<String>,
    selected_in: Option<String>,
    params: GlobalParams,
    framing: Framing,
    device_id: u8,
    auto_apply: bool,
    dirty: bool,
    last_sent: Vec<(u8, Vec<u8>)>,
    queue: Vec<OutMsg>,
    log: Log,
    tab: Tab,
    hex_input: String,
    hex_error: Option<String>,
    held_note: Option<u8>,
    bend: i16,
    modwheel: u8,
    confirm_restore: bool,
    settings_loaded: bool,
}

impl PolyDApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self {
            hub: MidiHub::default(),
            out_ports: Vec::new(),
            in_ports: Vec::new(),
            selected_out: None,
            selected_in: None,
            params: GlobalParams::default(),
            framing: Framing::Qsg,
            device_id: sysex::DEFAULT_DEVICE_ID,
            auto_apply: true,
            dirty: false,
            last_sent: Vec::new(),
            queue: Vec::new(),
            log: Log::new(),
            tab: Tab::Global,
            hex_input: String::new(),
            hex_error: None,
            held_note: None,
            bend: 0,
            modwheel: 0,
            confirm_restore: false,
            settings_loaded: false,
        };

        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.all_styles_mut(|style| {
            style.spacing.slider_width = 190.0;
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(9.0, 4.0);
        });

        app.refresh_ports();
        app.auto_connect(&cc.egui_ctx);
        app.log.info(format!(
            "POLY D model {:02X} {:02X} {:02X}, protocol family {FAMILY_LABEL}",
            sysex::MODEL[0],
            sysex::MODEL[1],
            sysex::MODEL[2],
            FAMILY_LABEL = sysex::FAMILY
        ));
        app
    }

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
        if let Some(selected) = &self.selected_out
            && !self.out_ports.contains(selected)
        {
            self.selected_out = None;
        }
        if self.selected_out.is_none() {
            self.selected_out = midi::guess_poly_d_port(&self.out_ports);
        }
        if let Some(selected) = &self.selected_in
            && !self.in_ports.contains(selected)
        {
            self.selected_in = None;
        }
        if self.selected_in.is_none() {
            self.selected_in = midi::guess_poly_d_port(&self.in_ports);
        }
    }

    fn auto_connect(&mut self, ctx: &egui::Context) {
        let Some(name) = self.selected_out.clone() else {
            self.log.info(format!(
                "no POLY D output found ({} MIDI outputs visible)",
                self.out_ports.len()
            ));
            return;
        };
        if let Err(err) = self.hub.connect_output(&name) {
            self.log.error(format!("could not open {name:?}: {err}"));
            return;
        }
        self.log.info(format!("output opened: {name}"));
        if self.selected_in.clone().as_deref() == Some(name.as_str()) {
            self.connect_input(ctx);
        }
        self.read_settings();
    }

    fn connect_output(&mut self) {
        let Some(name) = self.selected_out.clone() else {
            self.log.error("no output port selected");
            return;
        };
        match self.hub.connect_output(&name) {
            Ok(()) => {
                self.last_sent.clear();
                self.log.info(format!("output opened: {name}"));
                self.read_settings();
            }
            Err(err) => self.log.error(format!("{name}: {err}")),
        }
    }

    fn connect_input(&mut self, ctx: &egui::Context) {
        let Some(name) = self.selected_in.clone() else {
            self.log.error("no input port selected");
            return;
        };
        let ctx = ctx.clone();
        match self.hub.connect_input(&name, move || ctx.request_repaint()) {
            Ok(()) => self.log.info(format!("input monitoring: {name}")),
            Err(err) => self.log.error(format!("{name}: {err}")),
        }
    }

    fn send_all(&mut self) {
        self.last_sent.clear();
        for (name, packet, payload) in self.params.payloads() {
            self.remember(packet, &payload);
            self.queue.push(OutMsg {
                label: format!("{name} ({packet:02X})"),
                bytes: sysex::param(self.framing, self.device_id, packet, &payload),
            });
        }
        self.dirty = false;
    }

    fn was_sent(&self, packet: u8, payload: &[u8]) -> bool {
        self.last_sent
            .iter()
            .any(|(last_packet, last_payload)| *last_packet == packet && last_payload == payload)
    }

    fn remember(&mut self, packet: u8, payload: &[u8]) {
        self.last_sent.retain(|(last, _)| *last != packet);
        self.last_sent.push((packet, payload.to_vec()));
    }

    /// Queue one parameter write, honouring the auto-apply switch and
    /// suppressing repeats of a value that was already sent.
    fn write_param(&mut self, packet: u8, payload: &[u8], label: &str) {
        if !self.auto_apply {
            self.dirty = true;
            return;
        }
        if self.was_sent(packet, payload) {
            return;
        }
        self.remember(packet, payload);
        self.queue.push(OutMsg {
            label: format!("{label} ({packet:02X})"),
            bytes: sysex::param(self.framing, self.device_id, packet, payload),
        });
    }

    fn send_message(&mut self, label: &str, bytes: Vec<u8>) {
        self.queue.push(OutMsg {
            label: label.to_string(),
            bytes,
        });
    }

    fn flush(&mut self) {
        for out in std::mem::take(&mut self.queue) {
            if !self.hub.output_open() {
                self.log
                    .push(Dir::Info, format!("dry run · {}", out.label), &out.bytes);
                continue;
            }
            match self.hub.send(&out.bytes) {
                Ok(()) => self.log.push(Dir::Tx, out.label, &out.bytes),
                Err(err) => self.log.error(format!("{}: {err}", out.label)),
            }
        }
    }

    fn poll(&mut self) {
        for bytes in self.hub.poll() {
            let (dir, label) = self.classify(&bytes);
            self.log.push(dir, label, &bytes);
        }
    }

    /// Turn one received message into a log severity and a label. A `0x76`
    /// settings block is applied to the UI, a rejected ACK is logged as an
    /// error so failed writes do not look successful.
    fn classify(&mut self, bytes: &[u8]) -> (Dir, String) {
        let Some(message) = sysex::parse(bytes) else {
            return match bytes.first() {
                Some(0xF0) => (Dir::Error, "SysEx (not a POLY D frame)".to_string()),
                Some(status) => (
                    Dir::Rx,
                    format!("MIDI channel message (ch {})", (status & 0x0F) + 1),
                ),
                None => (Dir::Info, "empty message".to_string()),
            };
        };

        let prefix = [sysex::op::SETTINGS_BLOCK];
        if let Some(block) = message.body.strip_prefix(&prefix) {
            match GlobalParams::from_block(block) {
                Ok(params) => {
                    let summary = format!(
                        "TX {} / RX {} · transpose {} · bend {} · local KB {}",
                        params.tx_channel,
                        params.rx_channel,
                        params.transpose,
                        params.pitch_bend,
                        if params.local_keyboard { "on" } else { "off" }
                    );
                    self.params = params;
                    self.last_sent.clear();
                    self.settings_loaded = true;
                    return (
                        Dir::Rx,
                        format!(
                            "DID {:02X} · settings block {} bytes · {summary}",
                            message.device_id,
                            block.len()
                        ),
                    );
                }
                Err(err) => {
                    return (Dir::Error, format!("settings block: {err}"));
                }
            }
        }

        let rejected = matches!(model::ack_code(message.body), Some(code) if code != 0);
        let label = format!(
            "DID {:02X} · {}",
            message.device_id,
            model::decode(message.body).unwrap_or_else(|| "unknown body".to_string())
        );
        if rejected {
            (Dir::Error, label)
        } else {
            (Dir::Rx, label)
        }
    }

    fn read_settings(&mut self) {
        let bytes = sysex::read_entire(self.device_id);
        self.send_message("read settings · 75", bytes);
    }

    fn send_channel(&mut self) {
        let payload = [0x01, self.params.tx_channel, self.params.rx_channel];
        self.write_param(packet::CHANNEL, &payload, "MIDI channel");
    }

    fn send_velocity(&mut self) {
        let payload = [
            self.params.velocity_on,
            self.params.velocity_off,
            self.params.velocity_curve,
        ];
        self.write_param(packet::VELOCITY, &payload, "Velocity");
    }

    // ---------------------------------------------------------------- rows

    #[allow(clippy::too_many_arguments)]
    fn row_u8(
        &mut self,
        ui: &mut egui::Ui,
        name: &'static str,
        range: std::ops::RangeInclusive<u8>,
        value: u8,
        hint: &str,
        packet: u8,
        padded: bool,
    ) -> u8 {
        ui.label(name);
        let mut value_now = value;
        let response = ui.add(egui::Slider::new(&mut value_now, range));
        hint_cell(ui, hint);
        ui.end_row();
        if response.changed() {
            let payload = if padded {
                vec![value_now, 0x00]
            } else {
                vec![value_now]
            };
            self.write_param(packet, &payload, name);
        }
        value_now
    }

    fn row_bool(
        &mut self,
        ui: &mut egui::Ui,
        name: &'static str,
        value: bool,
        hint: &str,
        packet: u8,
        padded: bool,
    ) -> bool {
        ui.label(name);
        let mut value_now = value;
        let checkbox_label = if value_now { "on" } else { "off" };
        let response = ui.checkbox(&mut value_now, checkbox_label);
        hint_cell(ui, hint);
        ui.end_row();
        if response.changed() {
            let byte = u8::from(value_now);
            let payload = if padded { vec![byte, 0x00] } else { vec![byte] };
            self.write_param(packet, &payload, name);
        }
        value_now
    }

    fn row_enum(
        &mut self,
        ui: &mut egui::Ui,
        name: &'static str,
        value: u8,
        labels: &[&str],
        hint: &str,
        packet: u8,
    ) -> u8 {
        ui.label(name);
        let mut picked = None;
        egui::ComboBox::from_id_salt(name)
            .width(150.0)
            .selected_text(
                labels
                    .get(value as usize)
                    .copied()
                    .unwrap_or("unknown value"),
            )
            .show_ui(ui, |ui| {
                for (index, label) in labels.iter().enumerate() {
                    if ui
                        .selectable_label(value as usize == index, *label)
                        .clicked()
                    {
                        picked = Some(index as u8);
                    }
                }
            });
        hint_cell(ui, hint);
        ui.end_row();
        if let Some(byte) = picked {
            self.write_param(packet, &[byte], name);
            return byte;
        }
        value
    }

    // ------------------------------------------------------------- panels

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("POLY D").size(19.0).strong());
            ui.label(RichText::new("global SysEx controller").weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.checkbox(&mut self.auto_apply, "auto-apply").changed()
                    && self.auto_apply
                    && self.dirty
                {
                    self.send_all();
                }
                if ui
                    .button("Send all")
                    .on_hover_text("Write all 21 global parameters")
                    .clicked()
                {
                    self.send_all();
                }
                if ui
                    .button("Read")
                    .on_hover_text("Read all global settings (ReadEntireParameters, 0x75)")
                    .clicked()
                {
                    self.read_settings();
                }
                if self.dirty && !self.auto_apply {
                    ui.label(RichText::new("pending changes").color(WARN));
                }
                if self.settings_loaded {
                    ui.label(RichText::new("settings read").color(OK));
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label("Output");
            let ports = self.out_ports.clone();
            egui::ComboBox::from_id_salt("out_port")
                .width(300.0)
                .truncate()
                .selected_text(
                    self.selected_out
                        .clone()
                        .map(|name| short_port(&name))
                        .unwrap_or_else(|| "— select a port —".to_string()),
                )
                .show_ui(ui, |ui| {
                    if ports.is_empty() {
                        ui.weak("no MIDI outputs visible");
                    }
                    for name in &ports {
                        let selected = self.selected_out.as_deref() == Some(name.as_str());
                        if ui.selectable_label(selected, name).clicked() {
                            self.selected_out = Some(name.clone());
                        }
                    }
                });
            if ui.button("Rescan").clicked() {
                self.refresh_ports();
            }
            if self.hub.output_open() {
                if ui.button("Disconnect").clicked() {
                    self.hub.disconnect_output();
                    self.log.info("output closed");
                }
            } else if ui
                .add_enabled(self.selected_out.is_some(), egui::Button::new("Connect"))
                .clicked()
            {
                self.connect_output();
            }

            ui.separator();
            ui.label("Monitor");
            let ports = self.in_ports.clone();
            egui::ComboBox::from_id_salt("in_port")
                .width(260.0)
                .truncate()
                .selected_text(
                    self.selected_in
                        .clone()
                        .map(|name| short_port(&name))
                        .unwrap_or_else(|| "— none —".to_string()),
                )
                .show_ui(ui, |ui| {
                    if ports.is_empty() {
                        ui.weak("no MIDI inputs visible");
                    }
                    for name in &ports {
                        let selected = self.selected_in.as_deref() == Some(name.as_str());
                        if ui.selectable_label(selected, name).clicked() {
                            self.selected_in = Some(name.clone());
                        }
                    }
                });
            if self.hub.input_name().is_some() {
                if ui.button("Stop").clicked() {
                    self.hub.disconnect_input();
                    self.log.info("input monitoring stopped");
                }
            } else if ui
                .add_enabled(self.selected_in.is_some(), egui::Button::new("Start"))
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.connect_input(&ctx);
            }
        });

        ui.horizontal(|ui| {
            match self.hub.output_name() {
                Some(name) => {
                    status_dot(ui, OK);
                    ui.label(format!("out: {}", short_port(name)));
                }
                None => {
                    status_dot(ui, MUTED);
                    ui.label(RichText::new("out: not connected").color(MUTED));
                }
            }
            if let Some(name) = self.hub.input_name() {
                ui.separator();
                status_dot(ui, OK);
                ui.label(format!("in: {}", short_port(name)));
            }
            ui.separator();
            ui.label("Device ID");
            let mut device_id = self.device_id;
            if ui
                .add(egui::DragValue::new(&mut device_id).range(0..=127))
                .changed()
            {
                self.device_id = device_id;
                self.last_sent.clear();
            }
            ui.separator();
            ui.label("Framing");
            let mut framing = self.framing;
            egui::ComboBox::from_id_salt("framing")
                .width(250.0)
                .truncate()
                .selected_text(framing.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut framing, Framing::Qsg, Framing::Qsg.label());
                    ui.selectable_value(
                        &mut framing,
                        Framing::SynthTribeLong,
                        format!("{} — rejected by 1.1.3", Framing::SynthTribeLong.label()),
                    );
                });
            if framing != self.framing {
                self.framing = framing;
                self.last_sent.clear();
            }
        });
        ui.add_space(4.0);
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Log");
            ui.weak(format!("{} entries", self.log.entries.len()));
            if ui.button("Clear").clicked() {
                self.log.entries.clear();
            }
            ui.checkbox(&mut self.log.show_rx, "RX");
            ui.checkbox(&mut self.log.show_info, "info");
            ui.weak("dry run = message built but no output port is open");
        });
        let lines = self.log.visible();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (dir, line) in lines {
                    ui.label(
                        RichText::new(line)
                            .monospace()
                            .size(11.5)
                            .color(dir.color()),
                    );
                }
            });
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for tab in Tab::ALL {
                if ui.selectable_label(self.tab == tab, tab.title()).clicked() {
                    self.tab = tab;
                }
            }
        });
        ui.separator();
        match self.tab {
            Tab::Global => self.global_tab(ui),
            Tab::Routing => self.routing_tab(ui),
            Tab::Test => self.test_tab(ui),
            Tab::Console => self.console_tab(ui),
            Tab::Reference => self.reference_tab(ui),
        }
    }

    fn global_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section(ui, "Channel & keyboard");
                egui::Grid::new("grid_channel")
                    .num_columns(3)
                    .spacing([12.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("MIDI channel TX");
                        let mut tx = self.params.tx_channel;
                        let response = ui.add(egui::Slider::new(&mut tx, 0..=16));
                        hint_cell(ui, "0 = any channel (QSG range 0-16)");
                        ui.end_row();
                        if response.changed() {
                            self.params.tx_channel = tx;
                            self.send_channel();
                        }

                        ui.label("MIDI channel RX");
                        let mut rx = self.params.rx_channel;
                        let response = ui.add(egui::Slider::new(&mut rx, 0..=16));
                        hint_cell(ui, "0 = All (any channel)");
                        ui.end_row();
                        if response.changed() {
                            self.params.rx_channel = rx;
                            self.send_channel();
                        }

                        self.params.transpose = self.row_u8(
                            ui,
                            "MIDI IN transpose",
                            0..=24,
                            self.params.transpose,
                            "wire value: 12 = none, 0 = -12, 24 = +12",
                            packet::TRANSPOSE,
                            false,
                        );
                        self.params.pitch_bend = self.row_u8(
                            ui,
                            "Pitch bend range",
                            0..=24,
                            self.params.pitch_bend,
                            "semitones (0-24)",
                            packet::PITCH_BEND,
                            true,
                        );
                        self.params.key_priority = self.row_enum(
                            ui,
                            "Key priority",
                            self.params.key_priority,
                            KEY_PRIORITY_LABELS,
                            "note-stealing order",
                            packet::KEY_PRIORITY,
                        );
                        self.params.multi_trigger = self.row_bool(
                            ui,
                            "Multi-trigger",
                            self.params.multi_trigger,
                            "retrigger envelopes for every new note",
                            packet::MULTI_TRIGGER,
                            true,
                        );
                        self.params.local_keyboard = self.row_bool(
                            ui,
                            "Local keyboard mode",
                            self.params.local_keyboard,
                            "packet 0x2F, write verified on firmware 1.1.3",
                            packet::LOCAL_KEYBOARD,
                            false,
                        );
                    });

                section(ui, "Velocity");
                egui::Grid::new("grid_velocity")
                    .num_columns(3)
                    .spacing([12.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Note-on velocity");
                        let mut on = self.params.velocity_on;
                        let response = ui.add(egui::Slider::new(&mut on, 0..=127));
                        hint_cell(ui, "0 = take velocity from the key");
                        ui.end_row();
                        if response.changed() {
                            self.params.velocity_on = on;
                            self.send_velocity();
                        }

                        ui.label("Note-off velocity");
                        let mut off = self.params.velocity_off;
                        let response = ui.add(egui::Slider::new(&mut off, 0..=127));
                        hint_cell(ui, "0 = dynamic");
                        ui.end_row();
                        if response.changed() {
                            self.params.velocity_off = off;
                            self.send_velocity();
                        }

                        ui.label("Velocity curve");
                        let mut curve = self.params.velocity_curve;
                        let mut picked = None;
                        egui::ComboBox::from_id_salt("velocity_curve")
                            .width(150.0)
                            .selected_text(
                                VELOCITY_CURVE_LABELS
                                    .get(curve as usize)
                                    .copied()
                                    .unwrap_or("?"),
                            )
                            .show_ui(ui, |ui| {
                                for (index, label) in VELOCITY_CURVE_LABELS.iter().enumerate() {
                                    if ui
                                        .selectable_label(curve as usize == index, *label)
                                        .clicked()
                                    {
                                        picked = Some(index as u8);
                                    }
                                }
                            });
                        hint_cell(ui, "curve number 0-2");
                        ui.end_row();
                        if let Some(value) = picked {
                            curve = value;
                            self.params.velocity_curve = curve;
                            self.send_velocity();
                        }
                    });

                section(ui, "Modulation & CV");
                egui::Grid::new("grid_mod")
                    .num_columns(3)
                    .spacing([12.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        self.params.mod_curve = self.row_enum(
                            ui,
                            "Modulation curve",
                            self.params.mod_curve,
                            MOD_CURVE_LABELS,
                            "wheel response",
                            packet::MOD_CURVE,
                        );
                        self.params.mod_wheel_range = self.row_u8(
                            ui,
                            "Modulation wheel range",
                            0..=4,
                            self.params.mod_wheel_range,
                            "QSG: 2 = 100 %",
                            packet::MOD_WHEEL_RANGE,
                            false,
                        );
                        self.params.note_zero_cv = self.row_u8(
                            ui,
                            "Note at zero CV",
                            0..=127,
                            self.params.note_zero_cv,
                            "MIDI note that produces 0 V (36 = C2)",
                            packet::NOTE_ZERO_CV,
                            false,
                        );
                        self.params.accent_threshold = self.row_u8(
                            ui,
                            "Accent velocity threshold",
                            0..=127,
                            self.params.accent_threshold,
                            "SynthTribe only; packet 0x1C is not in the QSG",
                            packet::ACCENT_THRESHOLD,
                            false,
                        );
                    });
                ui.add_space(6.0);
            });
    }

    fn routing_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section(ui, "Clock");
                egui::Grid::new("grid_clock")
                    .num_columns(3)
                    .spacing([12.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        self.params.clock_source = self.row_enum(
                            ui,
                            "Clock source",
                            self.params.clock_source,
                            CLOCK_SOURCE_LABELS,
                            "packet 0x1B",
                            packet::CLOCK_SOURCE,
                        );
                        self.params.clock_rate = self.row_enum(
                            ui,
                            "Clock rate",
                            self.params.clock_rate,
                            CLOCK_RATE_LABELS,
                            "packet 0x1A",
                            packet::CLOCK_RATE,
                        );
                        self.params.clock_polarity = self.row_enum(
                            ui,
                            "External clock polarity",
                            self.params.clock_polarity,
                            CLOCK_POLARITY_LABELS,
                            "packet 0x19",
                            packet::CLOCK_POLARITY,
                        );
                        self.params.clock_out = self.row_enum(
                            ui,
                            "MIDI clock out",
                            self.params.clock_out,
                            OUTPUT_LABELS,
                            "packet 0x17",
                            packet::CLOCK_OUT,
                        );
                    });

                section(ui, "MIDI output routing");
                ui.weak("Each row is its own packet: 0 off, 1 DIN, 2 USB, 3 both.");
                ui.add_space(2.0);
                egui::Grid::new("grid_routing")
                    .num_columns(3)
                    .spacing([12.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        self.params.out_mod_wheel = self.row_enum(
                            ui,
                            "Modulation wheel",
                            self.params.out_mod_wheel,
                            OUTPUT_LABELS,
                            "0x21",
                            packet::OUT_MOD_WHEEL,
                        );
                        self.params.out_pitch_wheel = self.row_enum(
                            ui,
                            "Pitch wheel",
                            self.params.out_pitch_wheel,
                            OUTPUT_LABELS,
                            "0x22",
                            packet::OUT_PITCH_WHEEL,
                        );
                        self.params.out_keyboard = self.row_enum(
                            ui,
                            "Keyboard",
                            self.params.out_keyboard,
                            OUTPUT_LABELS,
                            "0x23",
                            packet::OUT_KEYBOARD,
                        );
                        self.params.out_aftertouch = self.row_enum(
                            ui,
                            "Keyboard aftertouch",
                            self.params.out_aftertouch,
                            OUTPUT_LABELS,
                            "0x24",
                            packet::OUT_AFTERTOUCH,
                        );
                        self.params.out_sequencer = self.row_enum(
                            ui,
                            "Sequencer",
                            self.params.out_sequencer,
                            OUTPUT_LABELS,
                            "0x25",
                            packet::OUT_SEQUENCER,
                        );
                        self.params.out_arp = self.row_enum(
                            ui,
                            "Arpeggiator",
                            self.params.out_arp,
                            OUTPUT_LABELS,
                            "0x26",
                            packet::OUT_ARP,
                        );
                    });
            });
    }

    fn test_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section(ui, "Play");
                ui.weak(format!(
                    "Note, bend and wheel messages go straight to MIDI channel {} (MIDI channel TX).",
                    self.params.tx_channel.clamp(1, 16)
                ));
                ui.add_space(4.0);
                let hit = keyboard(ui, self.held_note, 48, 2);
                let pointer_down = ui.input(|i| i.pointer.primary_down());
                self.handle_keyboard(hit, pointer_down);

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("Pitch bend");
                    let response = ui.add(
                        egui::Slider::new(&mut self.bend, -8192..=8191).show_value(false),
                    );
                    ui.weak(format!("{}", self.bend));
                    if response.changed() {
                        self.send_bend();
                    }
                    if ui.button("Centre").clicked() {
                        self.bend = 0;
                        self.send_bend();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Mod wheel");
                    let response = ui.add(egui::Slider::new(&mut self.modwheel, 0..=127));
                    if response.changed() {
                        self.send_modwheel();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("All notes off").clicked() {
                        self.send_all_notes_off();
                    }
                });

                ui.separator();
                section(ui, "Maintenance");
                if ui
                    .button(RichText::new("Restore factory settings").color(DANGER))
                    .on_hover_text("Sends packet 0x7D to the POLY D")
                    .clicked()
                {
                    self.confirm_restore = true;
                }
                ui.weak("Overwrites every stored setting on the instrument.");
            });
        self.restore_modal(ui.ctx());
    }

    fn console_tab(&mut self, ui: &mut egui::Ui) {
        section(ui, "Read requests");
        ui.weak(
            "Verified on firmware 1.1.3: 0x75 returns the 25-byte settings block, 0x02/0x08 \
             return mode and firmware. The 0x77 reads used by SynthTribe are rejected (ACK 5).",
        );
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Read settings · 75").clicked() {
                self.read_settings();
            }
            if ui.button("Query mode · 02").clicked() {
                let bytes = sysex::query_mode(self.device_id);
                self.send_message("query mode", bytes);
            }
            if ui.button("Firmware · 08").clicked() {
                let bytes = sysex::firmware_version(self.device_id);
                self.send_message("firmware version", bytes);
            }
        });

        ui.separator();
        section(ui, "Send raw SysEx");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.hex_input)
                    .desired_width(460.0)
                    .hint_text("F0 00 20 32 00 01 0C 00 2F 01 F7   or body only: 2F 01"),
            );
            if ui.button("Send").clicked() {
                self.send_hex();
            }
        });
        ui.weak(
            "Hex pairs; spaces, commas, colons and 0x prefixes are ignored. \
             A body without F0 is wrapped in the POLY D envelope, a missing F7 is appended.",
        );
        if let Some(error) = &self.hex_error {
            ui.colored_label(DANGER, error);
        }

        ui.separator();
        section(ui, "Quick write");
        ui.horizontal_wrapped(|ui| {
            for (label, value) in [("Local KB on", 1u8), ("Local KB off", 0u8)] {
                if ui.button(label).clicked() {
                    let bytes = sysex::param_u8(
                        self.framing,
                        self.device_id,
                        packet::LOCAL_KEYBOARD,
                        value,
                    );
                    self.send_message(label, bytes);
                }
            }
            if ui.button("Reset transpose to none (12)").clicked() {
                let bytes = sysex::param_u8(self.framing, self.device_id, packet::TRANSPOSE, 12);
                self.send_message("transpose = 12", bytes);
            }
        });
    }

    fn reference_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section(ui, "Global parameter packets");
                egui::ScrollArea::horizontal()
                    .id_salt("specs_scroll")
                    .show(ui, |ui| {
                        egui::Grid::new("grid_specs")
                            .num_columns(7)
                            .spacing([10.0, 3.0])
                            .striped(true)
                            .show(ui, |ui| {
                                for header in [
                                    "parameter", "pkt", "payload", "range", "default", "blk",
                                    "source",
                                ] {
                                    ui.label(RichText::new(header).strong().size(11.5));
                                }
                                ui.end_row();
                                for spec in SPECS {
                                    let size = 11.5;
                                    ui.label(RichText::new(spec.name).size(size));
                                    ui.label(RichText::new(format!("{:02X}", spec.packet)).monospace().size(size));
                                    ui.label(RichText::new(spec.wire).monospace().size(size));
                                    ui.label(RichText::new(spec.range).weak().size(size));
                                    ui.label(RichText::new(spec.default).weak().size(size));
                                    let index = spec
                                        .index
                                        .map(|index| index.to_string())
                                        .unwrap_or_else(|| "-".to_string());
                                    ui.label(RichText::new(index).monospace().size(size));
                                    let color = match spec.source {
                                        model::Evidence::Verified => OK,
                                        model::Evidence::Qsg => MUTED,
                                        model::Evidence::Unsupported => WARN,
                                    };
                                    ui.label(RichText::new(spec.source.tag()).color(color).size(size));
                                    ui.end_row();
                                }
                            });
                    });

                section(ui, "Envelope in use");
                let example =
                    sysex::param_u8(self.framing, self.device_id, packet::LOCAL_KEYBOARD, 1);
                ui.monospace(sysex::hex(&example));
                ui.weak(format!(
                    "Device ID {:02X} · framing {} · model 00 01 0C · family {}",
                    self.device_id,
                    self.framing.label(),
                    sysex::FAMILY
                ));

                section(ui, "Verified on hardware (firmware 1.1.3)");
                for line in [
                    "Framing: the QSG form F0 00 20 32 00 01 0C <DID> <packet> <payload> F7 is accepted. \
                     The SynthTribe 0x78 long-envelope form is rejected with ACK code 5, so it stays an \
                     experiment only.",
                    "Every write is answered with an ACK: 01 00 00 = accepted, 01 00 05 = rejected \
                     (unknown packet, out-of-range value, or wrong framing). Rejections are logged in red.",
                    "Read-back: 0x75 (ReadEntireParameters) returns 0x76 plus a 25-byte block. The block \
                     offsets are the config.json index fields and were confirmed one packet at a time; \
                     they are listed in the block column above.",
                    "Local keyboard mode (0x2F) is real on this firmware: 2F 01 / 2F 00 flip block offset \
                     15, and 2F 02 is rejected. The packet is absent from the QSG.",
                    "Accent velocity threshold (0x1C) is real: 1C 64 moves block offset 17. \
                     Sequencer auto-sync (0x1D) and Assign (0x1F) are rejected by this firmware, \
                     so the app does not send them.",
                    "The 0x77 reads used by SynthTribe are rejected as well; 0x75 is the working read.",
                ] {
                    ui.label(format!("• {line}"));
                }

                section(ui, "Limits");
                for line in [
                    "There is no MIDI CC parameter map for the POLY D; the published control surface is SysEx only.",
                    "Sequencer and arpeggiator pattern editing are out of scope. Only their MIDI output routing is covered.",
                    "Block offsets 0, 5 and 6 are not exposed: 0 read as zero on 1.1.3 and 5/6 mirror the velocity constants.",
                ] {
                    ui.label(format!("• {line}"));
                }
            });
    }

    // --------------------------------------------------------- performance

    fn handle_keyboard(&mut self, hit: Option<u8>, pointer_down: bool) {
        if pointer_down {
            match (self.held_note, hit) {
                (_, Some(note)) if self.held_note != Some(note) => {
                    if let Some(previous) = self.held_note.take() {
                        self.send_note(previous, false);
                    }
                    self.held_note = Some(note);
                    self.send_note(note, true);
                }
                (Some(previous), None) => {
                    self.held_note = None;
                    self.send_note(previous, false);
                }
                _ => {}
            }
        } else if let Some(previous) = self.held_note.take() {
            self.send_note(previous, false);
        }
    }

    fn channel_nibble(&self) -> u8 {
        self.params.tx_channel.clamp(1, 16) - 1
    }

    fn send_note(&mut self, note: u8, on: bool) {
        let status = if on { 0x90 } else { 0x80 } | self.channel_nibble();
        let velocity = if on { 100 } else { 64 };
        self.send_performance("note", vec![status, note & 0x7F, velocity]);
    }

    fn send_bend(&mut self) {
        let value = (i32::from(self.bend) + 8192).clamp(0, 16383) as u16;
        let status = 0xE0 | self.channel_nibble();
        let bytes = vec![status, (value & 0x7F) as u8, (value >> 7) as u8];
        self.send_performance("pitch bend", bytes);
    }

    fn send_modwheel(&mut self) {
        let status = 0xB0 | self.channel_nibble();
        self.send_performance("mod wheel", vec![status, 1, self.modwheel & 0x7F]);
    }

    fn send_all_notes_off(&mut self) {
        let status = 0xB0 | self.channel_nibble();
        self.send_performance("all notes off", vec![status, 123, 0]);
        self.held_note = None;
    }

    /// Performance traffic is sent immediately and is not logged by default;
    /// otherwise playing the on-screen keyboard would flood the console.
    fn send_performance(&mut self, label: &str, bytes: Vec<u8>) {
        if !self.hub.output_open() {
            return;
        }
        if let Err(err) = self.hub.send(&bytes) {
            self.log.error(format!("{label}: {err}"));
        }
    }

    fn send_hex(&mut self) {
        match sysex::parse_hex(&self.hex_input) {
            Ok(bytes) if bytes.is_empty() => {
                self.hex_error = Some("no bytes to send".to_string());
            }
            Ok(mut bytes) => {
                self.hex_error = None;
                if bytes[0] == 0xF0 {
                    if bytes.last() != Some(&0xF7) {
                        bytes.push(0xF7);
                    }
                } else {
                    bytes = sysex::envelope(self.device_id, &bytes);
                }
                self.send_message("manual", bytes);
            }
            Err(err) => self.hex_error = Some(err),
        }
    }

    fn restore_modal(&mut self, ctx: &egui::Context) {
        if !self.confirm_restore {
            return;
        }
        let mut open = true;
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Window::new("Restore factory settings?")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    "This sends packet 0x7D to the POLY D and overwrites every stored setting.",
                );
                ui.label(RichText::new("There is no undo.").color(DANGER));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Restore").clicked() {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            });
        if confirmed {
            let bytes = sysex::restore_factory(self.device_id);
            self.send_message("restore factory settings", bytes);
            self.confirm_restore = false;
        } else if cancelled || !open {
            self.confirm_restore = false;
        }
    }
}

impl eframe::App for PolyDApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        egui::Panel::top("top_bar").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("log_panel")
            .resizable(true)
            .default_size(170.0)
            .min_size(70.0)
            .show(ui, |ui| self.log_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| self.tabs(ui));
        self.flush();
    }
}

// ------------------------------------------------------------------ helpers

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).strong().color(HEADING));
    ui.add_space(2.0);
}

/// Small filled circle; the default egui font has no `●` glyph.
fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

fn hint_cell(ui: &mut egui::Ui, hint: &str) {
    if hint.is_empty() {
        ui.label("");
    } else {
        ui.weak(hint);
    }
}

/// Shorten an ALSA/CoreMIDI port name for status labels, e.g.
/// `POLY D:POLY D MIDI 1 16:0` becomes `POLY D MIDI 1`.
fn short_port(name: &str) -> String {
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

fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = i16::from(note) / 12 - 1;
    format!("{}{}", NAMES[(note % 12) as usize], octave)
}

/// Draw a two-octave keyboard. Returns the note under the pointer while the
/// left button is held inside the widget.
fn keyboard(ui: &mut egui::Ui, held: Option<u8>, base: u8, octaves: usize) -> Option<u8> {
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
