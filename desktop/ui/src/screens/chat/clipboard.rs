//! Copying text (servers/clipboard.ts `copyText`), which can fail like the
//! browser's: without a display there is no clipboard to write to.

use egui::Id;

use crate::toasts::Toasts;
use crate::widgets::StatusTone;

/// Writes `text` to the clipboard; false when it could not.
pub type Copier = fn(&egui::Context, &str) -> bool;

fn copier_id() -> Id {
    Id::unique("gelabber-copier")
}

/// egui hands the text to the system clipboard of the window.
fn system(ctx: &egui::Context, text: &str) -> bool {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        return false;
    }
    ctx.copy_text(text.to_owned());
    true
}

/// `copyText`: whether the text is on the clipboard now.
pub fn copy(ctx: &egui::Context, text: &str) -> bool {
    let copier = ctx
        .data(|d| d.get_temp::<Copier>(copier_id()))
        .unwrap_or(system);
    copier(ctx, text)
}

/// Replaces the system clipboard (tests).
pub fn set_copier(ctx: &egui::Context, copier: Copier) {
    ctx.data_mut(|d| d.insert_temp(copier_id(), copier));
}

/// "Text kopieren" of a message (MessageRow.tsx): a toast either way.
pub fn copy_message(ctx: &egui::Context, text: &str, toasts: &mut Toasts) {
    if copy(ctx, text) {
        toasts.notify("Text kopiert.");
    } else {
        toasts.error("Der Text konnte nicht kopiert werden.");
    }
}

/// "Link kopieren" of the invite dialog (InviteDialog.tsx): its status
/// line, and whether the link went out.
pub fn copy_link(ctx: &egui::Context, link: &str, summary: &str) -> (String, StatusTone, bool) {
    if copy(ctx, link) {
        (
            format!("Link kopiert · {summary}"),
            StatusTone::Success,
            true,
        )
    } else {
        (
            "Kopieren fehlgeschlagen. Kopiere den Link manuell.".into(),
            StatusTone::Error,
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_clipboard_is_reported() {
        let ctx = egui::Context::default();
        set_copier(&ctx, |_, _| false);
        let mut toasts = Toasts::default();
        copy_message(&ctx, "Hallo", &mut toasts);
        assert_eq!(toasts.texts(), ["Der Text konnte nicht kopiert werden."]);
        assert_eq!(
            copy_link(&ctx, "https://x/invite/abc", "24 Stunden · Unbegrenzt"),
            (
                "Kopieren fehlgeschlagen. Kopiere den Link manuell.".into(),
                StatusTone::Error,
                false
            )
        );
        set_copier(&ctx, |_, text| text == "Hallo");
        let mut toasts = Toasts::default();
        copy_message(&ctx, "Hallo", &mut toasts);
        assert_eq!(toasts.texts(), ["Text kopiert."]);
    }
}
