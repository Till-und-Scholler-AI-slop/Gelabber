//! Voice: a voice channel's page with its room and stage, the call bar,
//! and the pieces the shell's sidebar, member panel and overview take from
//! the call (web voice/, components/Voice*.tsx). Everything reads
//! `gelabber_call::CallState` and the gateway's roster each frame and
//! sends the call's commands.

pub mod call_bar;
pub mod chips;
pub mod diagnostics;
pub mod dialogs;
pub mod hint;
pub mod look;
pub mod page;
pub mod quick;
pub mod session;
pub mod stage;
pub mod summary;
pub mod sync;
pub mod tile;
pub mod video;
pub mod volume;
pub mod window;

pub use page::show;

#[cfg(test)]
mod tests;
