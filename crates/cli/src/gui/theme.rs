//! Visual system for the Lokalized desktop UI.

use egui::{
    self, Button, Color32, CornerRadius, FontId, Frame, Margin, Response, RichText, Stroke, Style,
    TextStyle, Ui, Visuals,
};

// ─── Palette ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub input: Color32,
    pub border: Color32,
    #[allow(dead_code)]
    pub border_focus: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_muted: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub missing: Color32,
    pub missing_surface: Color32,
    pub unused: Color32,
    pub unused_surface: Color32,
    pub success: Color32,
    pub success_surface: Color32,
    pub danger: Color32,
    pub danger_surface: Color32,
    pub selection: Color32,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            bg: Color32::from_rgb(9, 10, 12),
            panel: Color32::from_rgb(12, 13, 17),
            sidebar: Color32::from_rgb(14, 15, 20),
            surface: Color32::from_rgb(20, 22, 28),
            surface_hover: Color32::from_rgb(28, 31, 40),
            input: Color32::from_rgb(16, 18, 24),
            border: Color32::from_rgba_premultiplied(255, 255, 255, 18),
            border_focus: Color32::from_rgba_premultiplied(124, 108, 250, 140),
            accent: Color32::from_rgb(124, 108, 250),
            accent_soft: Color32::from_rgb(96, 86, 200),
            accent_muted: Color32::from_rgba_premultiplied(124, 108, 250, 40),
            text: Color32::from_rgb(245, 246, 250),
            text_secondary: Color32::from_rgb(196, 200, 214),
            text_muted: Color32::from_rgb(118, 124, 142),
            missing: Color32::from_rgb(251, 146, 120),
            missing_surface: Color32::from_rgba_premultiplied(251, 146, 120, 28),
            unused: Color32::from_rgb(156, 163, 184),
            unused_surface: Color32::from_rgba_premultiplied(156, 163, 184, 22),
            success: Color32::from_rgb(74, 222, 168),
            success_surface: Color32::from_rgba_premultiplied(74, 222, 168, 24),
            danger: Color32::from_rgb(248, 113, 113),
            danger_surface: Color32::from_rgba_premultiplied(248, 113, 113, 28),
            selection: Color32::from_rgba_premultiplied(124, 108, 250, 55),
        }
    }
}

pub fn apply(ctx: &egui::Context) -> Palette {
    let p = Palette::default();
    let mut visuals = Visuals::dark();
    visuals.dark_mode = true;
    visuals.panel_fill = p.panel;
    visuals.window_fill = p.bg;
    visuals.extreme_bg_color = p.bg;
    visuals.faint_bg_color = p.surface;
    visuals.hyperlink_color = p.accent;
    visuals.warn_fg_color = p.missing;
    visuals.error_fg_color = p.danger;
    visuals.override_text_color = Some(p.text);

    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = Color32::TRANSPARENT;
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text_muted);
    w.inactive.bg_fill = p.surface;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.hovered.bg_fill = p.surface_hover;
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.bg_stroke = Stroke::new(1.0, p.border);
    w.active.bg_fill = p.accent_soft;
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.open.bg_fill = p.surface_hover;

    visuals.selection.bg_fill = p.selection;
    visuals.selection.stroke = Stroke::new(1.0, p.accent.linear_multiply(0.6));
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.collapsing_header_frame = false;

    ctx.set_visuals(visuals);

    let mut style = Style::default();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.indent = 18.0;
    style.spacing.interact_size = egui::vec2(36.0, 28.0);
    style.spacing.window_margin = Margin::same(16);
    style.visuals.text_cursor.stroke.color = p.accent;
    style
        .text_styles
        .insert(TextStyle::Heading, egui::FontId::proportional(22.0));
    style
        .text_styles
        .insert(TextStyle::Body, egui::FontId::proportional(13.5));
    style
        .text_styles
        .insert(TextStyle::Button, egui::FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Small, egui::FontId::proportional(11.5));
    style
        .text_styles
        .insert(TextStyle::Monospace, egui::FontId::monospace(12.5));
    ctx.set_style(style);

    p
}

// ─── Frames ──────────────────────────────────────────────────────────────────

pub fn panel_frame(p: Palette) -> Frame {
    Frame::new().fill(p.panel).inner_margin(Margin::ZERO)
}

pub fn sidebar_frame(p: Palette) -> Frame {
    Frame::new()
        .fill(p.sidebar)
        .stroke(Stroke::new(1.0, p.border))
        .inner_margin(Margin::symmetric(14, 16))
}

pub fn content_frame(p: Palette) -> Frame {
    Frame::new()
        .fill(p.bg)
        .inner_margin(Margin::symmetric(20, 18))
}

pub fn card(p: Palette) -> Frame {
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(18))
}

pub fn inset_card(p: Palette) -> Frame {
    Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 12))
}

// ─── Typography ──────────────────────────────────────────────────────────────

pub fn title(text: &str, p: Palette) -> RichText {
    RichText::new(text).strong().size(20.0).color(p.text)
}

pub fn label(text: &str, p: Palette) -> RichText {
    RichText::new(text).size(11.0).color(p.text_muted)
}

// ─── Widgets ─────────────────────────────────────────────────────────────────

pub fn stat_chip(ui: &mut Ui, value: &str, label: &str, color: Color32, p: Palette) {
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(value).strong().size(15.0).color(color));
                ui.label(RichText::new(label).size(10.0).color(p.text_muted));
            });
        });
}

pub fn badge(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(fg));
        });
}

pub fn primary_button(ui: &mut Ui, label: &str, p: Palette) -> Response {
    let text = RichText::new(label).color(Color32::WHITE).size(13.0);
    ui.add(
        Button::new(text)
            .fill(p.accent)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(8)),
    )
}

pub fn secondary_button(ui: &mut Ui, label: &str, p: Palette) -> Response {
    ui.add(
        Button::new(RichText::new(label).size(13.0).color(p.text_secondary))
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(8)),
    )
}

pub fn ghost_button(ui: &mut Ui, label: &str, p: Palette) -> Response {
    ui.add(
        Button::new(RichText::new(label).size(12.5).color(p.text_muted))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(6)),
    )
}

pub fn danger_button(ui: &mut Ui, label: &str, p: Palette) -> Response {
    ui.add(
        Button::new(RichText::new(label).size(13.0).color(p.danger))
            .fill(p.danger_surface)
            .stroke(Stroke::new(1.0, p.danger.linear_multiply(0.35)))
            .corner_radius(CornerRadius::same(8)),
    )
}

pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str, p: Palette) -> Response {
    Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("⌕").size(14.0).color(p.text_muted));
                ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(hint)
                        .frame(false)
                        .margin(Margin::ZERO)
                        .desired_width(f32::INFINITY)
                        .text_color(p.text),
                )
            });
        })
        .response
}

pub fn multiline_field(
    ui: &mut Ui,
    text: &mut String,
    hint: &str,
    rows: usize,
    p: Palette,
) -> Response {
    Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(text)
                    .hint_text(hint)
                    .frame(false)
                    .desired_width(f32::INFINITY)
                    .desired_rows(rows)
                    .text_color(p.text),
            )
        })
        .inner
}

pub fn singleline_field(ui: &mut Ui, text: &mut String, hint: &str, p: Palette) -> Response {
    Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .frame(false)
                    .desired_width(f32::INFINITY)
                    .text_color(p.text),
            )
        })
        .inner
}

pub fn field_label(ui: &mut Ui, text: &str, p: Palette) {
    ui.label(label(text, p));
    ui.add_space(4.0);
}

pub fn divider(ui: &mut Ui, p: Palette) {
    let rect = ui.available_rect_before_wrap();
    let y = rect.top() + 6.0;
    ui.painter()
        .hline(rect.left()..=rect.right(), y, Stroke::new(1.0, p.border));
    ui.add_space(12.0);
}

pub fn empty_state(ui: &mut Ui, title: &str, hint: &str, p: Palette) {
    card(p).show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            ui.label(
                RichText::new("◆")
                    .size(32.0)
                    .color(p.accent_muted.linear_multiply(2.5)),
            );
            ui.add_space(16.0);
            ui.label(
                RichText::new(title)
                    .size(16.0)
                    .strong()
                    .color(p.text_secondary),
            );
            ui.add_space(6.0);
            ui.label(RichText::new(hint).size(13.0).color(p.text_muted));
            ui.add_space(48.0);
        });
    });
}

/// Breadcrumb-style key path: `common` › `actions` › `save`
pub fn key_breadcrumb(ui: &mut Ui, key: &str, p: Palette) {
    let parts: Vec<&str> = key.split('.').collect();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new("›").color(p.text_muted).size(12.0));
            }
            let is_last = i == parts.len() - 1;
            ui.label(
                RichText::new(*part)
                    .font(FontId::monospace(if is_last { 13.5 } else { 12.0 }))
                    .size(if is_last { 15.0 } else { 13.0 })
                    .strong()
                    .color(if is_last { p.text } else { p.text_muted }),
            );
        }
    });
}
