//! Focus goes back to what opened a dialog once it closes (Modal.tsx
//! `restoreFocus`, ImageLightbox.tsx): egui does not focus a clicked
//! button, so the opener is the widget clicked (or focused) as the dialog
//! first shows.

use egui::{Context, Id, LayerId};

#[derive(Clone, Copy)]
struct Open {
    layer: LayerId,
    opener: Option<Id>,
    frame: u64,
}

fn key() -> Id {
    Id::unique("gelabber-dialog-openers")
}

fn clicked_key() -> Id {
    Id::unique("gelabber-last-clicked")
}

/// Called by a dialog every frame it is drawn on `layer`.
pub fn dialog_shown(ctx: &Context, layer: LayerId) {
    let frame = ctx.cumulative_frame_nr();
    let known = ctx.data(|d| {
        d.get_temp::<Vec<Open>>(key())
            .is_some_and(|v| v.iter().any(|o| o.layer == layer))
    });
    // A dialog asked for in a click handler may first show a frame later.
    let opener = (!known).then(|| {
        let last = ctx
            .data(|d| d.get_temp::<(Id, u64)>(clicked_key()))
            .filter(|(_, f)| f + 1 >= frame)
            .map(|(id, _)| id);
        ctx.interaction_snapshot(|s| s.clicked)
            .or(last)
            .or_else(|| ctx.memory(|m| m.focused()))
    });
    ctx.data_mut(|d| {
        let open = d.get_temp_mut_or_default::<Vec<Open>>(key());
        match open.iter_mut().find(|o| o.layer == layer) {
            Some(o) => o.frame = frame,
            None => open.push(Open {
                layer,
                opener: opener.flatten(),
                frame,
            }),
        }
    });
}

/// At the end of a frame: a dialog not drawn in it has closed, and focus
/// left inside it goes back to its opener. Closed by a key, the opener
/// shows its ring, as `:focus-visible` does after a keyboard close.
pub fn restore(ctx: &Context) {
    let frame = ctx.cumulative_frame_nr();
    if let Some(id) = ctx.interaction_snapshot(|s| s.clicked) {
        ctx.data_mut(|d| d.insert_temp(clicked_key(), (id, frame)));
    }
    // The last input's kind: the close itself shows only a frame later.
    let (pressed, pointer) = ctx.input(|i| {
        (
            i.raw
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { pressed: true, .. })),
            i.pointer.any_pressed(),
        )
    });
    let by_key_id = Id::unique("gelabber-last-input-key");
    if pressed || pointer {
        ctx.data_mut(|d| d.insert_temp(by_key_id, pressed && !pointer));
    }
    let by_key = ctx.data(|d| d.get_temp::<bool>(by_key_id)).unwrap_or(false);
    let closed: Vec<Open> = ctx.data_mut(|d| {
        let open = d.get_temp_mut_or_default::<Vec<Open>>(key());
        let (gone, kept) = open.drain(..).partition(|o| o.frame < frame);
        *open = kept;
        gone
    });
    // Input may have closed a dialog drawn this frame: one more frame
    // notices it is gone.
    if pressed || pointer || ctx.input(|i| i.pointer.any_released()) {
        let any_open = ctx.data(|d| {
            d.get_temp::<Vec<Open>>(key())
                .is_some_and(|v| !v.is_empty())
        });
        if any_open {
            ctx.request_repaint();
        }
    }
    // The newest first: a stacked question hands focus back before the
    // dialog under it would.
    for o in closed.iter().rev() {
        let Some(opener) = o.opener else { continue };
        let focus_left_with = match ctx.memory(|m| m.focused()) {
            None => true,
            Some(id) => ctx.read_response(id).is_none_or(|r| r.layer_id == o.layer),
        };
        if !focus_left_with || ctx.read_response(opener).is_none() {
            continue;
        }
        ctx.memory_mut(|m| m.request_focus(opener));
        if by_key {
            super::set_keyboard_focus(ctx, true);
        }
        ctx.request_repaint();
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_goes_back_to_the_opener() {
        let ctx = Context::default();
        let opener = Id::unique("opener");
        let inside = Id::unique("inside");
        let dialog = Id::unique("dialog");
        let layer = egui::Area::new(dialog).layer();
        // One frame: the dialog drawn or not, and a widget asking for focus.
        let frame = |open: bool, focus: Option<Id>| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(10.0, 10.0));
                let button = ui.interact(rect, opener, egui::Sense::click());
                if focus == Some(opener) {
                    button.request_focus();
                }
                if open {
                    dialog_shown(ui.ctx(), layer);
                    egui::Area::new(dialog).show(ui.ctx(), |ui| {
                        let r = ui.interact(rect, inside, egui::Sense::click());
                        if focus == Some(inside) {
                            r.request_focus();
                        }
                    });
                }
                restore(ui.ctx());
            });
            output.textures_delta.clear();
            ctx.memory(|m| m.focused())
        };
        assert_eq!(frame(false, Some(opener)), Some(opener));
        // Opened from the focused button; focus moves into the dialog.
        assert_eq!(frame(true, Some(inside)), Some(inside));
        assert_eq!(frame(true, None), Some(inside));
        // The first frame without the dialog hands focus back.
        assert_eq!(frame(false, None), Some(opener));
    }
}
