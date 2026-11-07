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
pub mod samples;
pub mod screenshot;
pub mod sysex;
pub mod widgets;

pub use app::TribeApp;
pub use descriptor::Device;

/// Run the GUI for one device.
///
/// With `screenshot` set the app draws about 120 frames, saves the window's
/// picture as a PNG to that path and exits (see [`screenshot::ScreenshotApp`]).
pub fn run(device: &'static Device, screenshot: Option<&str>) {
    let title = format!("opentribe · {}", device.name);
    let options = eframe::NativeOptions {
        // wgpu: works on machines without OpenGL drivers (DX12/Vulkan, with a
        // software fallback) and on Linux (Vulkan/GL), so the same build runs
        // on the widest range of machines.
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size([1040.0, 760.0])
            .with_min_inner_size([760.0, 540.0]),
        ..Default::default()
    };
    let result = match screenshot {
        Some(path) => eframe::run_native(
            &title,
            options,
            Box::new(move |cc| Ok(Box::new(screenshot::ScreenshotApp::new(cc, device, path)))),
        ),
        None => eframe::run_native(
            &title,
            options,
            Box::new(move |cc| Ok(Box::new(TribeApp::new(cc, device)))),
        ),
    };
    if let Err(err) = result {
        eprintln!("opentribe: failed to start the UI: {err}");
    }
}
