//! Types both binaries must agree on. The API mints a ticket; the SFU
//! consumes it. ICE URLs for browsers are parsed once, here, and returned
//! on the ticket. The SFU is ICE-lite and does not apply them. Both key
//! their per-client limits by the same client address.

pub mod client_addr;
pub mod ice;
pub mod ticket;
