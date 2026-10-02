// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::Write;

use crate::agent::AgentClient;

/// How the daemon asks the user to scan the QR code.
///
/// `Agent` routes requests to the per-user session agent over D-Bus; the
/// fallback `Terminal` prints the code on a controlling terminal (used with
/// `--no-ui` or when the UI bridge is unavailable).
#[derive(Clone)]
pub enum Notifier {
    Terminal,
    Agent(AgentClient),
}

/// The UI available for a new transaction, carrying the generation tag to
/// record on the pending operation so a stale agent-gone signal is
/// ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiTag {
    /// Terminal QR: a UI is available, but no agent is involved.
    Terminal,
    /// A session agent, tagged with its registration generation.
    Agent(u64),
}

impl UiTag {
    /// The generation tag recorded on the daemon's pending operations.
    pub fn generation(&self) -> Option<u64> {
        match self {
            UiTag::Terminal => None,
            UiTag::Agent(generation) => Some(*generation),
        }
    }
}

impl Notifier {
    /// The UI for a new transaction, or `None` when none is available.
    /// The agent slot is read exactly once, so the availability check and
    /// the generation tag cannot race with an agent departure (which would
    /// leave the transaction untagged and its agent-gone signal
    /// unmatched).
    pub fn ui_for_transaction(&self) -> Option<UiTag> {
        match self {
            Notifier::Terminal => Some(UiTag::Terminal),
            Notifier::Agent(client) => client.generation().map(UiTag::Agent),
        }
    }

    pub fn show(&self, tid: u64, url: &str, rp: Option<String>, timeout_secs: u64) {
        match self {
            Notifier::Terminal => print_qr_terminal(url),
            Notifier::Agent(client) => client.prompt(tid, url, rp, timeout_secs),
        }
    }

    /// Ask the user to pick the phone when another authenticator (e.g. a
    /// physical security key) is also present. Returns the agent's
    /// registration generation (to tag the pending selection) when the
    /// prompt was shown, `None` when no agent is available and the caller
    /// should decline instead of waiting for an answer.  The generation is
    /// read atomically with the availability check.
    pub fn select(&self, tid: u64) -> Option<u64> {
        match self {
            Notifier::Terminal => None,
            Notifier::Agent(client) => {
                let generation = client.generation()?;
                client.select(tid);
                Some(generation)
            }
        }
    }

    /// The phone has been seen over BLE; the user now confirms on it.
    pub fn phone_found(&self) {
        match self {
            Notifier::Terminal => println!("=== phone detected, confirm on your phone ===\n"),
            Notifier::Agent(client) => client.found(),
        }
    }

    pub fn hide(&self) {
        match self {
            Notifier::Terminal => println!("=== transaction finished ===\n"),
            Notifier::Agent(client) => client.close(),
        }
    }

    /// Show a user-facing notice (desktop notification, or the controlling
    /// terminal).  Only static daemon text may be passed.
    pub fn notify(&self, summary: &str, body: &str) {
        match self {
            Notifier::Terminal => print_notice(summary, body),
            Notifier::Agent(client) => client.notify(summary, body),
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

fn print_notice(summary: &str, body: &str) {
    // Nothing secret here, but stdout may be the journal; prefer the
    // controlling terminal as with the QR code.
    use std::io::IsTerminal;
    let text = format!("\n=== {summary} ===\n{body}\n\n");
    if std::io::stdout().is_terminal() {
        print!("{text}");
    } else if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        let _ = tty.write_all(text.as_bytes());
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
