//! Descriptor-driven egui shell.
//!
//! Everything shown for parameters (tabs, sections, widgets, hints) comes from
//! the [`Device`] descriptor, so a new instrument only supplies data.

use eframe::egui;
use egui::RichText;

use crate::descriptor::{self, Device, Evidence, Kind, Param, Values};
use crate::log::{Dir, Log};
use crate::midi::{self, MidiHub};
use crate::sysex::{self, WriteStyle};
use crate::widgets::{self, DANGER, MUTED, OK, WARN};

const PLAY_TAB: &str = "Play & test";
const CONSOLE_TAB: &str = "Console";
const REFERENCE_TAB: &str = "Reference";

struct OutMsg {
    label: String,
    bytes: Vec<u8>,
}

pub struct TribeApp {
    device: &'static Device,
    hub: MidiHub,
    out_ports: Vec<String>,
    in_ports: Vec<String>,
    selected_out: Option<String>,
    selected_in: Option<String>,
    values: Values,
    style: WriteStyle,
    device_id: u8,
    auto_apply: bool,
    dirty: bool,
    last_sent: Vec<(u8, Vec<u8>)>,
    queue: Vec<OutMsg>,
    log: Log,
    param_tabs: Vec<&'static str>,
    tab: usize,
    hex_input: String,
    hex_error: Option<String>,
    held_note: Option<u8>,
    bend: i16,
    modwheel: u8,
    confirm_restore: bool,
    settings_loaded: bool,
}

impl TribeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, device: &'static Device) -> Self {
        let mut app = Self {
            device,
            hub: MidiHub::default(),
            out_ports: Vec::new(),
            in_ports: Vec::new(),
            selected_out: None,
            selected_in: None,
            values: Values::defaults(device),
            style: device
                .styles
                .first()
                .copied()
                .unwrap_or(WriteStyle::PacketDirect),
            device_id: 0,
            auto_apply: true,
            dirty: false,
            last_sent: Vec::new(),
            queue: Vec::new(),
            log: Log::default(),
            param_tabs: device.tabs(),
            tab: 0,
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

        app.log.info(format!(
            "{} · model {:02X} {:02X} {:02X} · family {} · {} parameters",
            device.name,
            device.model[0],
            device.model[1],
            device.model[2],
            device.family,
            device.params.len()
        ));
        app.refresh_ports();
        app.auto_connect(&cc.egui_ctx);
        app
    }

    fn tab_titles(&self) -> Vec<&'static str> {
        let mut titles = self.param_tabs.clone();
        titles.push(PLAY_TAB);
        titles.push(CONSOLE_TAB);
        titles.push(REFERENCE_TAB);
        titles
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
        let hint = self.device.port_hint;
        if let Some(selected) = &self.selected_out
            && !self.out_ports.contains(selected)
        {
            self.selected_out = None;
        }
        if self.selected_out.is_none() {
            self.selected_out = midi::guess_port(&self.out_ports, hint);
        }
        if let Some(selected) = &self.selected_in
            && !self.in_ports.contains(selected)
        {
            self.selected_in = None;
        }
        if self.selected_in.is_none() {
            self.selected_in = midi::guess_port(&self.in_ports, hint);
        }
    }

    fn auto_connect(&mut self, ctx: &egui::Context) {
        let Some(name) = self.selected_out.clone() else {
            self.log.info(format!(
                "no {} output found ({} MIDI outputs visible)",
                self.device.name,
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

    fn read_settings(&mut self) {
        match descriptor::read_request(self.device, self.device_id) {
            Some(bytes) => self.send_message("read settings", bytes),
            None => self.log.info("this device has no documented settings read"),
        }
    }

    fn send_all(&mut self) {
        self.last_sent.clear();
        for (group_id, bytes) in
            descriptor::all_messages(self.device, &self.values, self.style, self.device_id)
        {
            if let Some(group) = self.device.write_group(group_id) {
                let payload = descriptor::payload_for(&self.values, group);
                self.remember(group.addr.packet(), &payload);
            }
            self.queue.push(OutMsg {
                label: group_id.to_string(),
                bytes,
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

    /// Queue the write group that carries `param_id`, honouring auto-apply and
    /// suppressing repeats of a value that was already sent.
    fn write_param(&mut self, param_id: &str) {
        let Some(param) = self.device.param(param_id) else {
            return;
        };
        let Some(group) = self.device.write_group(param.write) else {
            return;
        };
        if !self.auto_apply {
            self.dirty = true;
            return;
        }
        let payload = descriptor::payload_for(&self.values, group);
        let packet = group.addr.packet();
        if self.was_sent(packet, &payload) {
            return;
        }
        self.remember(packet, &payload);
        let bytes =
            descriptor::message_for(self.device, &self.values, self.style, self.device_id, group);
        self.queue.push(OutMsg {
            label: format!("{} ({packet:02X})", param.title),
            bytes,
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

    fn classify(&mut self, bytes: &[u8]) -> (Dir, String) {
        let Some(message) = descriptor::parse(self.device, bytes) else {
            return match bytes.first() {
                Some(0xF0) => (Dir::Error, "SysEx (not for this device)".to_string()),
                Some(status) => (
                    Dir::Rx,
                    format!("MIDI channel message (ch {})", (status & 0x0F) + 1),
                ),
                None => (Dir::Info, "empty message".to_string()),
            };
        };

        if let Some(block) = message.body.strip_prefix(&[sysex::op::SETTINGS_BLOCK]) {
            let expected = self.device.read.map(|spec| spec.block_len).unwrap_or(0);
            if block.len() >= expected && expected > 0 {
                let applied = descriptor::apply_block(self.device, &mut self.values, block);
                self.last_sent.clear();
                self.settings_loaded = true;
                return (
                    Dir::Rx,
                    format!(
                        "DID {:02X} · settings block · {applied} parameters loaded",
                        message.device_id
                    ),
                );
            }
            return (
                Dir::Error,
                format!(
                    "settings block is {} bytes, expected {expected}",
                    block.len()
                ),
            );
        }

        let rejected = matches!(
            descriptor::ack_code(message.body),
            Some(code) if code != sysex::op::ACK_OK
        );
        let label = format!(
            "DID {:02X} · {}",
            message.device_id,
            descriptor::decode_reply(self.device, message.body)
                .unwrap_or_else(|| "unknown reply".to_string())
        );
        if rejected {
            (Dir::Error, label)
        } else {
            (Dir::Rx, label)
        }
    }

    // ------------------------------------------------------------ param row

    fn param_row(&mut self, ui: &mut egui::Ui, param: &'static Param) {
        ui.label(param.title);
        let mut value = self.values.get_or(param.id, param.default);
        let changed = match param.kind {
            Kind::Byte { min, max } => ui.add(egui::Slider::new(&mut value, min..=max)).changed(),
            Kind::Bool => {
                let mut flag = value != 0;
                let checkbox_label = if flag { "on" } else { "off" };
                let response = ui.checkbox(&mut flag, checkbox_label);
                value = u8::from(flag);
                response.changed()
            }
            Kind::Enum(labels) => {
                let mut picked = None;
                egui::ComboBox::from_id_salt(param.id)
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
                match picked {
                    Some(byte) => {
                        value = byte;
                        true
                    }
                    None => false,
                }
            }
        };
        ui.horizontal(|ui| {
            widgets::hint_cell(ui, param.hint);
            match param.evidence {
                Evidence::Qsg => {}
                Evidence::Verified => {
                    ui.label(RichText::new("verified").color(OK).size(11.0));
                }
                Evidence::Unsupported => {
                    ui.label(RichText::new("rejected by firmware").color(WARN).size(11.0));
                }
            }
        });
        ui.end_row();
        if changed {
            self.values.set(param.id, value);
            self.write_param(param.id);
        }
    }

    // ------------------------------------------------------------- panels

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.device.name).size(19.0).strong());
            ui.label(RichText::new("opentribe").weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.checkbox(&mut self.auto_apply, "auto-apply").changed()
                    && self.auto_apply
                    && self.dirty
                {
                    self.send_all();
                }
                if ui
                    .button("Send all")
                    .on_hover_text(format!(
                        "Write all {} parameter groups",
                        self.device.writes.len()
                    ))
                    .clicked()
                {
                    self.send_all();
                }
                if ui
                    .button("Read")
                    .on_hover_text("Read all settings from the device")
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
                        .map(|name| widgets::short_port(&name))
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
                        .map(|name| widgets::short_port(&name))
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
                    widgets::status_dot(ui, OK);
                    ui.label(format!("out: {}", widgets::short_port(name)));
                }
                None => {
                    widgets::status_dot(ui, MUTED);
                    ui.label(RichText::new("out: not connected").color(MUTED));
                }
            }
            if let Some(name) = self.hub.input_name() {
                ui.separator();
                widgets::status_dot(ui, OK);
                ui.label(format!("in: {}", widgets::short_port(name)));
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
            if self.device.styles.len() > 1 {
                ui.separator();
                ui.label("Framing");
                let mut style = self.style;
                egui::ComboBox::from_id_salt("framing")
                    .width(250.0)
                    .truncate()
                    .selected_text(style.label())
                    .show_ui(ui, |ui| {
                        for candidate in self.device.styles {
                            ui.selectable_value(&mut style, *candidate, candidate.label());
                        }
                    });
                if style != self.style {
                    self.style = style;
                    self.last_sent.clear();
                }
            }
        });
        ui.add_space(4.0);
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
        let titles = self.tab_titles();
        ui.horizontal(|ui| {
            for (index, title) in titles.iter().enumerate() {
                if ui.selectable_label(self.tab == index, *title).clicked() {
                    self.tab = index;
                }
            }
        });
        ui.separator();
        if let Some(title) = titles.get(self.tab).copied() {
            if self.param_tabs.contains(&title) {
                self.params_tab(ui, title);
            } else if title == PLAY_TAB {
                self.play_tab(ui);
            } else if title == CONSOLE_TAB {
                self.console_tab(ui);
            } else {
                self.reference_tab(ui);
            }
        }
    }

    fn params_tab(&mut self, ui: &mut egui::Ui, tab: &'static str) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (section, params) in self.device.sections(tab) {
                    widgets::section(ui, section);
                    egui::Grid::new(format!("{tab}/{section}"))
                        .num_columns(3)
                        .spacing([12.0, 6.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for param in params {
                                self.param_row(ui, param);
                            }
                        });
                }
                ui.add_space(6.0);
                for line in self.device.notes {
                    ui.weak(format!("• {line}"));
                }
            });
    }

    fn play_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                widgets::section(ui, "Play");
                ui.weak(format!(
                    "Note, bend and wheel messages go straight to MIDI channel {}.",
                    self.performance_channel()
                ));
                ui.add_space(4.0);
                let hit = widgets::keyboard(ui, self.held_note, 48, 2);
                let pointer_down = ui.input(|i| i.pointer.primary_down());
                self.handle_keyboard(hit, pointer_down);

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("Pitch bend");
                    let response =
                        ui.add(egui::Slider::new(&mut self.bend, -8192..=8191).show_value(false));
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
                if ui.button("All notes off").clicked() {
                    self.send_all_notes_off();
                }

                if self.device.restore.is_some() {
                    ui.separator();
                    widgets::section(ui, "Maintenance");
                    if ui
                        .button(RichText::new("Restore factory settings").color(DANGER))
                        .clicked()
                    {
                        self.confirm_restore = true;
                    }
                    ui.weak("Overwrites every stored setting on the instrument.");
                }
            });
        self.restore_modal(ui.ctx());
    }

    fn console_tab(&mut self, ui: &mut egui::Ui) {
        widgets::section(ui, "Read requests");
        ui.weak("ACK codes are shown in the log; rejected requests are logged in red.");
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            if self.device.read.is_some() && ui.button("Read settings").clicked() {
                self.read_settings();
            }
            if ui.button("Query mode").clicked() {
                let bytes = descriptor::mode_request(self.device, self.device_id);
                self.send_message("query mode", bytes);
            }
            if ui.button("Firmware version").clicked() {
                let bytes = descriptor::firmware_request(self.device, self.device_id);
                self.send_message("firmware version", bytes);
            }
        });

        ui.separator();
        widgets::section(ui, "Send raw SysEx");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.hex_input)
                    .desired_width(460.0)
                    .hint_text("F0 … F7   or body only, e.g. 2F 01"),
            );
            if ui.button("Send").clicked() {
                self.send_hex();
            }
        });
        ui.weak(
            "Hex pairs; spaces, commas, colons and 0x prefixes are ignored. \
             A body without F0 is wrapped in the device envelope, a missing F7 is appended.",
        );
        if let Some(error) = &self.hex_error {
            ui.colored_label(DANGER, error);
        }

        ui.separator();
        widgets::section(ui, "Envelope in use");
        if let Some(sample) = self.example_message() {
            ui.monospace(sysex::hex(&sample));
        }
        ui.weak(format!(
            "Device ID {:02X} · {} · model {:02X} {:02X} {:02X} · family {}",
            self.device_id,
            self.style.label(),
            self.device.model[0],
            self.device.model[1],
            self.device.model[2],
            self.device.family
        ));
    }

    fn reference_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                widgets::section(ui, "Parameters");
                let row_height = 11.5;
                egui::ScrollArea::horizontal()
                    .id_salt("specs_scroll")
                    .show(ui, |ui| {
                        egui::Grid::new("specs")
                            .num_columns(7)
                            .spacing([10.0, 3.0])
                            .striped(true)
                            .show(ui, |ui| {
                                for header in [
                                    "parameter",
                                    "pkt",
                                    "payload",
                                    "range",
                                    "default",
                                    "blk",
                                    "source",
                                ] {
                                    ui.label(RichText::new(header).strong().size(row_height));
                                }
                                ui.end_row();
                                for param in self.device.params {
                                    let group = self.device.write_group(param.write);
                                    let (packet, payload) = match group {
                                        Some(group) => (
                                            format!("{:02X}", group.addr.packet()),
                                            payload_template(group),
                                        ),
                                        None => ("-".to_string(), "-".to_string()),
                                    };
                                    ui.label(RichText::new(param.title).size(row_height));
                                    ui.label(RichText::new(packet).monospace().size(row_height));
                                    ui.label(RichText::new(payload).monospace().size(row_height));
                                    ui.label(
                                        RichText::new(kind_range(param.kind))
                                            .weak()
                                            .size(row_height),
                                    );
                                    ui.label(
                                        RichText::new(param.kind.label(param.default))
                                            .weak()
                                            .size(row_height),
                                    );
                                    let index = param
                                        .read_index
                                        .map(|index| index.to_string())
                                        .unwrap_or_else(|| "-".to_string());
                                    ui.label(RichText::new(index).monospace().size(row_height));
                                    let color = match param.evidence {
                                        Evidence::Verified => OK,
                                        Evidence::Qsg => MUTED,
                                        Evidence::Unsupported => WARN,
                                    };
                                    ui.label(
                                        RichText::new(param.evidence.tag())
                                            .color(color)
                                            .size(row_height),
                                    );
                                    ui.end_row();
                                }
                            });
                    });

                if !self.device.rejected.is_empty() {
                    widgets::section(ui, "Rejected by the firmware");
                    for (name, packet) in self.device.rejected {
                        ui.label(format!(
                            "• {name} · packet {packet:02X} · not sent by this app"
                        ));
                    }
                }

                widgets::section(ui, "Verified on hardware");
                for line in self.device.notes {
                    ui.label(format!("• {line}"));
                }

                widgets::section(ui, "Limits");
                for line in self.device.limits {
                    ui.label(format!("• {line}"));
                }
            });
    }

    // --------------------------------------------------------- performance

    fn performance_channel(&self) -> u8 {
        match self.device.channel_param.and_then(|id| self.values.get(id)) {
            Some(channel) if (1..=16).contains(&channel) => channel,
            _ => 1,
        }
    }

    fn channel_nibble(&self) -> u8 {
        self.performance_channel().clamp(1, 16) - 1
    }

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

    fn send_note(&mut self, note: u8, on: bool) {
        let status = if on { 0x90 } else { 0x80 } | self.channel_nibble();
        let velocity = if on { 100 } else { 64 };
        self.send_performance("note", vec![status, note & 0x7F, velocity]);
    }

    fn send_bend(&mut self) {
        let value = (i32::from(self.bend) + 8192).clamp(0, 16383) as u16;
        let status = 0xE0 | self.channel_nibble();
        self.send_performance(
            "pitch bend",
            vec![status, (value & 0x7F) as u8, (value >> 7) as u8],
        );
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

    /// Performance traffic is sent immediately and not logged, otherwise
    /// playing the on-screen keyboard would flood the console.
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
                    bytes = sysex::envelope(
                        self.device.model,
                        self.device.family,
                        self.device_id,
                        &bytes,
                    );
                }
                self.send_message("manual", bytes);
            }
            Err(err) => self.hex_error = Some(err),
        }
    }

    fn example_message(&self) -> Option<Vec<u8>> {
        let group = self.device.writes.first()?;
        Some(descriptor::message_for(
            self.device,
            &self.values,
            self.style,
            self.device_id,
            group,
        ))
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
                ui.label("This overwrites every stored setting on the instrument.");
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
            if let Some(bytes) = descriptor::restore_request(self.device, self.device_id) {
                self.send_message("restore factory settings", bytes);
            }
            self.confirm_restore = false;
        } else if cancelled || !open {
            self.confirm_restore = false;
        }
    }
}

impl eframe::App for TribeApp {
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

fn payload_template(group: &descriptor::WriteGroup) -> String {
    use descriptor::Slot;
    let mut parts: Vec<String> = Vec::new();
    if let Some(sub) = group.addr.sub() {
        parts.push(format!("{sub:02X}"));
    }
    for slot in group.slots {
        match slot {
            Slot::Lit(byte) => parts.push(format!("{byte:02X}")),
            Slot::Param(id) => {
                let short = id.rsplit('.').next().unwrap_or(id);
                parts.push(format!("<{short}>"));
            }
        }
    }
    parts.join(" ")
}

fn kind_range(kind: Kind) -> String {
    match kind {
        Kind::Byte { min, max } => format!("{min}-{max}"),
        Kind::Bool => "on/off".to_string(),
        Kind::Enum(labels) => labels.join(" / "),
    }
}
