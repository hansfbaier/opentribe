//! Shared core for the `opentribe` device controllers.
//!
//! A controller is a [`descriptor::Device`] plus this shell: the descriptor
//! supplies parameters, write groups, read layout and notes, the shell supplies
//! MIDI I/O, the egui UI, the message log and the raw SysEx console.

pub mod app;
pub mod cli;
pub mod descriptor;
pub mod log;
pub mod midi;
pub mod sysex;
pub mod widgets;

pub use app::TribeApp;
pub use descriptor::Device;

/// Run the GUI for one device.
pub fn run(device: &'static Device) {
    let title = format!("opentribe · {}", device.name);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size([1040.0, 760.0])
            .with_min_inner_size([760.0, 540.0]),
        ..Default::default()
    };
    if let Err(err) = eframe::run_native(
        &title,
        options,
        Box::new(move |cc| Ok(Box::new(TribeApp::new(cc, device)))),
    ) {
        eprintln!("opentribe: failed to start the UI: {err}");
    }
}
