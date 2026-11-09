//! What a text selection copies, the browser's way. egui's
//! `LabelSelectionState` copies a whole paragraph for a selection that
//! is only a caret (a plain click), and copies every character it was
//! given. Here a click selects nothing to copy, neither for Ctrl+C nor
//! for Linux's middle-click buffer, and hidden text (a covered spoiler,
//! `aria-hidden` on the web) stays out of both.

use std::sync::Arc;

use egui::{Context, Galley, OutputCommand, Plugin, Ui};

/// Stands in for each hidden character in the text a selection sees, so
/// cursors keep their places; [`Copying`] takes it out of what is copied.
pub const HIDDEN: char = '\u{FFFF}';

/// Filters the label selection's copy commands at the end of each pass.
#[derive(Default)]
pub struct Copying {
    /// How many commands the pass had sent before `LabelSelectionState`
    /// ran: the ones after are its own.
    before: Option<usize>,
    /// Whether some text showed a selection of at least one character.
    selected: bool,
}

/// Registers [`Copying`] once per context.
pub fn install(ctx: &Context) {
    ctx.add_plugin(Copying::default());
}

/// Notes that `painted` (the galley handed to the selection, before it
/// painted) came out with selected characters: the selection keeps
/// `painted` itself when it has none to show.
pub fn note(ui: &Ui, painted: &Arc<Galley>) {
    let shown = ui.ctx().graphics(|g| {
        g.get(ui.layer_id())
            .and_then(|list| list.all_entries().last())
            .is_some_and(|last| match &last.shape {
                egui::Shape::Text(text) => {
                    Arc::ptr_eq(&text.galley.job, &painted.job)
                        && !Arc::ptr_eq(&text.galley, painted)
                }
                _ => false,
            })
    });
    if shown {
        ui.ctx().with_plugin(|c: &mut Copying| c.selected = true);
    }
}

impl Plugin for Copying {
    fn debug_name(&self) -> &'static str {
        "gelabber::selection::Copying"
    }

    fn setup(&mut self, ctx: &Context) {
        // End-of-pass callbacks run before the plugins added after them,
        // `LabelSelectionState` among them.
        ctx.on_end_pass(
            "gelabber::selection::before",
            Arc::new(|ui: &mut Ui| {
                let sent = ui.ctx().output(|o| o.commands.len());
                ui.ctx()
                    .with_plugin(|c: &mut Copying| c.before = Some(sent));
            }),
        );
    }

    fn on_begin_pass(&mut self, _ui: &mut Ui) {
        self.before = None;
        self.selected = false;
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        let Some(before) = self.before.take() else {
            return;
        };
        let selected = std::mem::take(&mut self.selected);
        ui.ctx().output_mut(|o| {
            let mut index = 0;
            o.commands.retain_mut(|command| {
                let theirs = index >= before;
                index += 1;
                match command {
                    OutputCommand::CopyText(text) | OutputCommand::TextSelectionSettled(text)
                        if theirs =>
                    {
                        text.retain(|c| c != HIDDEN);
                        selected && !text.is_empty()
                    }
                    _ => true,
                }
            });
        });
    }
}
