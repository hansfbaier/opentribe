//! Behringer PRO-800 editor, librarian and controller.
//!
//! The PRO-800 speaks whole patches: `77 <lsb> <msb>` reads a program,
//! `78 <lsb> <msb> <7-in-8 payload>` writes it. This crate ports the Swumpf
//! editor's control surface (`vendor/swumpf/swumpf_114.html`) to the same
//! `egui` shell the other opentribe instruments use, and adds a headless CLI.

pub mod app;
pub mod cli;
pub mod librarian;
pub mod params;
pub mod patch;
pub mod transport;

pub use patch::Patch;

/// Name shown in the window title and CLI help.
pub const DEVICE_NAME: &str = "PRO-800";
/// Substring used to find the MIDI port.
pub const PORT_HINT: &str = "PRO 800";
/// Firmware the editor was written against.
pub const KNOWN_FIRMWARE: &str = "1.4.6";

/// Start the GUI, optionally saving one screenshot and exiting.
pub fn run(screenshot: Option<&str>) {
    let title = format!("opentribe · {DEVICE_NAME}");
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size([1180.0, 820.0])
            .with_min_inner_size([860.0, 600.0]),
        ..Default::default()
    };
    let shot = screenshot.map(std::path::PathBuf::from);
    let result = eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            let app: Box<dyn eframe::App> = match shot {
                Some(path) => Box::new(app::ShotApp::new(cc, path)),
                None => Box::new(app::Pro800App::new(cc)),
            };
            Ok(app)
        }),
    );
    if let Err(err) = result {
        eprintln!("opentribe: failed to start the UI: {err}");
    }
}
