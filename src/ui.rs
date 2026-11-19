//! Theme and the handful of custom widgets the app is built from.

use std::sync::Arc;

use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, Align, Align2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Galley, Id, Layout, Margin,
    Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, TextEdit, TextFormat, TextWrapMode, Ui, UiBuilder,
};
use egui::epaint::RectShape;
use egui::text::TextWrapping;

pub mod c {
    use super::Color32;
    pub const BG: Color32 = Color32::from_rgb(15, 15, 17);
    pub const SIDEBAR: Color32 = Color32::from_rgb(19, 19, 22);
    pub const SURFACE: Color32 = Color32::from_rgb(24, 24, 28);
    pub const RAISED: Color32 = Color32::from_rgb(34, 34, 39);
    pub const HOVER: Color32 = Color32::from_rgb(42, 42, 48);
    pub const BORDER: Color32 = Color32::from_rgb(44, 44, 51);
    pub const DIVIDER: Color32 = Color32::from_rgb(33, 33, 38);
    pub const SELECT: Color32 = Color32::from_rgb(38, 38, 45);
    pub const TEXT: Color32 = Color32::from_rgb(234, 234, 237);
    pub const MUTED: Color32 = Color32::from_rgb(142, 142, 150);
    pub const FAINT: Color32 = Color32::from_rgb(96, 96, 104);
    pub const ACCENT: Color32 = Color32::from_rgb(206, 174, 116);
    pub const ACCENT_HOVER: Color32 = Color32::from_rgb(222, 192, 136);
    pub const ON_ACCENT: Color32 = Color32::from_rgb(24, 20, 14);
    pub const GOOD: Color32 = Color32::from_rgb(124, 186, 136);
    pub const WARN: Color32 = Color32::from_rgb(220, 168, 88);
    pub const BAD: Color32 = Color32::from_rgb(226, 112, 106);
    pub const CLOSE: Color32 = Color32::from_rgb(196, 43, 28);
}

pub const ROW_H: f32 = 38.0;
pub const PAD_X: f32 = 44.0;

pub fn body() -> FontId {
    FontId::new(14.0, FontFamily::Proportional)
}
pub fn small() -> FontId {
    FontId::new(12.5, FontFamily::Proportional)
}
pub fn strong() -> FontId {
    FontId::new(14.0, FontFamily::Name("sb".into()))
}
pub fn btn_font() -> FontId {
    FontId::new(13.0, FontFamily::Name("sb".into()))
}
pub fn title_font() -> FontId {
    FontId::new(26.0, FontFamily::Name("sb".into()))
}

/// Loads Segoe UI (regular + semibold) from Windows and sets up the dark style.
pub fn setup(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::empty();
    let dir = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap_or("C:\\Windows".into())).join("Fonts");
    let load = |f: &str| std::fs::read(dir.join(f)).ok();
    let regular = load("segoeui.ttf").or_else(|| load("arial.ttf"));
    let semibold = load("seguisb.ttf").or_else(|| load("segoeuib.ttf"));
    let serif = load("pala.ttf").or_else(|| load("constan.ttf")).or_else(|| load("georgia.ttf"));
    if let Some(r) = regular {
        fonts.font_data.insert("r".into(), Arc::new(egui::FontData::from_owned(r)));
    }
    if let Some(s) = semibold {
        fonts.font_data.insert("sb".into(), Arc::new(egui::FontData::from_owned(s)));
    }
    if let Some(f) = serif {
        fonts.font_data.insert("serif".into(), Arc::new(egui::FontData::from_owned(f)));
    }
    fonts.families.insert(FontFamily::Proportional, vec!["r".into()]);
    fonts.families.insert(FontFamily::Name("serif".into()), vec!["serif".into(), "sb".into(), "r".into()]);
    fonts.families.insert(FontFamily::Monospace, vec!["r".into()]);
    fonts.families.insert(FontFamily::Name("sb".into()), vec!["sb".into(), "r".into()]);
    ctx.set_fonts(fonts);

    ctx.style_mut(|s| {
        let v = &mut s.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = c::BG;
        v.window_fill = c::SURFACE;
        v.extreme_bg_color = c::SURFACE;
        v.override_text_color = Some(c::TEXT);
        v.selection.bg_fill = c::ACCENT.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0_f32, c::TEXT);
        v.text_cursor.stroke = Stroke::new(1.5_f32, c::ACCENT);
        v.widgets.inactive.bg_fill = c::RAISED;
        v.widgets.hovered.bg_fill = c::HOVER;
        v.widgets.active.bg_fill = c::HOVER;
        v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, c::MUTED);
        v.window_shadow = egui::epaint::Shadow::NONE;
        v.popup_shadow = egui::epaint::Shadow::NONE;
        s.spacing.item_spacing = vec2(0.0, 0.0);
        s.spacing.scroll = egui::style::ScrollStyle {
            bar_width: 6.0,
            floating_allocated_width: 0.0,
            bar_inner_margin: 3.0,
            bar_outer_margin: 2.0,
            ..egui::style::ScrollStyle::floating()
        };
        s.interaction.selectable_labels = false;
        s.animation_time = 0.12;
    });
}

// ---------------------------------------------------------------- text helpers

pub fn galley(ui: &Ui, text: &str, font: FontId, color: Color32) -> Arc<Galley> {
    ui.painter().layout_no_wrap(text.to_owned(), font, color)
}

/// Single line, cut with "…" to fit `max_w`.
pub fn truncated(ui: &Ui, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
    job.wrap = TextWrapping::truncate_at_width(max_w.max(1.0));
    ui.fonts_mut(|f| f.layout_job(job))
}

pub fn text_at(ui: &Ui, pos: egui::Pos2, anchor: Align2, text: &str, font: FontId, color: Color32) {
    ui.painter().text(pos, anchor, text, font, color);
}

pub fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(title_font()).color(c::TEXT));
    ui.add_space(26.0);
}

pub fn section(ui: &mut Ui, text: &str, top: f32) {
    ui.add_space(top);
    ui.label(RichText::new(text).font(small()).color(c::MUTED));
    ui.add_space(10.0);
}

pub fn sub(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(small()).color(c::MUTED));
}

// ---------------------------------------------------------------- buttons

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
}

pub fn button(ui: &mut Ui, text: &str, kind: Kind, enabled: bool) -> Response {
    let g = galley(ui, text, btn_font(), c::TEXT);
    let pad = if kind == Kind::Ghost { 9.0 } else { 14.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2((g.size().x + pad * 2.0).round(), 32.0), if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(resp.id, enabled && resp.hovered(), 0.09);
        let down = enabled && resp.is_pointer_button_down_on();
        let (fill, stroke, text_c) = match kind {
            Kind::Primary => (
                if !enabled { c::RAISED } else if down { c::ACCENT } else { c::ACCENT.lerp_to_gamma(c::ACCENT_HOVER, t) },
                None,
                if enabled { c::ON_ACCENT } else { c::FAINT },
            ),
            Kind::Secondary => (
                if !enabled { c::SURFACE } else if down { c::RAISED } else { c::RAISED.lerp_to_gamma(c::HOVER, t) },
                Some(if enabled { c::BORDER } else { c::DIVIDER }),
                if enabled { c::TEXT } else { c::FAINT },
            ),
            Kind::Ghost => (
                Color32::TRANSPARENT.lerp_to_gamma(c::RAISED, t),
                None,
                if enabled { c::MUTED.lerp_to_gamma(c::TEXT, t) } else { c::FAINT },
            ),
        };
        let p = ui.painter();
        p.rect_filled(rect, 6.0, fill);
        if let Some(s) = stroke {
            p.rect_stroke(rect, 6.0, Stroke::new(1.0_f32, s), StrokeKind::Inside);
        }
        p.galley_with_override_text_color(rect.center() - g.size() / 2.0, g, text_c);
    }
    if enabled { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp }
}

// ---------------------------------------------------------------- inputs

pub fn field(ui: &mut Ui, id: &str, s: &mut String, hint: &str, width: f32, max_len: usize) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
    let bg = ui.painter().add(Shape::Noop);
    let te = TextEdit::singleline(s)
        .id_salt(id)
        .frame(false)
        .hint_text(RichText::new(hint).color(c::FAINT))
        .font(body())
        .text_color(c::TEXT)
        .margin(Margin::ZERO)
        .vertical_align(Align::Center)
        .char_limit(max_len);
    let resp = ui.put(rect.shrink2(vec2(11.0, 0.0)), te);
    let focus = resp.has_focus();
    let t = ui.ctx().animate_bool_with_time(resp.id.with("f"), focus, 0.1);
    ui.painter().set(
        bg,
        Shape::Vec(vec![
            RectShape::filled(rect, 6.0, c::SURFACE).into(),
            RectShape::stroke(rect, 6.0, Stroke::new(1.0_f32, c::BORDER.lerp_to_gamma(c::ACCENT, t)), StrokeKind::Inside).into(),
        ]),
    );
    resp
}

pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let g = galley(ui, label, body(), c::TEXT);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(46.0 + g.size().x, 34.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.14);
    let track = Rect::from_min_size(pos2(rect.left(), rect.center().y - 9.0), vec2(32.0, 18.0));
    let p = ui.painter();
    p.rect_filled(track, 9.0, c::RAISED.lerp_to_gamma(c::ACCENT, t));
    p.rect_stroke(track, 9.0, Stroke::new(1.0_f32, c::BORDER.lerp_to_gamma(c::ACCENT, t)), StrokeKind::Inside);
    let kx = egui::lerp(track.left() + 9.0..=track.right() - 9.0, t);
    p.circle_filled(pos2(kx, track.center().y), 7.0, c::MUTED.lerp_to_gamma(c::ON_ACCENT, t));
    p.galley_with_override_text_color(pos2(track.right() + 14.0, rect.center().y - g.size().y / 2.0), g, c::TEXT);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

// ---------------------------------------------------------------- cards and rows

/// Rounded bordered container sized to its content.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let bg = ui.painter().add(Shape::Noop);
    let w = ui.available_width();
    let r = ui.vertical(|ui| {
        ui.set_width(w);
        ui.add_space(4.0);
        let r = add(ui);
        ui.add_space(4.0);
        r
    });
    let rect = r.response.rect;
    ui.painter().set(
        bg,
        Shape::Vec(vec![
            RectShape::filled(rect, 8.0, c::SURFACE).into(),
            RectShape::stroke(rect, 8.0, Stroke::new(1.0_f32, c::BORDER), StrokeKind::Inside).into(),
        ]),
    );
    r.inner
}

pub enum Lead<'a> {
    None,
    Num(&'a str),
    Check,
    Dot(Color32),
    /// Something is in progress.
    Spin,
}

/// One line inside a card. `actions` are laid out right-to-left at the right edge.
pub fn row(ui: &mut Ui, lead: Lead, title_text: &str, detail: &str, detail_color: Color32, compact: bool, divider: bool, actions: impl FnOnce(&mut Ui)) {
    let h = if compact { 40.0 } else { 62.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    if divider {
        ui.painter().hline(rect.x_range().shrink(16.0), rect.top(), Stroke::new(1.0_f32, c::DIVIDER));
    }
    // A detached child so the buttons don't move the parent's cursor.
    let mut act = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(16.0, 0.0))).layout(Layout::right_to_left(Align::Center)));
    act.spacing_mut().item_spacing.x = 8.0;
    actions(&mut act);
    let used = act.min_rect();
    let right = if used.width() > 1.0 { used.left() - 16.0 } else { rect.right() - 16.0 };

    let mut x = rect.left() + 18.0;
    let cy = rect.center().y;
    let p = ui.painter();
    match lead {
        Lead::None => {}
        Lead::Num(n) => {
            p.text(pos2(x + 8.0, cy), Align2::CENTER_CENTER, n, strong(), c::FAINT);
            x += 34.0;
        }
        Lead::Check => {
            let s = Stroke::new(2.0_f32, c::GOOD);
            let o = pos2(x + 8.0, cy);
            p.line_segment([o + vec2(-5.0, 0.0), o + vec2(-1.5, 3.5)], s);
            p.line_segment([o + vec2(-1.5, 3.5), o + vec2(5.0, -4.0)], s);
            x += 34.0;
        }
        Lead::Dot(col) => {
            p.circle_filled(pos2(x + 8.0, cy), 3.5, col);
            x += 34.0;
        }
        Lead::Spin => {
            spinner(ui, pos2(x + 8.0, cy));
            x += 34.0;
        }
    }
    if compact {
        let tw = 210f32.min(((right - x) / 2.0).max(80.0));
        let g = truncated(ui, title_text, body(), c::TEXT, tw);
        ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, c::TEXT);
        let g = truncated(ui, detail, body(), detail_color, right - x - tw - 14.0);
        ui.painter().galley(pos2(x + tw + 14.0, cy - g.size().y / 2.0), g, detail_color);
    } else {
        let g = truncated(ui, title_text, strong(), c::TEXT, right - x);
        ui.painter().galley(pos2(x, rect.top() + 12.0), g, c::TEXT);
        let g = truncated(ui, detail, small(), detail_color, right - x);
        ui.painter().galley(pos2(x, rect.top() + 34.0), g, detail_color);
    }
}

// ---------------------------------------------------------------- tables

pub struct Col {
    pub title: &'static str,
    pub weight: f32,
    pub right: bool,
}

pub const fn col(title: &'static str, weight: f32) -> Col {
    Col { title, weight, right: false }
}
pub const fn col_r(title: &'static str, weight: f32) -> Col {
    Col { title, weight, right: true }
}

#[derive(Default)]
pub struct TableOut {
    pub clicked: Option<usize>,
    pub double: Option<usize>,
}

/// Virtualised table in a card filling `rect`. `cell(row, col)` returns text and colour.
pub fn table(ui: &mut Ui, id: &str, rect: Rect, cols: &[Col], n: usize, selected: Option<usize>, empty: &str, cell: impl Fn(usize, usize) -> (String, Color32)) -> TableOut {
    let mut out = TableOut::default();
    let p = ui.painter();
    p.rect_filled(rect, 8.0, c::SURFACE);
    p.rect_stroke(rect, 8.0, Stroke::new(1.0_f32, c::BORDER), StrokeKind::Inside);

    let inner = rect.shrink2(vec2(1.0, 6.0));
    let total: f32 = cols.iter().map(|c| c.weight).sum();
    let w = inner.width();
    let mut xs = Vec::with_capacity(cols.len() + 1);
    let mut acc = inner.left();
    for c in cols {
        xs.push(acc);
        acc += w * c.weight / total;
    }
    xs.push(inner.right());

    let header = Rect::from_min_size(inner.min, vec2(w, 36.0));
    for (i, col) in cols.iter().enumerate() {
        let (l, r) = (xs[i] + 14.0, xs[i + 1] - 14.0);
        let (x, a) = if col.right { (r, Align2::RIGHT_CENTER) } else { (l, Align2::LEFT_CENTER) };
        ui.painter().text(pos2(x, header.center().y), a, col.title, small(), c::FAINT);
    }

    let body_rect = Rect::from_min_max(pos2(inner.left(), header.bottom()), inner.max);
    if n == 0 {
        ui.painter().text(pos2(inner.left() + 14.0, body_rect.top() + ROW_H / 2.0), Align2::LEFT_CENTER, empty, body_font_small(), c::FAINT);
        return out;
    }
    ui.scope_builder(UiBuilder::new().max_rect(body_rect), |ui| {
        egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show_rows(ui, ROW_H, n, |ui, range| {
            for i in range {
                let (r, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
                if !ui.is_rect_visible(r) {
                    continue;
                }
                let hov = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.08);
                let fill = if selected == Some(i) { c::SELECT } else { Color32::TRANSPARENT.lerp_to_gamma(c::RAISED.gamma_multiply(0.55), hov) };
                if fill != Color32::TRANSPARENT {
                    ui.painter().rect_filled(r, 0.0, fill);
                }
                if i > 0 {
                    ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0_f32, c::DIVIDER));
                }
                for (ci, col) in cols.iter().enumerate() {
                    let (text, color) = cell(i, ci);
                    let maxw = xs[ci + 1] - xs[ci] - 28.0;
                    let g = truncated(ui, &text, body(), color, maxw);
                    let x = if col.right { xs[ci + 1] - 14.0 - g.size().x } else { xs[ci] + 14.0 };
                    ui.painter().galley(pos2(x, r.center().y - g.size().y / 2.0), g, color);
                }
                if resp.clicked() {
                    out.clicked = Some(i);
                }
                if resp.double_clicked() {
                    out.double = Some(i);
                }
            }
        });
    });
    out
}

fn body_font_small() -> FontId {
    body()
}

// ---------------------------------------------------------------- window chrome

#[derive(Clone, Copy, PartialEq)]
pub enum Caption {
    Min,
    Max,
    Restore,
    Close,
}

pub fn caption_button(ui: &mut Ui, rect: Rect, kind: Caption) -> Response {
    let resp = ui.interact(rect, Id::new(("caption", kind as u8)), Sense::click());
    let t = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.08);
    let (bg, fg) = if kind == Caption::Close {
        (Color32::TRANSPARENT.lerp_to_gamma(c::CLOSE, t), c::MUTED.lerp_to_gamma(Color32::WHITE, t))
    } else {
        (Color32::TRANSPARENT.lerp_to_gamma(c::RAISED, t), c::MUTED.lerp_to_gamma(c::TEXT, t))
    };
    let p = ui.painter();
    if bg.a() > 0 {
        p.rect_filled(rect, CornerRadius::ZERO, bg);
    }
    let o = rect.center();
    let s = Stroke::new(1.0_f32, fg);
    let snap = |v: f32| v.round() + 0.5;
    match kind {
        Caption::Min => {
            p.hline((o.x - 5.0)..=(o.x + 5.0), snap(o.y), s);
        }
        Caption::Max => {
            p.rect_stroke(Rect::from_center_size(pos2(snap(o.x), snap(o.y)), vec2(10.0, 10.0)), 1.5, s, StrokeKind::Middle);
        }
        Caption::Restore => {
            let r = Rect::from_center_size(pos2(snap(o.x) - 1.0, snap(o.y) + 1.0), vec2(8.0, 8.0));
            p.rect_stroke(r, 1.0, s, StrokeKind::Middle);
            p.hline((r.left() + 2.0)..=(r.right() + 2.0), r.top() - 2.0, s);
            p.vline(r.right() + 2.0, (r.top() - 2.0)..=(r.bottom() - 2.0), s);
        }
        Caption::Close => {
            p.line_segment([o + vec2(-5.0, -5.0), o + vec2(5.0, 5.0)], s);
            p.line_segment([o + vec2(5.0, -5.0), o + vec2(-5.0, 5.0)], s);
        }
    };
    resp
}

/// Wrapped paragraph used in dialogs.
pub fn paragraph(ui: &mut Ui, text: &str, color: Color32) {
    ui.add(egui::Label::new(RichText::new(text).font(body()).color(color)).wrap_mode(TextWrapMode::Wrap));
}

// ---------------------------------------------------------------- pixel art

/// A small loading ring: eight pixel blocks, one bright one chasing round with a fading tail.
pub fn spinner(ui: &Ui, center: egui::Pos2) {
    let t = ui.ctx().input(|i| i.time);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
    let head = ((t * 10.0) as usize) % 8;
    for i in 0..8 {
        let a = i as f32 / 8.0 * std::f32::consts::TAU;
        let p = center + vec2(a.sin(), -a.cos()) * 6.0;
        let age = (head + 8 - i) % 8;
        let col = if age == 0 { c::ACCENT } else { c::ACCENT.gamma_multiply((0.55 - age as f32 * 0.12).max(0.12)) };
        let r = Rect::from_center_size(pos2(p.x.round(), p.y.round()), vec2(3.0, 3.0));
        ui.painter().rect_filled(r, 0.0, col);
    }
}

/// "BINGO HUB" in the serif, letter-spaced.
pub fn wordmark(ui: &Ui, left_center: egui::Pos2) {
    let mut job = LayoutJob::default();
    job.append(
        "BINGO HUB",
        0.0,
        TextFormat { font_id: FontId::new(15.0, FontFamily::Name("serif".into())), color: c::TEXT, extra_letter_spacing: 2.2, ..Default::default() },
    );
    let g = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().galley(pos2(left_center.x, left_center.y - g.size().y / 2.0), g, c::TEXT);
}
