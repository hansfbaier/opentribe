//! Open POLY D — an egui controller for the Behringer POLY D global SysEx map.

mod app;
mod midi;
mod model;
mod sysex;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Open POLY D")
            .with_inner_size([1040.0, 760.0])
            .with_min_inner_size([760.0, 540.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Open POLY D",
        options,
        Box::new(|cc| Ok(Box::new(app::PolyDApp::new(cc)))),
    )
}
