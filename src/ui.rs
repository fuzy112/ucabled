use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How the daemon notifies the user about an ongoing transaction.
#[derive(Clone)]
pub enum Notifier {
    Terminal,
    Gui(GuiHandle),
}

/// Shows the QR code in a short-lived helper process (`ucabled-qr`), spawned
/// per transaction. The helper exits on its own when the user cancels
/// (Cancel button or closing the window); a watcher thread turns that into a
/// cancel signal. `hide()` kills the helper when the transaction ends.
#[derive(Clone)]
pub struct GuiHandle {
    current: Arc<Mutex<Option<Child>>>,
    cancel_tx: Arc<tokio::sync::mpsc::UnboundedSender<()>>,
}

impl GuiHandle {
    pub fn new(cancel_tx: tokio::sync::mpsc::UnboundedSender<()>) -> Self {
        Self {
            current: Arc::new(Mutex::new(None)),
            cancel_tx: Arc::new(cancel_tx),
        }
    }

    fn helper_path() -> std::path::PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("ucabled-qr")))
            .filter(|p| p.exists())
            .unwrap_or_else(|| "ucabled-qr".into())
    }
}

impl Notifier {
    pub fn show(&self, url: &str, rp: Option<String>, timeout_secs: u64) {
        match self {
            Notifier::Terminal => print_qr_terminal(url),
            Notifier::Gui(handle) => handle.show(url, rp, timeout_secs),
        }
    }

    /// The phone has been seen over BLE; the user now confirms on it.
    pub fn phone_found(&self) {
        match self {
            Notifier::Terminal => println!("=== phone detected, confirm on your phone ===\n"),
            Notifier::Gui(handle) => handle.phone_found(),
        }
    }

    pub fn hide(&self) {
        match self {
            Notifier::Terminal => println!("=== transaction finished ===\n"),
            Notifier::Gui(handle) => handle.hide(),
        }
    }
}

impl GuiHandle {
    fn show(&self, url: &str, rp: Option<String>, timeout_secs: u64) {
        self.hide();

        let mut cmd = Command::new(Self::helper_path());
        cmd.arg(url);
        if let Some(rp) = &rp {
            cmd.arg(rp);
        }
        cmd.arg("--timeout").arg(timeout_secs.to_string());
        // The helper reads status lines from stdin ("found" once the phone's
        // BLE advert is seen) and updates its label.
        cmd.stdin(Stdio::piped());
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("ucabled-qr unavailable ({e}), falling back to terminal QR");
                print_qr_terminal(url);
                return;
            }
        };

        {
            let mut guard = self.current.lock().unwrap();
            *guard = Some(child);
        }

        let current = self.current.clone();
        let cancel_tx = self.cancel_tx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            let mut guard = current.lock().unwrap();
            match guard.as_mut() {
                None => break,
                Some(child) => match child.try_wait() {
                    Ok(Some(_)) => {
                        *guard = None;
                        let _ = cancel_tx.send(());
                        break;
                    }
                    Ok(None) => {}
                    Err(_) => break,
                },
            }
        });
    }

    fn hide(&self) {
        let mut guard = self.current.lock().unwrap();
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn phone_found(&self) {
        let mut guard = self.current.lock().unwrap();
        if let Some(child) = guard.as_mut() {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(b"found\n");
                let _ = stdin.flush();
            }
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
        Err(e) => format!("QR render failed: {e}\n{url}"),
    }
}

fn print_qr_terminal(url: &str) {
    println!("\n=== Scan with your phone (passkey) ===");
    print!("{}", qr_unicode(url));
    println!("=======================================\n");
}
