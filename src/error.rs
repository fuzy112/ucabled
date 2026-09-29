// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt;

/// CTAP status bytes used when a caBLE transaction fails.
pub const CTAP1_ERR_TIMEOUT: u8 = 0x05;
pub const CTAP2_ERR_KEEPALIVE_CANCEL: u8 = 0x2d;
pub const CTAP2_ERR_NO_CREDENTIALS: u8 = 0x2e;

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
            Self::Failed(_) => CTAP2_ERR_NO_CREDENTIALS,
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
        assert_eq!(TransactionError::transport(anyhow::anyhow!("x")).ctap_status(), 0x05);
        assert_eq!(TransactionError::failed(anyhow::anyhow!("x")).ctap_status(), 0x2e);
    }
}
