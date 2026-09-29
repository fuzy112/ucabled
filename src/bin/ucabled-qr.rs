use eframe::egui;

struct QrWindow {
    rp: Option<String>,
    texture: egui::TextureHandle,
}

impl eframe::App for QrWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) {
            std::process::exit(0);
        }

        let mut cancel = false;
        ui.vertical_centered(|ui| {
            ui.add_space(12.0);
            ui.heading("Passkey request");
            if let Some(rp) = &self.rp {
                ui.label(format!("Site: {rp}"));
            }
            ui.add_space(6.0);
            ui.image(egui::load::SizedTexture::new(
                self.texture.id(),
                egui::vec2(300.0, 300.0),
            ));
            ui.add_space(6.0);
            ui.label("Scan with your phone (iCloud Keychain / Google Password Manager)");
            ui.add_space(6.0);
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
        if cancel {
            std::process::exit(0);
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

fn main() -> eframe::Result<()> {
    let mut args = std::env::args().skip(1);
    let url = args.next().expect("usage: ucabled-qr <qr-url> [rp]");
    let rp = args.next();

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
            }))
        }),
    )
}
