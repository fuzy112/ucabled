// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::Write;

use crate::agent::UiClient;

/// How the daemon asks the user to scan the QR code.
///
/// `Agent` routes requests to the per-user session agent over D-Bus; the
/// fallback `Terminal` prints the code on a controlling terminal (used with
/// `--no-ui` or when the UI bridge is unavailable).
#[derive(Clone)]
pub enum Notifier {
    Terminal,
    Agent(UiClient),
}

impl Notifier {
    /// Whether a UI is ready to show the QR code.
    pub fn available(&self) -> bool {
        match self {
            Notifier::Terminal => true,
            Notifier::Agent(ui) => ui.available(),
        }
    }

    pub fn show(&self, tid: u64, url: &str, rp: Option<String>, timeout_secs: u64) {
        match self {
            Notifier::Terminal => print_qr_terminal(url),
            Notifier::Agent(ui) => ui.show(tid, url, rp, timeout_secs),
        }
    }

    /// The phone has been seen over BLE; the user now confirms on it.
    pub fn phone_found(&self) {
        match self {
            Notifier::Terminal => println!("=== phone detected, confirm on your phone ===\n"),
            Notifier::Agent(ui) => ui.found(),
        }
    }

    pub fn hide(&self) {
        match self {
            Notifier::Terminal => println!("=== transaction finished ===\n"),
            Notifier::Agent(ui) => ui.close(),
        }
    }
}

pub fn qr_unicode(url: &str) -> String {
    match qrcode::QrCode::new(url.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::unicode::Dense1x2>()
            .dark_color(qrcode::render::unicode::Dense1x2::Dark)
            .light_color(qrcode::render::unicode::Dense1x2::Light)
            .build(),
        // Never embed the URL: it contains the transaction secret.
        Err(e) => format!("QR render failed: {e}"),
    }
}

fn print_qr_terminal(url: &str) {
    use std::io::IsTerminal;
    let body = format!(
        "\n=== Scan with your phone (passkey) ===\n{}\n=======================================\n",
        qr_unicode(url)
    );
    // Under a systemd service stdout is the journal, where the QR code (and
    // the secret it encodes) must not persist. Show it on the controlling
    // terminal when there is one and drop it silently otherwise.
    if std::io::stdout().is_terminal() {
        print!("{body}");
    } else if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        let _ = tty.write_all(body.as_bytes());
    } else {
        tracing::warn!("no terminal available to display the QR code");
    }
}
