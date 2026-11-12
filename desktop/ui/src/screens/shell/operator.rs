//! `/betrieb` in the app (#517): the operator area is the web's only, so
//! the page is a card that opens it in the system browser, and the user
//! menu's "Betrieb" opens it there directly.

use egui::{Rect, Ui, pos2};
use gelabber_i18n::operator as texts;

use crate::app::AppCtx;
use crate::screens::auth::layout;
use crate::theme::space;
use crate::widgets::Button;

/// Where `/betrieb` and the user menu's "Betrieb" lead
/// (`DEFAULT_OPERATOR_PAGE` in web/src/operator/areas.ts).
pub const DEFAULT_PAGE: &str = "server";

/// The area's pages (`OPERATOR_PAGES` in web/src/operator/areas.ts).
pub const PAGES: [&str; 5] = ["einladungen", "konten", "server", "meldungen", "auslastung"];

/// `isOperatorPage`: `/betrieb` itself, or one of [`PAGES`]; any other
/// page is "not found", as on the web.
pub fn is_page(page: Option<&str>) -> bool {
    page.is_none_or(|page| PAGES.contains(&page))
}

/// Opens `path` of the server's web client (e.g. `/betrieb/server`) in the
/// system browser; a failure is only logged, as for a chat link.
pub fn open_in_browser(app: &AppCtx, path: &str) {
    crate::platform::open_url_logged(&app.client.http().url(path));
}

/// The hand-off card in the page column: `.lr-auth-card mx-auto my-10
/// w-full max-w-sm`, its heading and a full-width button in
/// `.auth-actions`.
pub fn show(ui: &mut Ui, app: &mut AppCtx, page: Rect, path: &str) {
    let width = page.width().min(384.0);
    let min = pos2(page.center().x - width / 2.0, page.top() + space::S10);
    layout::card_at(ui, app, min, width, |ui, app| {
        layout::heading(ui, texts::handoff_title());
        ui.add_space(space::S6);
        if ui
            .add(Button::new(texts::handoff_open()).primary().fill())
            .clicked()
        {
            open_in_browser(app, path);
        }
    });
}

#[cfg(test)]
mod tests {
    /// `/betrieb` and its five pages have the hand-off card; any other
    /// page is "not found", as `requireOperator` answers on the web.
    #[test]
    fn only_the_areas_pages_are_known() {
        assert!(super::is_page(None));
        for page in super::PAGES {
            assert!(super::is_page(Some(page)), "{page}");
        }
        for page in ["unbekannt", "Server", "settings", ""] {
            assert!(!super::is_page(Some(page)), "{page}");
        }
    }
}
