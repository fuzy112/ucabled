// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;

/// Exit codes for `--select`: the agent maps them to the user's choice.
const EXIT_USE_PHONE: i32 = 0;
const EXIT_DECLINE: i32 = 1;

/// Shown when another authenticator (e.g. a physical security key) is plugged
/// in and Firefox asks the user to pick one. Firefox itself only supports
/// picking by touching a physical key, so this window is how the user chooses
/// the phone instead.
struct SelectWindow {
    start: Instant,
    timeout: Duration,
}

impl eframe::App for SelectWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) {
            std::process::exit(EXIT_DECLINE);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            std::process::exit(EXIT_DECLINE);
        }
        if self.start.elapsed() >= self.timeout {
            std::process::exit(EXIT_DECLINE);
        }

        ui.vertical_centered(|ui| {
            ui.add_space(12.0);
            ui.heading("Passkey request");
            ui.add_space(6.0);
            ui.label("A security key is also connected.");
            ui.label("Use your phone passkey instead?");
            ui.add_space(12.0);
            if ui.button("Use phone").clicked() {
                std::process::exit(EXIT_USE_PHONE);
            }
            if ui.button("Cancel").clicked() {
                std::process::exit(EXIT_DECLINE);
            }
        });
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

struct QrWindow {
    rp: Option<String>,
    texture: egui::TextureHandle,
    /// A heavily downscaled copy, drawn scaled up so the code is illegible
    /// once the phone has scanned it.
    blurred: egui::TextureHandle,
    phone_found: Arc<AtomicBool>,
    start: Instant,
    timeout: Duration,
}

impl eframe::App for QrWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) {
            std::process::exit(0);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            std::process::exit(0);
        }

        let found = self.phone_found.load(Ordering::Relaxed);
        let remaining = self.timeout.saturating_sub(self.start.elapsed());
        let expired = !found && remaining.is_zero();

        let mut close = false;
        ui.vertical_centered(|ui| {
            ui.add_space(12.0);
            ui.heading("Passkey request");
            if let Some(rp) = &self.rp {
                ui.label(format!("Site: {rp}"));
            }
            ui.add_space(6.0);
            if found {
                ui.image(egui::load::SizedTexture::new(
                    self.blurred.id(),
                    egui::vec2(300.0, 300.0),
                ));
                ui.label("Phone detected — confirm on your phone");
            } else if expired {
                ui.label("Code expired. Close this window and start again.");
            } else {
                ui.image(egui::load::SizedTexture::new(
                    self.texture.id(),
                    egui::vec2(300.0, 300.0),
                ));
                ui.label("Scan with your phone (iCloud Keychain / Google Password Manager)");
                ui.label(format!("Code expires in {}s", remaining.as_secs()));
            }
            ui.add_space(6.0);
            if ui
                .button(if expired { "Close" } else { "Cancel" })
                .clicked()
            {
                close = true;
            }
        });
        if close {
            std::process::exit(0);
        }

        // Keep the countdown ticking while the code is still valid.
        if !found && !expired {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

/// Build the crisp QR texture plus a heavily downscaled copy. The downscaled
/// copy is drawn scaled back up with linear filtering, which smears the module
/// pattern beyond recognition while still reading as "the code".
fn make_qr_textures(ctx: &egui::Context, url: &str) -> (egui::TextureHandle, egui::TextureHandle) {
    let code = qrcode::QrCode::new(url.as_bytes()).expect("invalid QR contents");
    let qr_width = code.width();
    let border = 4;
    let size = qr_width + border * 2;
    let mut img = egui::ColorImage::new([size, size], vec![egui::Color32::WHITE; size * size]);
    for y in 0..qr_width {
        for x in 0..qr_width {
            if code[(x, y)] == qrcode::types::Color::Dark {
                img[(x + border, y + border)] = egui::Color32::BLACK;
            }
        }
    }
    let crisp = ctx.load_texture("qr", img.clone(), egui::TextureOptions::NEAREST);

    // Averaging the image down to a handful of pixels destroys the module
    // pattern; upscaling that with linear filtering blurs it out.
    const BLUR_SIZE: usize = 6;
    let blurred = ctx.load_texture(
        "qr-blurred",
        downscale(&img, BLUR_SIZE),
        egui::TextureOptions::LINEAR,
    );
    (crisp, blurred)
}

/// Box-filter `img` down to `target` x `target` pixels.
fn downscale(img: &egui::ColorImage, target: usize) -> egui::ColorImage {
    let (w, h) = (img.width(), img.height());
    let mut out = egui::ColorImage::new(
        [target, target],
        vec![egui::Color32::WHITE; target * target],
    );
    for ty in 0..target {
        for tx in 0..target {
            let x0 = tx * w / target;
            let x1 = ((tx + 1) * w / target).max(x0 + 1);
            let y0 = ty * h / target;
            let y1 = ((ty + 1) * h / target).max(y0 + 1);
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = img[(x, y)];
                    r += p.r() as u32;
                    g += p.g() as u32;
                    b += p.b() as u32;
                    n += 1;
                }
            }
            out[(tx, ty)] = egui::Color32::from_rgb((r / n) as u8, (g / n) as u8, (b / n) as u8);
        }
    }
    out
}

/// Read the first non-empty line from stdin as the QR URL.
fn read_url_from_stdin(reader: &mut impl BufRead) -> Option<String> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
    }
}

/// Watch stdin for the daemon's status lines; currently only "found", sent
/// once the phone's BLE advert has been received. If the URL arrived on stdin
/// it has already been consumed by `read_url_from_stdin`.
fn watch_status(reader: impl BufRead + Send + 'static) -> Arc<AtomicBool> {
    let found = Arc::new(AtomicBool::new(false));
    let flag = found.clone();
    std::thread::spawn(move || {
        for line in reader.lines() {
            match line {
                Ok(l) if l.trim() == "found" => flag.store(true, Ordering::Relaxed),
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    found
}

fn main() -> eframe::Result<()> {
    let mut url: Option<String> = None;
    let mut rp: Option<String> = None;
    let mut timeout = Duration::from_secs(300);
    let mut select = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--select" => select = true,
            "--timeout" => {
                if let Some(secs) = args.next().and_then(|v| v.parse::<u64>().ok()) {
                    timeout = Duration::from_secs(secs);
                }
            }
            _ => {
                if url.is_none() && arg.starts_with("FIDO:/") {
                    url = Some(arg);
                } else if rp.is_none() {
                    rp = Some(arg);
                }
            }
        }
    }

    if select {
        return run_select_window(timeout);
    }

    // The daemon sends the QR URL on stdin (it carries the transaction secret,
    // so it must not appear in argv); any later lines are status updates.
    let mut stdin = std::io::BufReader::new(std::io::stdin());
    if url.is_none() {
        url = read_url_from_stdin(&mut stdin);
    }
    let url = url.expect("usage: ucable-agent-helper [rp] [--timeout secs] (QR URL on stdin)");

    let phone_found = watch_status(stdin);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Phone Passkey Bridge")
            .with_app_id("ucabled")
            .with_inner_size([360.0, 480.0])
            .with_resizable(false)
            .with_decorations(false)
            .with_always_on_top(),
        ..Default::default()
    };
    eframe::run_native(
        "Phone Passkey Bridge",
        options,
        Box::new(move |cc| {
            let (texture, blurred) = make_qr_textures(&cc.egui_ctx, &url);
            Ok(Box::new(QrWindow {
                rp,
                texture,
                blurred,
                phone_found,
                start: Instant::now(),
                timeout,
            }))
        }),
    )
}

fn run_select_window(timeout: Duration) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Phone Passkey Bridge")
            .with_app_id("ucabled")
            .with_inner_size([360.0, 240.0])
            .with_resizable(false)
            .with_decorations(false)
            .with_always_on_top(),
        ..Default::default()
    };
    eframe::run_native(
        "Phone Passkey Bridge",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(SelectWindow {
                start: Instant::now(),
                timeout,
            }))
        }),
    )
}
