// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt;

use crate::ctap::{CTAP1_ERR_TIMEOUT, CTAP2_ERR_KEEPALIVE_CANCEL, CTAP2_ERR_NOT_ALLOWED};

/// Why a caBLE transaction failed, in terms the CTAP host understands.
#[derive(Debug)]
pub enum TransactionError {
    /// The user cancelled (QR window closed or a CANCEL command).
    Cancelled,
    /// The phone never showed up in time (BLE advert or tunnel).
    Timeout,
    /// Transport failure on the way to the phone.
    Transport(anyhow::Error),
    /// The transaction reached the phone but the protocol failed.
    Failed(anyhow::Error),
}

impl TransactionError {
    pub fn transport(e: impl Into<anyhow::Error>) -> Self {
        Self::Transport(e.into())
    }

    pub fn failed(e: impl Into<anyhow::Error>) -> Self {
        Self::Failed(e.into())
    }

    /// CTAP status byte to report to the host.
    pub fn ctap_status(&self) -> u8 {
        match self {
            Self::Cancelled => CTAP2_ERR_KEEPALIVE_CANCEL,
            Self::Timeout | Self::Transport(_) => CTAP1_ERR_TIMEOUT,
            // NOT_ALLOWED, not NO_CREDENTIALS: a failed Noise handshake
            // (wrong phone, tunnel tampering) or a protocol violation means
            // this attempt failed, not that the phone has no passkey for the
            // site; only a phone-reported 0x2e keeps that meaning.  This is
            // protocol accuracy, not a browser-visible signal: Firefox 158
            // maps 0x32, 0x2e, 0x2d, 0x05 and 0x7f to the same generic
            // NotAllowedError and does not retry another authenticator on
            // its own.
            Self::Failed(_) => CTAP2_ERR_NOT_ALLOWED,
        }
    }

    /// Whether the failure was a refused tunnel redirect.  Unlike timeouts
    /// and user cancels — routine outcomes — this points at a misbehaving
    /// or hostile tunnel server, so the daemon surfaces it to the user.
    pub fn is_redirect_refused(&self) -> bool {
        match self {
            Self::Transport(e) | Self::Failed(e) => e
                .chain()
                .any(|cause| cause.is::<crate::tunnel::RedirectRefused>()),
            _ => false,
        }
    }
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "cancelled by user"),
            Self::Timeout => write!(f, "timed out waiting for the phone"),
            Self::Transport(e) => write!(f, "transport error: {e:#}"),
            Self::Failed(e) => write!(f, "transaction failed: {e:#}"),
        }
    }
}

impl std::error::Error for TransactionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(e) | Self::Failed(e) => Some(e.as_ref()),
            Self::Cancelled | Self::Timeout => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping() {
        assert_eq!(TransactionError::Cancelled.ctap_status(), 0x2d);
        assert_eq!(TransactionError::Timeout.ctap_status(), 0x05);
        assert_eq!(
            TransactionError::transport(anyhow::anyhow!("x")).ctap_status(),
            0x05
        );
        assert_eq!(
            TransactionError::failed(anyhow::anyhow!("x")).ctap_status(),
            0x32
        );
    }

    #[test]
    fn session_failures_are_not_allowed() {
        // The session boundary reports a failed handshake or an unexpected
        // tunnel message as Failed; the host must see NOT_ALLOWED, not
        // NO_CREDENTIALS, so it does not treat the attempt as "no passkey".
        for message in ["caBLE handshake failed", "unexpected message type 9"] {
            let err = TransactionError::failed(anyhow::anyhow!(message));
            assert_eq!(err.ctap_status(), CTAP2_ERR_NOT_ALLOWED);
        }
    }

    #[test]
    fn redirect_refused_detection() {
        let refused =
            TransactionError::transport(crate::tunnel::RedirectRefused("test".to_string()));
        assert!(refused.is_redirect_refused());
        assert!(!TransactionError::Timeout.is_redirect_refused());
        assert!(!TransactionError::Cancelled.is_redirect_refused());
        assert!(!TransactionError::transport(anyhow::anyhow!("x")).is_redirect_refused());
    }
}
