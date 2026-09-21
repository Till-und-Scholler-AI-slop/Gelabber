//! Types both binaries must agree on. The API mints a ticket; the SFU
//! consumes it. ICE URLs for browsers are parsed once, here, and returned
//! on the ticket. The SFU is ICE-lite and does not apply them.

pub mod ice;
pub mod ticket;
