//! `--screenshot <file.png>` support: run the GUI normally, draw about 120
//! frames, save the window's picture and exit.
//!
//! This is what lets tools (CI, Craft Anything's build check) photograph the
//! app without a human: the harness below wraps the regular [`TribeApp`],
//! counts frames, asks egui for a screenshot of the next painted frame and
//! writes it as a PNG before closing the viewport.

use std::path::PathBuf;

use eframe::egui;

use crate::app::TribeApp;
use crate::descriptor::Device;

/// Frame at which the screenshot is requested (the picture is taken the frame
/// after the request, so roughly 120 frames are drawn).
const SHOT_FRAME: u64 = 120;

/// Give up and close the window if no screenshot arrives within this many
/// frames (a few minutes at vsync).
const SHOT_DEADLINE: u64 = 1800;

pub struct ScreenshotApp {
    inner: TribeApp,
    path: PathBuf,
    frames: u64,
    requested: bool,
}

impl ScreenshotApp {
    pub fn new(cc: &eframe::CreationContext<'_>, device: &'static Device, path: &str) -> Self {
        Self {
            inner: TribeApp::new(cc, device),
            path: PathBuf::from(path),
            frames: 0,
            requested: false,
        }
    }
}

impl eframe::App for ScreenshotApp {
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
        } else if self.requested && self.frames > SHOT_DEADLINE {
            eprintln!(
                "screenshot: nothing delivered after {} frames, giving up",
                self.frames
            );
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Drive frames continuously so the picture is taken without input.
        ui.ctx().request_repaint();
    }
}

/// Encode an egui screenshot as a PNG file.
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
