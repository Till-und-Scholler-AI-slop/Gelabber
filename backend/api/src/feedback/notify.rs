//! Every mail of the feedback package. Each send runs after the response,
//! in a spawned task, and never holds up the route that triggers it.
//!
//! Not built yet: these send nothing until the mails land (#542,
//! feedback-3).

use uuid::Uuid;

use crate::state::AppState;

/// A new entry reached the operators (created, or a visitor's confirmed).
pub fn new_entry(_state: &AppState, _feedback_id: Uuid) {}

/// The sender replied in the thread of `feedback_id`.
pub fn person_reply(_state: &AppState, _feedback_id: Uuid) {}

/// An operator replied with `message_id`. `first_unread` is true when the
/// reply set the sender's unread mark, so one mail goes out per unread
/// stretch.
pub fn operator_reply(_state: &AppState, _message_id: Uuid, _first_unread: bool) {}
