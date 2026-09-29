use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;

struct QrWindow {
    rp: Option<String>,
    texture: egui::TextureHandle,
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
            if ui.button(if expired { "Close" } else { "Cancel" }).clicked() {
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

fn make_qr_texture(ctx: &egui::Context, url: &str) -> egui::TextureHandle {
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
    ctx.load_texture("qr", img, egui::TextureOptions::NEAREST)
}

/// Watch stdin for the daemon's status lines; currently only "found", sent
/// once the phone's BLE advert has been received.
fn watch_status() -> Arc<AtomicBool> {
    let found = Arc::new(AtomicBool::new(false));
    let flag = found.clone();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
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
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--timeout" => {
                if let Some(secs) = args.next().and_then(|v| v.parse::<u64>().ok()) {
                    timeout = Duration::from_secs(secs);
                }
            }
            _ => {
                if url.is_none() {
                    url = Some(arg);
                } else if rp.is_none() {
                    rp = Some(arg);
                }
            }
        }
    }
    let url = url.expect("usage: ucabled-qr <qr-url> [rp] [--timeout secs]");

    let phone_found = watch_status();
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
            Ok(Box::new(QrWindow {
                rp,
                texture: make_qr_texture(&cc.egui_ctx, &url),
                phone_found,
                start: Instant::now(),
                timeout,
            }))
        }),
    )
}
