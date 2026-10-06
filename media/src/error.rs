//! Closed SFU failures. The WebSocket adapter maps [`SfuError::code`] onto
//! `ServerFrame::Err`. Native detail stays on [`Display`] for `warn!`.

use std::fmt;

#[derive(Debug)]
pub enum SfuError {
    NotInRoom,
    BadAnnounce,
    /// Go Live announced without the ticket bit.
    Forbidden,
    /// Valid Live authority is temporarily unable to publish within its lease fence.
    LiveBusy,
    ProtocolVersion,
    UnsupportedCodec,
    /// Native worker/listener is unavailable.
    Unavailable,
    /// Kick/ban deny is set for this user on this server.
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
            Self::LiveBusy => "live_busy",
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
            Self::LiveBusy => write!(f, "live peer lease busy"),
            Self::ProtocolVersion => write!(f, "unsupported media protocol version"),
            Self::UnsupportedCodec => write!(f, "unsupported media codec"),
            Self::Unavailable => write!(f, "unavailable"),
            Self::Revoked => write!(f, "revoked"),
            Self::Negotiation(source) | Self::Ice(source) => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for SfuError {}
