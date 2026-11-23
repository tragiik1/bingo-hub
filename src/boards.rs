//! Editing on the Boards page. Goals are changed in place: double-click (or Enter) to edit, the
//! field at the bottom to add, Delete to remove. Boards are made, renamed, copied and removed from
//! the list on the left. Every change is saved straight away, and Ctrl+Z steps back through them.

use std::path::{Path, PathBuf};

use eframe::egui::{
    self, pos2, text::CCursor, text::CCursorRange, vec2, Align, Align2, Color32, CursorIcon, Frame, Id, Key, Margin, Modifiers, Popup, Rect, Sense,
    Stroke, StrokeKind, TextEdit, Ui, UiBuilder,
};

use crate::core::{self, Board, Goal};
use crate::ui::{self, c, Kind, ROW_H};

/// Goals the Bingo Board needs for one card: its board size squared (5×5 unless changed there).
pub fn card_size() -> usize {
    let n = core::board_config_get("GS_BoardSize").and_then(|s| s.parse().ok()).filter(|n| (3..=8).contains(n)).unwrap_or(5usize);
    n * n
}

/// What the page should do after a frame of editing.
pub enum Out {
    /// A board file changed (or was made): reload, and select this one.
    Changed(PathBuf),
    /// A board file went away: reload.
    Reload,
    Toast(String),
    Error(String),
    /// Ask before moving this board to the Recycle Bin.
    ConfirmDelete(PathBuf, String),
    Select(usize),
    ClearSearch,
}

enum Undo {
    Goals { path: PathBuf, before: Vec<Goal> },
    Created(PathBuf),
    Renamed { from: PathBuf, to: PathBuf },
}

struct GoalEdit {
    index: usize,
    text: String,
    err: Option<&'static str>,
    fresh: bool,
}

struct Rename {
    path: PathBuf,
    text: String,
    err: Option<String>,
    fresh: bool,
    /// After naming a new board, carry on into adding goals.
    then_add: bool,
}

#[derive(Default)]
pub struct Editor {
    board: Option<PathBuf>,
    goal_sel: Option<usize>,
    edit: Option<GoalEdit>,
    add: String,
    add_err: Option<&'static str>,
    focus_add: bool,
    rename: Option<Rename>,
    undo: Vec<Undo>,
    scroll_to: Option<usize>,
    /// The goal list's scroll offset and height last frame, to scroll only as far as needed.
    view: (f32, f32),
    /// Goals one card needs (`card_size`), read when the page loads its boards.
    pub card: usize,
}

fn dup(b: &Board, text: &str, except: Option<usize>) -> bool {
    let t = text.trim().to_lowercase();
    b.goals.iter().enumerate().any(|(i, g)| Some(i) != except && g.name.trim().to_lowercase() == t)
}

fn panel(ui: &Ui, rect: Rect) {
    let p = ui.painter();
    p.rect_filled(rect, 8.0, c::SURFACE);
    p.rect_stroke(rect, 8.0, Stroke::new(1.0_f32, c::BORDER), StrokeKind::Inside);
}

/// Focuses a single-line edit and puts the cursor at the end, or selects all of it.
fn focus_text(ui: &Ui, id: Id, len: usize, select_all: bool) {
    ui.memory_mut(|m| m.request_focus(id));
    let mut st = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let range = if select_all { CCursorRange::two(CCursor::new(0), CCursor::new(len)) } else { CCursorRange::one(CCursor::new(len)) };
    st.cursor.set_char_range(Some(range));
    st.store(ui.ctx(), id);
}

fn inline_edit(ui: &mut Ui, rect: Rect, id: Id, text: &mut String, max: usize) -> egui::Response {
    let te = TextEdit::singleline(text)
        .id(id)
        .frame(false)
        .font(ui::body())
        .text_color(c::TEXT)
        .margin(Margin::ZERO)
        .vertical_align(Align::Center)
        .char_limit(max);
    // A detached child, so the edit doesn't move the list's layout cursor.
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(Align::Center)));
    child.add_sized(rect.size(), te)
}

/// A small text button drawn at `right`, right-aligned. Returns its rect and whether it was clicked.
fn link(ui: &mut Ui, right: f32, cy: f32, text: &str, id: Id, danger: bool) -> (Rect, bool) {
    let g = ui::galley(ui, text, ui::small(), c::MUTED);
    let r = Rect::from_min_size(pos2(right - g.size().x - 8.0, cy - 12.0), vec2(g.size().x + 16.0, 24.0));
    let resp = ui.interact(r, id, Sense::click());
    let t = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.08);
    let col = c::MUTED.lerp_to_gamma(if danger { c::BAD } else { c::TEXT }, t);
    if t > 0.0 {
        ui.painter().rect_filled(r, 5.0, Color32::TRANSPARENT.lerp_to_gamma(c::HOVER, t));
    }
    ui.painter().galley(pos2(r.left() + 8.0, cy - g.size().y / 2.0), g, col);
    (r, resp.on_hover_cursor(CursorIcon::PointingHand).clicked())
}

fn menu_item(ui: &mut Ui, text: &str, danger: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let t = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.06);
    ui.painter().rect_filled(r, 5.0, Color32::TRANSPARENT.lerp_to_gamma(c::HOVER, t));
    ui::text_at(ui, pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, text, ui::body(), if danger { c::BAD } else { c::TEXT });
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

impl Editor {
    /// True while a goal or a board name is being typed, so page shortcuts stay out of the way.
    pub fn busy(&self) -> bool {
        self.edit.is_some() || self.rename.is_some()
    }

    fn push(&mut self, u: Undo) {
        self.undo.push(u);
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
    }

    /// Saves `board` with its goals changed by `f`, remembering the old list for Ctrl+Z.
    fn change(&mut self, board: &Board, out: &mut Vec<Out>, f: impl FnOnce(&mut Vec<Goal>)) -> bool {
        let mut b = board.clone();
        f(&mut b.goals);
        match core::save_board(&b) {
            Ok(()) => {
                self.push(Undo::Goals { path: b.path.clone(), before: board.goals.clone() });
                out.push(Out::Changed(b.path));
                true
            }
            Err(e) => {
                out.push(Out::Error(e));
                false
            }
        }
    }

    fn delete_goal(&mut self, board: &Board, i: usize, out: &mut Vec<Out>) {
        if i >= board.goals.len() {
            return;
        }
        let name = board.goals[i].name.clone();
        if self.change(board, out, |g| {
            g.remove(i);
        }) {
            let left = board.goals.len() - 1;
            self.goal_sel = (left > 0).then(|| i.min(left - 1));
            let short: String = if name.chars().count() > 40 { name.chars().take(38).collect::<String>() + "…" } else { name };
            out.push(Out::Toast(format!("Deleted \"{short}\". Ctrl+Z brings it back.")));
        }
    }

    fn start_edit(&mut self, board: &Board, i: usize) {
        if let Some(g) = board.goals.get(i) {
            self.goal_sel = Some(i);
            self.edit = Some(GoalEdit { index: i, text: g.name.clone(), err: None, fresh: true });
            self.scroll_to = Some(i);
        }
    }

    /// Steps back one change.
    pub fn undo(&mut self, board_running: bool, out: &mut Vec<Out>) {
        self.edit = None;
        self.rename = None;
        let Some(u) = self.undo.pop() else {
            out.push(Out::Toast("Nothing to undo".into()));
            return;
        };
        let res = match u {
            Undo::Goals { path, before } => {
                let b = Board { name: core::pretty(&path), path: path.clone(), goals: before };
                core::save_board(&b).map(|_| Out::Changed(path))
            }
            Undo::Created(path) => crate::sys::recycle(&path).map(|_| Out::Reload),
            Undo::Renamed { from, to } => {
                let stem = from.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                core::rename_board(&to, &stem, board_running).map(Out::Changed)
            }
        };
        match res {
            Ok(o) => {
                out.push(o);
                out.push(Out::Toast("Undone".into()));
            }
            Err(e) => out.push(Out::Error(e)),
        }
    }

    /// Drops undo steps for a board that has gone to the Recycle Bin.
    pub fn forget(&mut self, path: &Path) {
        self.undo.retain(|u| match u {
            Undo::Goals { path: p, .. } | Undo::Created(p) => p != path,
            Undo::Renamed { from, to } => from != path && to != path,
        });
    }

    pub fn new_board(&mut self, out: &mut Vec<Out>) {
        let name = core::free_board_name("New board");
        match core::create_board(&name, vec![]) {
            Ok(path) => {
                self.push(Undo::Created(path.clone()));
                self.rename = Some(Rename { path: path.clone(), text: name, err: None, fresh: true, then_add: true });
                out.push(Out::Changed(path));
                out.push(Out::ClearSearch);
            }
            Err(e) => out.push(Out::Error(e)),
        }
    }

    fn duplicate(&mut self, b: &Board, out: &mut Vec<Out>) {
        let name = core::free_board_name(&format!("{} copy", b.name));
        match core::create_board(&name, b.goals.clone()) {
            Ok(path) => {
                self.push(Undo::Created(path.clone()));
                self.rename = Some(Rename { path: path.clone(), text: name, err: None, fresh: true, then_add: false });
                out.push(Out::Changed(path));
            }
            Err(e) => out.push(Out::Error(e)),
        }
    }

    fn commit_rename(&mut self, board_running: bool, enter: bool, out: &mut Vec<Out>) {
        let Some(mut r) = self.rename.take() else { return };
        let text = r.text.trim().to_string();
        let same = core::board_path_for(&text).is_ok_and(|p| p == r.path);
        if !same && !text.is_empty() {
            match core::rename_board(&r.path, &text, board_running) {
                Ok(to) => {
                    self.push(Undo::Renamed { from: r.path.clone(), to: to.clone() });
                    out.push(Out::Changed(to));
                }
                Err(e) if enter => {
                    r.err = Some(if e.contains("already") { "Name taken".into() } else { "Can't use that name".into() });
                    r.fresh = true;
                    self.rename = Some(r);
                    return;
                }
                Err(e) => {
                    out.push(Out::Toast(format!("Name not changed. {e}")));
                    return;
                }
            }
        }
        if r.then_add && enter {
            self.focus_add = true;
        }
    }

    fn commit_edit(&mut self, board: &Board, enter: bool, out: &mut Vec<Out>) {
        let Some(mut e) = self.edit.take() else { return };
        let text = e.text.trim().to_string();
        let Some(old) = board.goals.get(e.index) else { return };
        if text.is_empty() || text == old.name {
            return;
        }
        if dup(board, &text, Some(e.index)) {
            if enter {
                e.err = Some("Already on this board");
                e.fresh = true;
                self.edit = Some(e);
            } else {
                out.push(Out::Toast("Not changed: that goal is already on this board.".into()));
            }
            return;
        }
        let i = e.index;
        self.change(board, out, |g| g[i].name = text);
    }

    /// Keys for the goal list, when nothing is being typed: arrows move, Enter or F2 edits,
    /// Delete removes, Ctrl+Z undoes.
    pub fn keys(&mut self, ui: &Ui, board: Option<&Board>, query: &str, board_running: bool, out: &mut Vec<Out>) {
        if self.busy() || ui.memory(|m| m.focused().is_some()) {
            return;
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)) {
            self.undo(board_running, out);
            return;
        }
        let Some(b) = board else { return };
        let vis = visible(b, query);
        let pos = self.goal_sel.and_then(|g| vis.iter().position(|&v| v == g));
        let (down, up, edit, del) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::F2),
                i.consume_key(Modifiers::NONE, Key::Delete),
            )
        });
        let mv = |p: usize| {
            let g = vis[p];
            (Some(g), Some(g))
        };
        if !vis.is_empty() && (down || up) {
            let p = match (pos, down) {
                (None, true) => 0,
                (None, false) => vis.len() - 1,
                (Some(p), true) => (p + 1).min(vis.len() - 1),
                (Some(p), false) => p.saturating_sub(1),
            };
            (self.goal_sel, self.scroll_to) = mv(p);
        }
        if let (Some(g), true) = (self.goal_sel.filter(|_| pos.is_some()), edit) {
            self.start_edit(b, g);
        }
        if let (Some(g), true) = (self.goal_sel.filter(|_| pos.is_some()), del) {
            self.delete_goal(b, g, out);
        }
    }

    // ------------------------------------------------------------------ boards list

    pub fn boards_panel(&mut self, ui: &mut Ui, rect: Rect, boards: &[Board], sel: usize, board_running: bool, out: &mut Vec<Out>) {
        panel(ui, rect);
        let inner = rect.shrink2(vec2(1.0, 6.0));
        let header = Rect::from_min_size(inner.min, vec2(inner.width(), 36.0));
        let bar = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 46.0), inner.max);
        let (l, r) = (inner.left() + 14.0, inner.right() - 14.0);
        let count_r = r - 26.0;
        ui::text_at(ui, pos2(l, header.center().y), Align2::LEFT_CENTER, "Board", ui::small(), c::FAINT);
        ui::text_at(ui, pos2(count_r, header.center().y), Align2::RIGHT_CENTER, "Goals", ui::small(), c::FAINT);

        // "New board" sits at the foot of the list.
        ui.painter().hline(inner.x_range(), bar.top(), Stroke::new(1.0_f32, c::DIVIDER));
        ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_size(pos2(bar.left() + 6.0, bar.center().y - 16.0), vec2(200.0, 32.0))), |ui| {
            if ui::button(ui, "+  New board", Kind::Ghost, true).clicked() {
                self.new_board(out);
            }
        });

        let body = Rect::from_min_max(pos2(inner.left(), header.bottom()), pos2(inner.right(), bar.top()));
        if boards.is_empty() {
            ui::text_at(ui, pos2(l, body.top() + ROW_H / 2.0), Align2::LEFT_CENTER, "No boards yet", ui::body(), c::FAINT);
            return;
        }
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            egui::ScrollArea::vertical().id_salt("board_list").auto_shrink([false, false]).show_rows(ui, ROW_H, boards.len(), |ui, range| {
                for i in range {
                    let b = &boards[i];
                    let (row, resp) = ui.allocate_exact_size(vec2(body.width(), ROW_H), Sense::click());
                    if i > 0 {
                        ui.painter().hline(row.x_range(), row.top(), Stroke::new(1.0_f32, c::DIVIDER));
                    }
                    let renaming = self.rename.as_ref().is_some_and(|r| r.path == b.path);
                    // The pointer test is on the rect: the row's own buttons sit on top of it.
                    let over = ui.rect_contains_pointer(row) && !Popup::is_any_open(ui.ctx());
                    let hov = ui.ctx().animate_bool_with_time(resp.id, over || resp.context_menu_opened(), 0.08);
                    let fill = if i == sel || renaming { c::SELECT } else { Color32::TRANSPARENT.lerp_to_gamma(c::RAISED.gamma_multiply(0.55), hov) };
                    ui.painter().rect_filled(row, 0.0, fill);
                    let cy = row.center().y;

                    if renaming {
                        let id = Id::new("board_rename");
                        let r_ = self.rename.as_mut().unwrap();
                        let field = Rect::from_min_max(pos2(l - 6.0, row.top() + 5.0), pos2(r - 4.0, row.bottom() - 5.0));
                        let warn = r_.err.is_some();
                        ui.painter().rect_filled(field, 6.0, c::BG);
                        ui.painter().rect_stroke(field, 6.0, Stroke::new(1.0_f32, if warn { c::WARN } else { c::ACCENT }), StrokeKind::Inside);
                        let mut text_r = field.shrink2(vec2(6.0, 0.0));
                        if let Some(err) = &r_.err {
                            let g = ui::galley(ui, err, ui::small(), c::WARN);
                            ui.painter().galley(pos2(text_r.right() - g.size().x, cy - g.size().y / 2.0), g.clone(), c::WARN);
                            text_r.max.x -= g.size().x + 10.0;
                        }
                        let te = inline_edit(ui, text_r, id, &mut r_.text, 80);
                        if te.changed() {
                            r_.err = None;
                        }
                        if r_.fresh {
                            r_.fresh = false;
                            let len = r_.text.chars().count();
                            focus_text(ui, id, len, true);
                        } else if te.lost_focus() {
                            if ui.input(|i| i.key_pressed(Key::Escape)) {
                                // Esc on a board just made with "New board" takes it away again.
                                let made = self.rename.take().filter(|r| r.then_add).map(|r| r.path);
                                if let (Some(p), true) = (made, b.goals.is_empty()) {
                                    if matches!(self.undo.last(), Some(Undo::Created(c)) if *c == p) {
                                        self.undo.pop();
                                    }
                                    let _ = crate::sys::recycle(&p);
                                    out.push(Out::Reload);
                                }
                            } else {
                                let enter = ui.input(|i| i.key_pressed(Key::Enter));
                                self.commit_rename(board_running, enter, out);
                            }
                        }
                        continue;
                    }

                    let name = ui::truncated(ui, &b.name, ui::body(), c::TEXT, count_r - l - 40.0);
                    ui.painter().galley(pos2(l, cy - name.size().y / 2.0), name, c::TEXT);
                    let n = b.goals.len();
                    ui::text_at(ui, pos2(count_r, cy), Align2::RIGHT_CENTER, &n.to_string(), ui::body(), if n < self.card { c::WARN } else { c::MUTED });

                    // A "⋯" menu on hover (and the same menu on right-click).
                    let dots = Rect::from_center_size(pos2(r - 6.0, cy), vec2(24.0, 24.0));
                    let dresp = ui.interact(dots, resp.id.with("dots"), Sense::click());
                    let menu_open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&dresp));
                    let dh = ui.ctx().animate_bool_with_time(dresp.id, dresp.hovered() || menu_open, 0.08);
                    if hov > 0.0 || i == sel || menu_open {
                        if dh > 0.0 {
                            ui.painter().rect_filled(dots, 5.0, Color32::TRANSPARENT.lerp_to_gamma(c::HOVER, dh));
                        }
                        let col = c::FAINT.lerp_to_gamma(c::TEXT, dh);
                        for k in -1..=1 {
                            ui.painter().circle_filled(pos2(dots.center().x + k as f32 * 4.5, cy), 1.6, col);
                        }
                    }
                    let dresp = dresp.on_hover_cursor(CursorIcon::PointingHand);
                    let menu = |ui: &mut Ui, me: &mut Editor, out: &mut Vec<Out>| {
                        if menu_item(ui, "Rename", false) {
                            me.rename = Some(Rename { path: b.path.clone(), text: b.name.clone(), err: None, fresh: true, then_add: false });
                        }
                        if menu_item(ui, "Duplicate", false) {
                            me.duplicate(b, out);
                        }
                        if menu_item(ui, "Show in folder", false) {
                            crate::sys::reveal(&b.path);
                        }
                        let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
                        ui.painter().hline(line.x_range().shrink(4.0), line.center().y, Stroke::new(1.0_f32, c::BORDER));
                        if menu_item(ui, "Delete…", true) {
                            out.push(Out::ConfirmDelete(b.path.clone(), b.name.clone()));
                        }
                    };
                    let frame = Frame::new().fill(c::RAISED).stroke(Stroke::new(1.0_f32, c::BORDER)).corner_radius(8.0).inner_margin(Margin::same(5));
                    Popup::menu(&dresp).frame(frame).width(180.0).show(|ui| menu(ui, self, out));
                    Popup::context_menu(&resp).frame(frame).width(180.0).show(|ui| menu(ui, self, out));

                    if resp.clicked() && !dresp.clicked() {
                        out.push(Out::Select(i));
                    }
                    if resp.double_clicked() {
                        self.rename = Some(Rename { path: b.path.clone(), text: b.name.clone(), err: None, fresh: true, then_add: false });
                    }
                }
            });
        });
    }

    // ------------------------------------------------------------------ goals list

    pub fn goals_panel(&mut self, ui: &mut Ui, rect: Rect, board: Option<&Board>, query: &str, out: &mut Vec<Out>) {
        if self.board.as_deref() != board.map(|b| b.path.as_path()) {
            self.board = board.map(|b| b.path.clone());
            self.goal_sel = None;
            self.edit = None;
            self.add_err = None;
            self.scroll_to = None;
        }
        panel(ui, rect);
        let Some(board) = board else { return };
        let inner = rect.shrink2(vec2(1.0, 6.0));
        let header = Rect::from_min_size(inner.min, vec2(inner.width(), 36.0));
        let bar = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 46.0), inner.max);
        let num_r = inner.left() + 34.0;
        let text_x = inner.left() + 56.0;
        let right = inner.right() - 14.0;

        let n = board.goals.len();
        ui::text_at(ui, pos2(text_x, header.center().y), Align2::LEFT_CENTER, "Goal", ui::small(), c::FAINT);
        let card = self.card;
        let (count, col) = if n < card { (format!("{n} goal{} · a card needs {card}", if n == 1 { "" } else { "s" }), c::WARN) } else { (format!("{n} goals"), c::FAINT) };
        ui::text_at(ui, pos2(right, header.center().y), Align2::RIGHT_CENTER, &count, ui::small(), col);

        // Add bar.
        ui.painter().hline(inner.x_range(), bar.top(), Stroke::new(1.0_f32, c::DIVIDER));
        let field = Rect::from_min_size(pos2(bar.left() + 10.0, bar.center().y - 16.0), vec2(bar.width() - 20.0, 32.0));
        let add_resp = ui.scope_builder(UiBuilder::new().max_rect(field), |ui| ui::field(ui, "goal_add", &mut self.add, "Add a goal", field.width(), 200)).inner;
        if add_resp.changed() {
            self.add_err = None;
        }
        if let Some(e) = self.add_err {
            ui::text_at(ui, pos2(field.right() - 12.0, field.center().y), Align2::RIGHT_CENTER, e, ui::small(), c::WARN);
        }
        if self.focus_add {
            self.focus_add = false;
            focus_text(ui, add_resp.id, self.add.chars().count(), false);
        }
        if add_resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            let t = self.add.trim().to_string();
            if !t.is_empty() {
                if dup(board, &t, None) {
                    self.add_err = Some("Already on this board");
                } else if self.change(board, out, |g| g.push(Goal::new(&t))) {
                    self.add.clear();
                    self.goal_sel = Some(n);
                    self.scroll_to = Some(n);
                    if !query.is_empty() && !t.to_lowercase().contains(query) {
                        out.push(Out::ClearSearch);
                    }
                }
            }
            // Stay in the field, ready for the next goal.
            add_resp.request_focus();
        }

        // The list.
        let body = Rect::from_min_max(pos2(inner.left(), header.bottom()), pos2(inner.right(), bar.top()));
        let vis = visible(board, query);
        if vis.is_empty() {
            let msg = if !query.is_empty() { "No matching goals" } else { "No goals yet. Type one below and press Enter." };
            ui::text_at(ui, pos2(text_x, body.top() + ROW_H / 2.0), Align2::LEFT_CENTER, msg, ui::body(), c::FAINT);
            return;
        }
        let mut area = egui::ScrollArea::vertical().id_salt(("goal_list", &board.path)).auto_shrink([false, false]);
        // A goal added this frame isn't in the list until the board reloads, so keep asking.
        if self.scroll_to.is_some_and(|g| g < n) {
            let g = self.scroll_to.take().unwrap_or(0);
            if let Some(p) = vis.iter().position(|&v| v == g) {
                let (off, h) = self.view;
                let (top, bottom) = (p as f32 * ROW_H, (p + 1) as f32 * ROW_H);
                if top < off {
                    area = area.vertical_scroll_offset(top);
                } else if h > 0.0 && bottom > off + h {
                    area = area.vertical_scroll_offset(bottom - h);
                }
            }
        }
        let shown = ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            area.show_rows(ui, ROW_H, vis.len(), |ui, range| {
                for p in range {
                    let i = vis[p];
                    let (row, resp) = ui.allocate_exact_size(vec2(body.width(), ROW_H), Sense::click());
                    if p > 0 {
                        ui.painter().hline(row.x_range(), row.top(), Stroke::new(1.0_f32, c::DIVIDER));
                    }
                    let cy = row.center().y;
                    let editing = self.edit.as_ref().is_some_and(|e| e.index == i);
                    let over = ui.rect_contains_pointer(row);
                    let hov = ui.ctx().animate_bool_with_time(resp.id, over, 0.08);
                    let selected = self.goal_sel == Some(i);
                    let fill = if selected || editing { c::SELECT } else { Color32::TRANSPARENT.lerp_to_gamma(c::RAISED.gamma_multiply(0.55), hov) };
                    ui.painter().rect_filled(row, 0.0, fill);
                    ui::text_at(ui, pos2(num_r, cy), Align2::RIGHT_CENTER, &(i + 1).to_string(), ui::body(), c::FAINT);

                    if editing {
                        let id = Id::new("goal_edit");
                        let e = self.edit.as_mut().unwrap();
                        let field = Rect::from_min_max(pos2(text_x - 8.0, row.top() + 5.0), pos2(right + 4.0, row.bottom() - 5.0));
                        let warn = e.err.is_some();
                        ui.painter().rect_filled(field, 6.0, c::BG);
                        ui.painter().rect_stroke(field, 6.0, Stroke::new(1.0_f32, if warn { c::WARN } else { c::ACCENT }), StrokeKind::Inside);
                        let mut text_r = field.shrink2(vec2(8.0, 0.0));
                        if let Some(err) = e.err {
                            let g = ui::galley(ui, err, ui::small(), c::WARN);
                            ui.painter().galley(pos2(text_r.right() - g.size().x, cy - g.size().y / 2.0), g.clone(), c::WARN);
                            text_r.max.x -= g.size().x + 12.0;
                        }
                        let te = inline_edit(ui, text_r, id, &mut e.text, 200);
                        if te.changed() {
                            e.err = None;
                        }
                        if e.fresh {
                            e.fresh = false;
                            let len = e.text.chars().count();
                            focus_text(ui, id, len, false);
                        } else if te.lost_focus() {
                            if ui.input(|inp| inp.key_pressed(Key::Escape)) {
                                self.edit = None;
                            } else {
                                let enter = ui.input(|inp| inp.key_pressed(Key::Enter));
                                self.commit_edit(board, enter, out);
                            }
                        }
                        continue;
                    }

                    // Edit and Delete appear on the row under the pointer and on the selected row.
                    let mut text_right = right;
                    if hov > 0.0 || selected {
                        let (dr, del) = link(ui, right + 2.0, cy, "Delete", resp.id.with("del"), true);
                        let (er, ed) = link(ui, dr.left() - 2.0, cy, "Edit", resp.id.with("edit"), false);
                        text_right = er.left() - 8.0;
                        if del {
                            self.delete_goal(board, i, out);
                            continue;
                        }
                        if ed {
                            self.start_edit(board, i);
                            continue;
                        }
                    }
                    let g = ui::truncated(ui, &board.goals[i].name, ui::body(), c::TEXT, text_right - text_x);
                    ui.painter().galley(pos2(text_x, cy - g.size().y / 2.0), g, c::TEXT);
                    if resp.double_clicked() {
                        self.start_edit(board, i);
                    } else if resp.clicked() {
                        self.goal_sel = Some(i);
                        ui.memory_mut(|m| m.surrender_focus(add_resp.id));
                    }
                }
            })
        });
        let st = shown.inner.state;
        self.view = (st.offset.y, shown.inner.inner_rect.height());
    }
}

/// Goals that match the search, as indices into the board.
fn visible(b: &Board, query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    b.goals.iter().enumerate().filter(|(_, g)| q.is_empty() || g.name.to_lowercase().contains(&q)).map(|(i, _)| i).collect()
}
