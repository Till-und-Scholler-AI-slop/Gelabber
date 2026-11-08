//! The whole egui Style from the active theme, so nothing of egui's own
//! look shows through: Inter everywhere, the theme's colours on every
//! widget state, no item spacing (screens lay out like CSS boxes), the
//! browser's caret, selection and thin scrollbars.

use egui::style::{
    HandleShape, Interaction, ScrollStyle, Selection, Spacing, TextCursorStyle, WidgetVisuals,
    Widgets,
};
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle as EguiTextStyle, Visuals,
    vec2,
};

use crate::theme::{Theme, corner, mix};

/// Chromium's default selection highlight (Linux, both colour schemes).
const SELECTION: Color32 = Color32::from_rgb(0x33, 0x90, 0xff);

/// How glyph coverage becomes alpha. Calibrated against Chromium/Skia
/// (`--disable-lcd-text`, no hinting) on the auth pages.
fn text_transfer(theme: &Theme) -> egui::epaint::FontColorTransferFunction {
    if theme.is_dark() {
        egui::epaint::FontColorTransferFunction::Gamma(0.9)
    } else {
        egui::epaint::FontColorTransferFunction::Gamma(1.0)
    }
}

pub fn apply(ctx: &egui::Context, theme: &Theme) {
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (
                EguiTextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (
                EguiTextStyle::Body,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                EguiTextStyle::Button,
                FontId::new(13.0, FontFamily::Proportional),
            ),
            (
                EguiTextStyle::Heading,
                FontId::new(20.0, FontFamily::Proportional),
            ),
            (
                EguiTextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into();
        style.wrap_mode = Some(egui::TextWrapMode::Wrap);
        style.animation_time = crate::theme::motion::STANDARD;
        style.explanation_tooltips = false;
        style.url_in_tooltip = false;
        style.always_scroll_the_only_direction = true;
        style.spacing = spacing();
        style.interaction = Interaction {
            selectable_labels: false,
            multi_widget_text_select: false,
            tooltip_delay: 0.2,
            tooltip_grace_time: 0.4,
            show_tooltips_only_when_still: false,
            ..style.interaction.clone()
        };
        style.visuals = visuals(theme);
    });
}

fn spacing() -> Spacing {
    Spacing {
        item_spacing: vec2(0.0, 0.0),
        window_margin: Margin::same(24),
        button_padding: vec2(16.0, 0.0),
        menu_margin: Margin::same(8),
        indent: 16.0,
        interact_size: vec2(36.0, 36.0),
        slider_width: 160.0,
        slider_rail_height: 4.0,
        combo_width: 160.0,
        text_edit_width: 280.0,
        icon_width: 20.0,
        icon_width_inner: 12.0,
        icon_spacing: 12.0,
        tooltip_width: 260.0,
        menu_width: 180.0,
        menu_spacing: 0.0,
        combo_height: 360.0,
        scroll: scrollbars(),
        ..Spacing::default()
    }
}

/// `scrollbar-width: thin`: a rounded 7px thumb in an 11px gutter, hidden
/// until it is needed, as the overlay scrollbars of Firefox and GTK: it
/// shows under the pointer while the area scrolls (`scroll_activity`) and
/// while the pointer is on the bar itself. The gutter stays reserved, so
/// nothing moves when it shows.
fn scrollbars() -> ScrollStyle {
    ScrollStyle {
        floating: true,
        bar_width: 7.0,
        floating_width: 7.0,
        floating_allocated_width: 11.0,
        handle_min_length: 24.0,
        bar_inner_margin: 2.0,
        bar_outer_margin: 2.0,
        foreground_color: false,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.0,
        dormant_handle_opacity: 0.0,
        active_handle_opacity: 0.0,
        interact_handle_opacity: 1.0,
        ..ScrollStyle::solid()
    }
}

/// How long a scrollbar stays after the last scroll, then how long it
/// fades out (seconds).
const SCROLLBAR_LINGER: f64 = 1.0;
const SCROLLBAR_FADE: f64 = 0.3;

/// Once per frame, before the screens: a scroll (wheel or touchpad) shows
/// the scrollbar of the area under the pointer, which fades out a second
/// after the last one.
pub fn scroll_activity(ctx: &egui::Context) {
    let since = f64::from(ctx.input(|i| i.time_since_last_scroll()));
    let opacity = (1.0 - (since - SCROLLBAR_LINGER) / SCROLLBAR_FADE).clamp(0.0, 1.0) as f32;
    if ctx.global_style().spacing.scroll.active_handle_opacity != opacity {
        ctx.all_styles_mut(|s| s.spacing.scroll.active_handle_opacity = opacity);
    }
    if since < SCROLLBAR_LINGER {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(SCROLLBAR_LINGER - since));
    } else if opacity > 0.0 {
        ctx.request_repaint();
    }
}

fn widget(fill: Color32, stroke: Color32, fg: Color32, radius: CornerRadius) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, stroke),
        corner_radius: radius,
        fg_stroke: Stroke::new(1.0, fg),
        expansion: 0.0,
    }
}

fn visuals(theme: &Theme) -> Visuals {
    let c = &theme.c;
    let small = corner(theme.radius.sm);
    let base = if theme.is_dark() {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    // Scroll thumbs use the widget fills: the scrollbar colours of
    // Chromium's dark and light schemes.
    let thumb = mix(c.bg, c.text, if theme.is_dark() { 0.35 } else { 0.4 });
    let thumb_hover = mix(c.bg, c.text, 0.5);
    let thumb_active = mix(c.bg, c.text, 0.6);
    Visuals {
        dark_mode: theme.is_dark(),
        text_options: egui::epaint::text::TextOptions {
            font_hinting: false,
            subpixel_binning: true,
            color_transfer_function: text_transfer(theme),
            ..base.text_options
        },
        override_text_color: None,
        weak_text_alpha: 1.0,
        weak_text_color: Some(c.muted),
        widgets: Widgets {
            noninteractive: widget(c.panel, c.border, c.text, small),
            inactive: widget(thumb, c.border_strong, c.text, small),
            hovered: widget(thumb_hover, c.border_strong, c.text, small),
            active: widget(thumb_active, c.focus, c.text, small),
            open: widget(c.selected, c.border_strong, c.text, small),
        },
        selection: Selection {
            bg_fill: SELECTION.gamma_multiply(0.8),
            stroke: Stroke::new(1.0, Color32::WHITE),
        },
        hyperlink_color: c.accent_text,
        faint_bg_color: c.surface,
        extreme_bg_color: c.bg,
        text_edit_bg_color: Some(c.bg),
        code_bg_color: c.surface,
        warn_fg_color: c.warning,
        error_fg_color: c.danger,
        window_corner_radius: corner(theme.radius.xl),
        window_shadow: theme.shadow_2,
        window_fill: c.elevated,
        window_stroke: Stroke::new(1.0, c.border),
        window_highlight_topmost: false,
        menu_corner_radius: corner(theme.radius.lg),
        panel_fill: c.bg,
        popup_shadow: theme.shadow_1,
        resize_corner_size: 12.0,
        text_cursor: TextCursorStyle {
            stroke: Stroke::new(1.0, c.text),
            preview: false,
            blink: true,
            on_duration: 0.5,
            off_duration: 0.5,
        },
        button_frame: true,
        collapsing_header_frame: false,
        indent_has_left_vline: false,
        striped: false,
        slider_trailing_fill: true,
        handle_shape: HandleShape::Circle,
        interact_cursor: Some(egui::CursorIcon::PointingHand),
        image_loading_spinners: false,
        disabled_alpha: 0.5,
        ..base
    }
}
