//! Closed SFU failures. The WebSocket adapter maps [`SfuError::code`] onto
//! `ServerFrame::Err`. Native detail stays on [`Display`] for `warn!`.

use std::fmt;

#[derive(Debug)]
pub enum SfuError {
    NotInRoom,
    BadAnnounce,
    /// Screen source announced without the ticket's go_live bit.
    Forbidden,
    ProtocolVersion,
    UnsupportedCodec,
    /// Native worker/listener is unavailable.
    Unavailable,
    /// Authority revoked: member/channel generation or session mismatch,
    /// expiry, or peer removed.
    Revoked,
    Negotiation(String),
    Ice(String),
}

impl SfuError {
    pub fn negotiation(err: impl fmt::Display) -> Self {
        Self::Negotiation(err.to_string())
    }

    pub fn ice(err: impl fmt::Display) -> Self {
        Self::Ice(err.to_string())
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::NotInRoom | Self::BadAnnounce => "bad_request",
            Self::Forbidden => "forbidden",
            Self::ProtocolVersion => "unsupported_media_version",
            Self::UnsupportedCodec => "unsupported_codec",
            Self::Unavailable => "unavailable",
            Self::Revoked => "unauthorized",
            Self::Negotiation(_) => "negotiation_failed",
            Self::Ice(_) => "ice_failed",
        }
    }
}

impl fmt::Display for SfuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInRoom => write!(f, "not in room"),
            Self::BadAnnounce => write!(f, "bad announce"),
            Self::Forbidden => write!(f, "forbidden"),
            Self::ProtocolVersion => write!(f, "unsupported media protocol version"),
            Self::UnsupportedCodec => write!(f, "unsupported media codec"),
            Self::Unavailable => write!(f, "unavailable"),
            Self::Revoked => write!(f, "revoked"),
            Self::Negotiation(source) | Self::Ice(source) => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for SfuError {}
