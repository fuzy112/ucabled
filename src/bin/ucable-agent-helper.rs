// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::BufRead;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, FontFamily, FontId, Margin, RichText, Stroke, StrokeKind, TextStyle};

/// Exit codes for `--select`: the agent maps them to the user's choice.
const EXIT_USE_PHONE: i32 = 0;
const EXIT_DECLINE: i32 = 1;

/// Rounded-corner radius of the whole window (drawn by us; the window itself
/// is transparent so the corners outside the rounded rectangle stay clear).
const WINDOW_RADIUS: f32 = 16.0;
/// Width of the content column.
const CONTENT_WIDTH: f32 = 300.0;
/// Side length of the QR image drawn inside its white card.
const QR_SIZE: f32 = 260.0;

/// Colours for one theme. egui already ships a decent dark and light style;
/// this just ties the window background, accents and progress bar together so
/// the two helper windows look like one product.
#[derive(Clone, Copy)]
struct Palette {
    dark: bool,
    bg: Color32,
    card: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    accent: Color32,
    accent_hover: Color32,
    accent_text: Color32,
    track: Color32,
    warn: Color32,
}

impl Palette {
    fn for_dark(dark: bool) -> Self {
        if dark {
            Self {
                dark,
                bg: Color32::from_rgb(0x1e, 0x1f, 0x24),
                card: Color32::from_rgb(0x2a, 0x2c, 0x33),
                border: Color32::from_rgb(0x3a, 0x3d, 0x45),
                text: Color32::from_rgb(0xec, 0xed, 0xf0),
                muted: Color32::from_rgb(0x9b, 0xa0, 0xab),
                accent: Color32::from_rgb(0x4f, 0x9c, 0xf9),
                accent_hover: Color32::from_rgb(0x6a, 0xac, 0xfa),
                accent_text: Color32::from_rgb(0x0b, 0x12, 0x1e),
                track: Color32::from_rgb(0x33, 0x36, 0x3d),
                warn: Color32::from_rgb(0xf2, 0x6b, 0x6b),
            }
        } else {
            Self {
                dark,
                bg: Color32::from_rgb(0xfa, 0xfa, 0xfc),
                card: Color32::from_rgb(0xef, 0xf0, 0xf4),
                border: Color32::from_rgb(0xd6, 0xd9, 0xe0),
                text: Color32::from_rgb(0x1b, 0x1d, 0x22),
                muted: Color32::from_rgb(0x5c, 0x63, 0x70),
                accent: Color32::from_rgb(0x2f, 0x6f, 0xe0),
                accent_hover: Color32::from_rgb(0x25, 0x5f, 0xc8),
                accent_text: Color32::from_rgb(0xff, 0xff, 0xff),
                track: Color32::from_rgb(0xe2, 0xe4, 0xea),
                warn: Color32::from_rgb(0xd6, 0x45, 0x45),
            }
        }
    }

    /// Colour of the countdown bar: the accent while there is time left, fading
    /// to a warning colour as the code runs out.
    fn bar_color(&self, fraction: f32) -> Color32 {
        let f = fraction.clamp(0.0, 1.0);
        if f >= 0.35 {
            self.accent
        } else {
            lerp_color(self.warn, self.accent, f / 0.35)
        }
    }
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// Pick a theme from the desktop's GTK settings. The xdg-desktop-portal
/// Settings interface is the canonical source (it is what GTK itself reports);
/// we fall back to reading the GTK settings.ini files and then to the
/// `GTK_THEME` environment variable, and finally assume dark.
fn detect_dark_theme() -> bool {
    portal_color_scheme()
        .or_else(gtk_settings_dark)
        .or_else(env_theme_dark)
        .unwrap_or(true)
}

/// `org.freedesktop.appearance color-scheme`: 0 = no preference, 1 = dark,
/// 2 = light. A missing portal or bus simply yields `None`.
fn portal_color_scheme() -> Option<bool> {
    let conn = dbus::blocking::Connection::new_session().ok()?;
    let proxy = conn.with_proxy(
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        Duration::from_millis(500),
    );
    let (scheme,): (dbus::arg::Variant<u32>,) = proxy
        .method_call(
            "org.freedesktop.portal.Settings",
            "Read",
            ("org.freedesktop.appearance", "color-scheme"),
        )
        .ok()?;
    match scheme.0 {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

fn gtk_settings_dark() -> Option<bool> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(config) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    {
        candidates.push(config.join("gtk-4.0/settings.ini"));
        candidates.push(config.join("gtk-3.0/settings.ini"));
    }
    candidates.push(PathBuf::from("/etc/gtk-4.0/settings.ini"));
    candidates.push(PathBuf::from("/etc/gtk-3.0/settings.ini"));

    for path in candidates {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(dark) = parse_gtk_settings(&text) {
                return Some(dark);
            }
        }
    }
    None
}

fn parse_gtk_settings(text: &str) -> Option<bool> {
    let mut prefer_dark = None;
    let mut theme_is_dark = None;
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "gtk-application-prefer-dark-theme" => {
                prefer_dark = Some(value.eq_ignore_ascii_case("true") || value == "1");
            }
            "gtk-theme-name" => {
                theme_is_dark = Some(value.to_ascii_lowercase().contains("dark"));
            }
            _ => {}
        }
    }
    // An explicit preference wins over guessing from the theme name.
    prefer_dark.or(theme_is_dark)
}

fn env_theme_dark() -> Option<bool> {
    let theme = std::env::var("GTK_THEME").ok()?.to_ascii_lowercase();
    if theme.contains("dark") {
        Some(true)
    } else if theme.contains("light") {
        Some(false)
    } else {
        None
    }
}

/// Apply the palette and a slightly larger, higher-contrast type scale.
fn apply_style(ctx: &egui::Context, pal: &Palette) {
    ctx.set_theme(if pal.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });

    let mut visuals = if pal.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = pal.bg;
    visuals.window_fill = pal.bg;
    visuals.window_stroke = Stroke::new(1.0, pal.border);
    visuals.window_corner_radius = WINDOW_RADIUS.into();
    visuals.extreme_bg_color = pal.track;
    visuals.faint_bg_color = pal.card;
    visuals.selection.bg_fill = pal.accent;
    visuals.selection.stroke = Stroke::new(1.0, pal.accent_text);
    visuals.hyperlink_color = pal.accent;
    visuals.override_text_color = None;

    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = 12.into();
        widget.fg_stroke.color = pal.text;
    }
    visuals.widgets.noninteractive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.weak_bg_fill = pal.card;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, pal.border);
    visuals.widgets.hovered.weak_bg_fill = pal.border;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, pal.accent);
    visuals.widgets.active.weak_bg_fill = pal.border;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, pal.accent);

    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(26.0, FontFamily::Proportional),
        );
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(15.5, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(16.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        );
        style.spacing.item_spacing = egui::vec2(6.0, 9.0);
        style.spacing.button_padding = egui::vec2(16.0, 10.0);
    });
}

/// Paint the rounded window background on the transparent native window.
fn paint_background(ui: &egui::Ui, pal: &Palette) {
    let rect = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(rect, WINDOW_RADIUS, pal.bg);
    painter.rect_stroke(
        rect,
        WINDOW_RADIUS,
        Stroke::new(1.0, pal.border),
        StrokeKind::Inside,
    );
}

/// Filled, accent-coloured call to action. The style is scoped so the button
/// still gets hover/press feedback from egui's widget state.
fn primary_button(ui: &mut egui::Ui, pal: &Palette, text: &str, width: f32) -> bool {
    ui.scope(|ui| {
        {
            let widgets = &mut ui.style_mut().visuals.widgets;
            for state in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
            ] {
                state.weak_bg_fill = pal.accent;
                state.fg_stroke = Stroke::new(1.0, pal.accent_text);
                state.bg_stroke = Stroke::NONE;
                state.corner_radius = 12.into();
                state.expansion = 0.0;
            }
            widgets.hovered.weak_bg_fill = pal.accent_hover;
            widgets.active.weak_bg_fill = pal.accent_hover;
        }
        ui.add_sized(
            egui::vec2(width, 44.0),
            egui::Button::new(RichText::new(text).size(16.0)),
        )
        .clicked()
    })
    .inner
}

fn secondary_button(ui: &mut egui::Ui, pal: &Palette, text: &str, width: f32) -> bool {
    ui.add_sized(
        egui::vec2(width, 40.0),
        egui::Button::new(RichText::new(text).size(15.0).color(pal.muted)),
    )
    .clicked()
}

fn qr_card(ui: &mut egui::Ui, id: egui::TextureId, pal: &Palette) {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .corner_radius(14)
        .inner_margin(Margin::same(10))
        .stroke(Stroke::new(1.0, pal.border))
        .show(ui, |ui| {
            ui.image(egui::load::SizedTexture::new(
                id,
                egui::vec2(QR_SIZE, QR_SIZE),
            ));
        });
}

fn countdown_bar(ui: &mut egui::Ui, pal: &Palette, fraction: f32) {
    ui.add(
        egui::ProgressBar::new(fraction)
            .desired_width(CONTENT_WIDTH)
            .desired_height(8.0)
            .fill(pal.bar_color(fraction))
            .corner_radius(4),
    );
}

/// Shown when another authenticator (e.g. a physical security key) is plugged
/// in and Firefox asks the user to pick one. Firefox itself only supports
/// picking by touching a physical key, so this window is how the user chooses
/// the phone instead.
struct SelectWindow {
    start: Instant,
    timeout: Duration,
    pal: Palette,
}

impl eframe::App for SelectWindow {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

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

        let remaining = self.timeout.saturating_sub(self.start.elapsed());
        let fraction = fraction_remaining(remaining, self.timeout);
        let pal = self.pal;

        paint_background(ui, &pal);
        ui.vertical_centered(|ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(20, 18))
                .show(ui, |ui| {
                    ui.set_width(CONTENT_WIDTH);
                    ui.vertical_centered(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("🔑").size(38.0));
                            ui.add_space(10.0);
                            ui.label(RichText::new("or").size(14.0).color(pal.muted));
                            ui.add_space(10.0);
                            ui.label(RichText::new("📱").size(38.0));
                        });
                        ui.add_space(10.0);
                        ui.label(RichText::new("Passkey request").size(24.0).color(pal.text));
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("A security key is also connected.")
                                .size(14.0)
                                .color(pal.muted),
                        );
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new("Use your phone's passkey instead?")
                                .size(15.0)
                                .color(pal.text),
                        );
                        ui.add_space(18.0);
                        if primary_button(ui, &pal, "Use phone", CONTENT_WIDTH) {
                            std::process::exit(EXIT_USE_PHONE);
                        }
                        ui.add_space(6.0);
                        if secondary_button(ui, &pal, "Keep using the security key", CONTENT_WIDTH)
                        {
                            std::process::exit(EXIT_DECLINE);
                        }
                        ui.add_space(16.0);
                        countdown_bar(ui, &pal, fraction);
                    });
                });
        });

        // Keep the countdown bar moving smoothly.
        ctx.request_repaint_after(Duration::from_millis(50));
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
    pal: Palette,
}

impl eframe::App for QrWindow {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

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
        let fraction = fraction_remaining(remaining, self.timeout);
        let expired = !found && remaining.is_zero();
        let pal = self.pal;

        let mut close = false;
        paint_background(ui, &pal);
        ui.vertical_centered(|ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(20, 16))
                .show(ui, |ui| {
                    ui.set_width(CONTENT_WIDTH);
                    ui.vertical_centered(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add_space(2.0);
                        ui.label(RichText::new("Passkey request").size(26.0).color(pal.text));
                        if let Some(rp) = &self.rp {
                            ui.add_space(3.0);
                            ui.label(RichText::new(rp).size(14.0).color(pal.muted));
                        }
                        ui.add_space(12.0);
                        if found {
                            qr_card(ui, self.blurred.id(), &pal);
                            ui.add_space(10.0);
                            ui.label(RichText::new("Phone detected").size(16.0).color(pal.accent));
                            ui.add_space(3.0);
                            ui.label(
                                RichText::new("Confirm the sign-in on your phone")
                                    .size(13.0)
                                    .color(pal.muted),
                            );
                        } else if expired {
                            ui.add_space(90.0);
                            ui.label(RichText::new("Code expired").size(20.0).color(pal.text));
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new("Close this window and start again.")
                                    .size(14.0)
                                    .color(pal.muted),
                            );
                            ui.add_space(90.0);
                        } else {
                            qr_card(ui, self.texture.id(), &pal);
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new("Scan with your phone")
                                    .size(15.0)
                                    .color(pal.text),
                            );
                            ui.add_space(10.0);
                            countdown_bar(ui, &pal, fraction);
                        }
                        ui.add_space(14.0);
                        let label = if expired { "Close" } else { "Cancel" };
                        if primary_button(ui, &pal, label, CONTENT_WIDTH) {
                            close = true;
                        }
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Press Esc to dismiss")
                                .size(11.0)
                                .color(pal.muted),
                        );
                    });
                });
        });
        if close {
            std::process::exit(0);
        }

        // Keep the countdown bar moving while the code is still valid.
        if !found && !expired {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

fn fraction_remaining(remaining: Duration, timeout: Duration) -> f32 {
    let total = timeout.as_secs_f32();
    if total <= 0.0 {
        0.0
    } else {
        (remaining.as_secs_f32() / total).clamp(0.0, 1.0)
    }
}

/// Pixels per QR module in the rendered texture. The texture is drawn scaled
/// down into the card, so a handful of pixels per module stay crisp.
const QR_MODULE_PX: usize = 10;
/// Quiet zone around the code, in modules.
const QR_QUIET_MODULES: f32 = 4.0;
/// Anti-aliasing samples per axis.
const QR_AA: usize = 4;
/// When a centre image is drawn, force at least this version so the icon does
/// not eat enough of the error-correction budget to break decoding.
const QR_MIN_VERSION: i16 = 5;
/// Side of the centre image, in modules.
const QR_CENTER_MODULES: usize = 10;
/// The passkey icon as an 8-bit grayscale mask (0 = ink, 255 = paper), 100×100
/// px — one byte per module pixel for the `QR_CENTER_MODULES` patch at
/// `QR_MODULE_PX`. Regenerate with `rsvg-convert -w 100 -h 100 -b white
/// passkey.svg` piped through `ffmpeg -pix_fmt gray`.
const PASSKEY_ICON: &[u8; 100 * 100] = include_bytes!("resources/passkey.gray");

/// Build the QR code with error-correction level M and at least version 5.
fn make_qr_code(url: &str) -> qrcode::QrCode {
    for v in QR_MIN_VERSION..=40 {
        if let Ok(code) =
            qrcode::QrCode::with_version(url.as_bytes(), qrcode::Version::Normal(v), qrcode::EcLevel::M)
        {
            return code;
        }
    }
    qrcode::QrCode::with_error_correction_level(url.as_bytes(), qrcode::EcLevel::M)
        .expect("QR contents too long")
}

/// Is (px, py) inside the rounded rectangle at (x0, y0), size w×h, radius r?
fn in_round_rect(x0: f32, y0: f32, w: f32, h: f32, r: f32, px: f32, py: f32) -> bool {
    let dx = (px - (x0 + w / 2.0)).abs() - (w / 2.0 - r);
    let dy = (py - (y0 + h / 2.0)).abs() - (h / 2.0 - r);
    dx.max(0.0) * dx.max(0.0) + dy.max(0.0) * dy.max(0.0) <= r * r
}

/// Offset of module (x, y) inside the 7×7 finder pattern it belongs to, if any.
fn locator_local(n: usize, x: usize, y: usize) -> Option<(f32, f32)> {
    const S: usize = 7;
    if x < S && y < S {
        Some((x as f32, y as f32))
    } else if x >= n - S && y < S {
        Some(((x - (n - S)) as f32, y as f32))
    } else if x < S && y >= n - S {
        Some((x as f32, (y - (n - S)) as f32))
    } else {
        None
    }
}

/// Whether (mx, my) — module coordinates with the quiet zone stripped — is dark
/// in this rendering: round data modules and rounded finder patterns.
fn qr_module_dark(code: &qrcode::QrCode, n: usize, mx: f32, my: f32) -> bool {
    if mx < 0.0 || my < 0.0 || mx >= n as f32 || my >= n as f32 {
        return false;
    }
    let (x, y) = (mx as usize, my as usize);
    let (fx, fy) = (mx - x as f32, my - y as f32);

    if let Some((lx, ly)) = locator_local(n, x, y) {
        // Three nested rounded rectangles: 7×7 ring, 5×5 gap, 3×3 core, all
        // with a corner radius of one module.
        let (px, py) = (lx + fx, ly + fy);
        if in_round_rect(2.0, 2.0, 3.0, 3.0, 1.0, px, py) {
            return true;
        }
        if in_round_rect(1.0, 1.0, 5.0, 5.0, 1.0, px, py) {
            return false;
        }
        return in_round_rect(0.0, 0.0, 7.0, 7.0, 1.0, px, py);
    }

    if code[(x, y)] == qrcode::types::Color::Dark {
        // Round dot, radius = module/2 − 1 px.
        let r = 0.5 - 1.0 / QR_MODULE_PX as f32;
        let (dx, dy) = (fx - 0.5, fy - 0.5);
        return dx * dx + dy * dy <= r * r;
    }
    false
}

/// Stamp the passkey icon onto a cleared patch in the centre of the code. The
/// patch is a whole number of modules, so it snaps to the module grid.
fn draw_center_icon(img: &mut egui::ColorImage, n: usize) {
    const IW: usize = 100;
    const IH: usize = 100;
    let box_px = QR_CENTER_MODULES * QR_MODULE_PX;
    let origin = (n - QR_CENTER_MODULES) / 2 + QR_QUIET_MODULES as usize;
    let (px0, py0) = (origin * QR_MODULE_PX, origin * QR_MODULE_PX);
    for iy in 0..box_px {
        for ix in 0..box_px {
            let ink = PASSKEY_ICON[(iy * IH / box_px) * IW + ix * IW / box_px];
            img[(px0 + ix, py0 + iy)] = egui::Color32::from_gray(ink);
        }
    }
}

/// Build the crisp QR texture plus a heavily downscaled copy. The crisp one is
/// drawn with round modules and rounded finders; the downscaled
/// copy is drawn scaled back up with linear filtering, which smears the module
/// pattern beyond recognition while still reading as "the code".
fn make_qr_textures(ctx: &egui::Context, url: &str) -> (egui::TextureHandle, egui::TextureHandle) {
    let code = make_qr_code(url);
    let n = code.width();
    let size = n + QR_QUIET_MODULES as usize * 2;
    let dim = size * QR_MODULE_PX;

    let mut img = egui::ColorImage::new([dim, dim], vec![egui::Color32::WHITE; dim * dim]);
    let samples = (QR_AA * QR_AA) as u32;
    for py in 0..dim {
        for px in 0..dim {
            let mut light = 0u32;
            for sy in 0..QR_AA {
                for sx in 0..QR_AA {
                    let mx = (px as f32 + (sx as f32 + 0.5) / QR_AA as f32) / QR_MODULE_PX as f32
                        - QR_QUIET_MODULES;
                    let my = (py as f32 + (sy as f32 + 0.5) / QR_AA as f32) / QR_MODULE_PX as f32
                        - QR_QUIET_MODULES;
                    if !qr_module_dark(&code, n, mx, my) {
                        light += 255;
                    }
                }
            }
            img[(px, py)] = egui::Color32::from_gray((light / samples) as u8);
        }
    }
    draw_center_icon(&mut img, n);
    let crisp = ctx.load_texture("qr", img.clone(), egui::TextureOptions::LINEAR);

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

    let pal = Palette::for_dark(detect_dark_theme());

    if select {
        return run_select_window(timeout, pal);
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
            .with_inner_size([380.0, 540.0])
            .with_resizable(false)
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top(),
        ..Default::default()
    };
    eframe::run_native(
        "Phone Passkey Bridge",
        options,
        Box::new(move |cc| {
            apply_style(&cc.egui_ctx, &pal);
            let (texture, blurred) = make_qr_textures(&cc.egui_ctx, &url);
            Ok(Box::new(QrWindow {
                rp,
                texture,
                blurred,
                phone_found,
                start: Instant::now(),
                timeout,
                pal,
            }))
        }),
    )
}

fn run_select_window(timeout: Duration, pal: Palette) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Phone Passkey Bridge")
            .with_app_id("ucabled")
            .with_inner_size([380.0, 320.0])
            .with_resizable(false)
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top(),
        ..Default::default()
    };
    eframe::run_native(
        "Phone Passkey Bridge",
        options,
        Box::new(move |cc| {
            apply_style(&cc.egui_ctx, &pal);
            Ok(Box::new(SelectWindow {
                start: Instant::now(),
                timeout,
                pal,
            }))
        }),
    )
}
