//! Find & replace, the insertion pad, and version-control dialogs.
//! (A child module of `app`, so it can use the app's private ids and state.)

use super::*;
use crate::{find, git};
use egui::text::{CCursor, CCursorRange};
use regex::Regex;
use std::path::Path;

fn pad_id() -> Id {
    Id::new("pad-text")
}
fn find_input_id() -> Id {
    Id::new("find-input")
}
fn commit_input_id() -> Id {
    Id::new("commit-input")
}

/// Which texts a search covers: the pad when it is open, otherwise the tab you are on.
pub(super) fn text_fields<'a>(p: &'a mut Prompt, view: View, var_names: &[String]) -> Vec<(Id, &'a mut String)> {
    let mut v: Vec<(Id, &'a mut String)> = Vec::new();
    match view {
        View::Editor => {
            if let Some(c) = p.context.as_mut() {
                v.push((context_id(), c));
            }
            for (i, d) in p.definitions.iter_mut().flatten().enumerate() {
                v.push((Id::new(("def-text", i)), &mut d.definition));
            }
            for (i, b) in p.body.iter_mut().enumerate() {
                match b {
                    BodyBlock::Text(t) => v.push((Id::new(("body-text", i)), t)),
                    BodyBlock::Flow(f) => v.push((flow_src_id(&f.id), &mut f.mermaid)),
                    BodyBlock::Code(c) => v.push((code_src_id(&c.id), &mut c.code)),
                }
            }
            for (i, r) in p.rules.iter_mut().flatten().enumerate() {
                v.push((rule_id(i), &mut r.text));
                for (ei, e) in r.examples.iter_mut().enumerate() {
                    v.push((Id::new(("rule-example", i, ei)), e));
                }
            }
        }
        View::Variables => {
            for var in p.variables.iter_mut() {
                if var_names.contains(&var.name) {
                    v.push((var_value_id(&var.name), &mut var.value));
                }
            }
        }
        View::Benchmarks => {
            for (qi, b) in p.benchmarks.iter_mut().enumerate() {
                v.push((bench_q_id(qi), &mut b.question));
                if let Some(e) = b.expected.as_mut() {
                    v.push((bench_expected_id(qi), e));
                }
                for (ai, a) in b.answers.iter_mut().enumerate() {
                    v.push((bench_answer_id(qi, ai), &mut a.text));
                }
            }
        }
    }
    v
}

fn selection_of(ctx: &Context, id: Id) -> Option<(usize, usize)> {
    let r = TextEdit::load_state(ctx, id)?.cursor.char_range()?;
    let (a, b) = (r.primary.index.0, r.secondary.index.0);
    Some((a.min(b), a.max(b)))
}

fn select(ctx: &Context, id: Id, start: usize, end: usize) {
    let mut state = TextEdit::load_state(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(start), CCursor::new(end))));
    state.store(ctx, id);
    // `line_mover` (called by every multiline editor) picks this up: focus + scroll into view.
    ctx.data_mut(|d| d.insert_temp(Id::new("find-focus"), id));
    ctx.request_repaint();
}

/// All matches as (field index, start, end).
fn all_matches(re: &Regex, fields: &[(Id, &mut String)]) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for (fi, (_, t)) in fields.iter().enumerate() {
        for (s, e) in find::find_all(re, t) {
            out.push((fi, s, e));
        }
    }
    out
}

/// Pick the next/previous match relative to `from` = (field, start, end of the current selection), wrapping around.
fn pick(all: &[(usize, usize, usize)], from: Option<(usize, usize, usize)>, forward: bool) -> Option<(usize, usize, usize)> {
    if all.is_empty() {
        return None;
    }
    let Some((cf, cs, ce)) = from else {
        return if forward { all.first().copied() } else { all.last().copied() };
    };
    if forward {
        all.iter().find(|(f, s, _)| (*f, *s) >= (cf, ce)).or(all.first()).copied()
    } else {
        all.iter().rev().find(|(f, _, e)| (*f, *e) <= (cf, cs)).or(all.last()).copied()
    }
}

#[derive(Default)]
pub(super) struct Extras {
    pub find_open: bool,
    pub replace_open: bool,
    pub query: find::Query,
    pub replacement: String,
    pub focus_find: u8,
    /// (field id, start, end) of the match we last jumped to.
    last: Option<(Id, usize, usize)>,
    pub pad_open: bool,
    pub pad_text: String,
    /// Where the pad's content goes: (field id, char index) captured when the pad was opened.
    pub pad_target: Option<(Id, usize)>,
    pub focus_pad: u8,
    pub commit: Option<String>,
    pub commit_focus: u8,
    pub rollback: Option<Rollback>,
    /// Absolute paths of tracked prompts.
    pub tracked: HashSet<PathBuf>,
    pub dirty: bool,
    pub select_name: bool,
    /// The "add a context" chooser is open.
    pub ctx_chooser: bool,
    pub ctx_dialog: Option<CtxDialog>,
}

#[derive(Default)]
pub(super) struct CtxDialog {
    name: String,
    text: String,
    confirm_delete: Option<String>,
}

pub(super) struct Rollback {
    path: PathBuf,
    name: String,
    versions: Result<Vec<git::Version>, String>,
    confirm: Option<usize>,
}

/// Where the search continues from: the focused search field's selection, else the last jump.
fn here(last: Option<(Id, usize, usize)>, ctx: &Context, fields: &[(Id, &mut String)]) -> Option<(usize, usize, usize)> {
        if let Some(fid) = ctx.memory(|m| m.focused()) {
            if let Some(fi) = fields.iter().position(|(id, _)| *id == fid) {
                if let Some((s, e)) = selection_of(ctx, fid) {
                    return Some((fi, s, e));
                }
            }
        }
        let (id, s, e) = last?;
        let fi = fields.iter().position(|(fid, _)| *fid == id)?;
        Some((fi, s, e))
    }

impl PromptEditorApp {
    // ------------------------------------------------------------------
    // shortcuts (called from `logic`)
    // ------------------------------------------------------------------

    pub(super) fn extras_shortcuts(&mut self, ctx: &Context) {
        if self.show_preview {
            return;
        }
        let cmd = Modifiers::COMMAND;
        let (new, find_k, replace_k, pad, commit) = ctx.input_mut(|i| {
            (
                i.consume_key(cmd, Key::N),
                i.consume_key(cmd, Key::F),
                i.consume_key(cmd, Key::H),
                i.consume_key(cmd, Key::P),
                i.consume_key(cmd, Key::K),
            )
        });
        let next = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F3));
        let prev = ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F3));
        if new {
            let folder = self.current_folder();
            self.new_doc_in(ctx, folder);
        }
        if find_k || replace_k {
            self.fx.find_open = true;
            self.fx.replace_open = replace_k || (self.fx.replace_open && !find_k);
            self.fx.focus_find = 3;
            // Seed the box with a single-line selection, like every editor does.
            if let Some(id) = ctx.memory(|m| m.focused()) {
                if let Some((s, e)) = selection_of(ctx, id).filter(|(s, e)| e > s) {
                    let mut fields = self.search_fields();
                    if let Some((_, t)) = fields.iter_mut().find(|(fid, _)| *fid == id) {
                        let sel: String = t.chars().skip(s).take(e - s).collect();
                        if !sel.contains('\n') && sel.chars().count() < 200 {
                            drop(fields);
                            self.fx.query.text = if self.fx.query.regex { regex::escape(&sel) } else { sel };
                        }
                    }
                }
            }
        }
        if next || prev {
            self.find_step(ctx, next);
        }
        if pad {
            self.toggle_pad(ctx);
        }
        if commit {
            self.open_commit(ctx);
        }
    }

    pub(super) fn extras_escape(&mut self, ctx: &Context) -> bool {
        if self.show_preview || !ctx.input(|i| i.key_pressed(Key::Escape)) {
            return false;
        }
        if self.fx.find_open {
            self.fx.find_open = false;
            return true;
        }
        if self.fx.pad_open {
            self.fx.pad_open = false; // the draft is kept
            return true;
        }
        false
    }

    // ------------------------------------------------------------------
    // find & replace
    // ------------------------------------------------------------------

    fn search_fields(&mut self) -> Vec<(Id, &mut String)> {
        if self.fx.pad_open {
            return vec![(pad_id(), &mut self.fx.pad_text)];
        }
        let names = self.doc.prompt.used_variables();
        text_fields(&mut self.doc.prompt, self.view, &names)
    }

    /// Where the search continues from: the focused search field's selection, else the last jump.
    fn regex_or_error(&self) -> Result<Option<Regex>, String> {
        find::compile(&self.fx.query)
    }

    /// Jump to the next / previous match.
    pub(super) fn open_find(&mut self, replace: bool) {
        self.fx.find_open = true;
        self.fx.replace_open = replace || self.fx.replace_open;
        self.fx.focus_find = 3;
    }

    pub(super) fn find_step(&mut self, ctx: &Context, forward: bool) {
        let Ok(Some(re)) = self.regex_or_error() else { return };
        let last = self.fx.last;
        let target = {
            let fields = self.search_fields();
            let all = all_matches(&re, &fields);
            let from = here(last, ctx, &fields);
            pick(&all, from, forward).map(|(fi, s, e)| (fields[fi].0, s, e))
        };
        match target {
            Some((id, s, e)) => self.show_match(ctx, id, s, e),
            None => self.toast("No matches"),
        }
    }

    fn show_match(&mut self, ctx: &Context, id: Id, s: usize, e: usize) {
        // Make sure the field is visible: expand collapsed blocks, open variable cards.
        for b in &mut self.doc.prompt.body {
            match b {
                BodyBlock::Flow(f) if flow_src_id(&f.id) == id => f.collapsed = false,
                BodyBlock::Code(c) if code_src_id(&c.id) == id => c.collapsed = false,
                _ => {}
            }
        }
        if !self.fx.pad_open {
            for n in self.doc.prompt.used_variables() {
                if var_value_id(&n) == id {
                    self.st.var_open.insert(n);
                }
            }
        }
        self.fx.last = Some((id, s, e));
        select(ctx, id, s, e);
    }

    pub(super) fn replace_next(&mut self, ctx: &Context) {
        let Ok(Some(re)) = self.regex_or_error() else { return };
        let q = self.fx.query.clone();
        let rep = self.fx.replacement.clone();
        let last = self.fx.last;
        let done = {
            let mut fields = self.search_fields();
            let all = all_matches(&re, &fields);
            let from = here(last, ctx, &fields).map(|(f, s, _)| (f, s, s));
            // the match under (or after) the cursor
            pick(&all, from.map(|(f, s, _)| (f, s, s)), true).map(|(fi, s, e)| {
                let added = find::replace_at(&q, &re, fields[fi].1, (s, e), &rep);
                (fields[fi].0, s + added)
            })
        };
        match done {
            Some((id, caret)) => {
                self.fx.last = Some((id, caret, caret));
                self.find_step(ctx, true);
            }
            None => self.toast("No matches"),
        }
    }

    pub(super) fn replace_everything(&mut self) {
        let Ok(Some(re)) = self.regex_or_error() else { return };
        let q = self.fx.query.clone();
        let rep = self.fx.replacement.clone();
        let mut n = 0;
        for (_, t) in self.search_fields() {
            n += find::replace_all(&q, &re, t, &rep);
        }
        self.fx.last = None;
        if n == 0 {
            self.toast("No matches");
        } else {
            self.toast(format!("Replaced {n} match{}", if n == 1 { "" } else { "es" }));
        }
    }

    pub(super) fn find_bar(&mut self, ctx: &Context) {
        if !self.fx.find_open || self.show_preview {
            return;
        }
        let compiled = self.regex_or_error();
        let last = self.fx.last;
        let (total, here_idx) = match &compiled {
            Ok(Some(re)) => {
                let fields = self.search_fields();
                let all = all_matches(re, &fields);
                let here = here(last, ctx, &fields);
                let idx = here.and_then(|(f, s, e)| all.iter().position(|m| *m == (f, s, e)));
                (all.len(), idx)
            }
            _ => (0, None),
        };

        let mut step: Option<bool> = None;
        let mut replace_one = false;
        let mut replace_all = false;
        let mut close = false;
        egui::Area::new(Id::new("find-bar"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-26.0, 96.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::bg_card())
                    .corner_radius(egui::CornerRadius::same(theme::RADIUS + 1))
                    .stroke(egui::Stroke::new(1.5, theme::neon()))
                    .inner_margin(egui::Margin::same(8))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 4],
                        blur: 18,
                        spread: 1,
                        color: theme::neon().gamma_multiply(50.0 / 255.0),
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let arrow = if self.fx.replace_open { "▾" } else { "▸" };
                            if theme::ghost_button(ui, arrow).on_hover_text("Show / hide replace (Ctrl+H)").clicked() {
                                self.fx.replace_open = !self.fx.replace_open;
                            }
                            let r = ui.add(
                                TextEdit::singleline(&mut self.fx.query.text)
                                    .id(find_input_id())
                                    .desired_width(230.0)
                                    .hint_text("find")
                                    .font(egui::TextStyle::Button),
                            );
                            if self.fx.focus_find > 0 {
                                r.request_focus();
                                self.fx.focus_find -= 1;
                                if self.fx.focus_find == 0 {
                                    if let Some(mut s) = TextEdit::load_state(ctx, find_input_id()) {
                                        let n = self.fx.query.text.chars().count();
                                        s.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0usize), CCursor::new(n))));
                                        s.store(ctx, find_input_id());
                                    }
                                }
                                ctx.request_repaint();
                            }
                            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                step = Some(!ui.input(|i| i.modifiers.shift));
                            }
                            let aa = ui.selectable_label(self.fx.query.case_sensitive, "Aa");
                            if aa.on_hover_text("Match case").clicked() {
                                self.fx.query.case_sensitive = !self.fx.query.case_sensitive;
                            }
                            let rx = ui.selectable_label(self.fx.query.regex, ".*");
                            if rx.on_hover_text("Regular expression").clicked() {
                                self.fx.query.regex = !self.fx.query.regex;
                            }
                            if theme::ghost_button(ui, "▲").on_hover_text("Previous (Shift+F3)").clicked() {
                                step = Some(false);
                            }
                            if theme::ghost_button(ui, "▼").on_hover_text("Next (F3 / Enter)").clicked() {
                                step = Some(true);
                            }
                            if theme::ghost_button(ui, "✕").on_hover_text("Close (Esc)").clicked() {
                                close = true;
                            }
                        });
                        if self.fx.replace_open {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.add_space(22.0);
                                let r = ui.add(
                                    TextEdit::singleline(&mut self.fx.replacement)
                                        .id(Id::new("replace-input"))
                                        .desired_width(230.0)
                                        .hint_text(if self.fx.query.regex { "replace with  (\\n  \\t  $1)" } else { "replace with" })
                                        .font(egui::TextStyle::Button),
                                );
                                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                    replace_one = true;
                                }
                                if theme::tool_button(ui, "Replace").clicked() {
                                    replace_one = true;
                                }
                                if theme::tool_button(ui, "All").on_hover_text("Replace all matches").clicked() {
                                    replace_all = true;
                                }
                            });
                        }
                        ui.horizontal(|ui| {
                            ui.add_space(22.0);
                            let (text, color) = match &compiled {
                                Err(e) => (format!("invalid regex: {e}"), theme::DANGER),
                                Ok(None) => ("type to search".to_owned(), theme::TEXT_MUTED),
                                Ok(Some(_)) if total == 0 => ("no matches".to_owned(), theme::WARN),
                                Ok(Some(_)) => (
                                    match here_idx {
                                        Some(i) => format!("{} of {total}", i + 1),
                                        None => format!("{total} match{}", if total == 1 { "" } else { "es" }),
                                    },
                                    theme::GOOD,
                                ),
                            };
                            ui.label(RichText::new(text).small().color(color));
                            let scope = if self.fx.pad_open {
                                "in the pad"
                            } else {
                                match self.view {
                                    View::Editor => "in the editor",
                                    View::Variables => "in variable values",
                                    View::Benchmarks => "in benchmarks",
                                }
                            };
                            ui.label(theme::muted(scope).small());
                        });
                    });
            });
        if close {
            self.fx.find_open = false;
        }
        if let Some(fwd) = step {
            self.find_step(ctx, fwd);
        }
        if replace_one {
            self.replace_next(ctx);
        }
        if replace_all {
            self.replace_everything();
        }
    }

    // ------------------------------------------------------------------
    // the pad
    // ------------------------------------------------------------------

    pub(super) fn toggle_pad(&mut self, ctx: &Context) {
        if self.fx.pad_open {
            self.fx.pad_open = false;
            return;
        }
        // Remember where the cursor is so the content lands there.
        self.fx.pad_target = None;
        if self.view == View::Editor {
            if let Some(id) = ctx.memory(|m| m.focused()) {
                let mut fields = self.search_fields();
                if fields.iter_mut().any(|(fid, _)| *fid == id) {
                    if let Some(r) = TextEdit::load_state(ctx, id).and_then(|s| s.cursor.char_range()) {
                        self.fx.pad_target = Some((id, r.primary.index.0));
                    }
                }
            }
        }
        self.fx.pad_open = true;
        self.fx.focus_pad = 3;
    }

    /// Ctrl+Enter in the pad: drop its text at the remembered cursor position.
    pub(super) fn insert_pad(&mut self, ctx: &Context) {
        let text = self.fx.pad_text.clone();
        self.fx.pad_open = false;
        if text.trim().is_empty() {
            return;
        }
        self.view = View::Editor;
        let target = self.fx.pad_target.take();
        let was_pad = std::mem::replace(&mut self.fx.pad_open, false);
        let _ = was_pad;
        let placed = {
            let body_len = self.doc.prompt.body.len();
            let mut fields = text_fields(&mut self.doc.prompt, View::Editor, &[]);
            let pick = match target.and_then(|(id, at)| fields.iter().position(|(fid, _)| *fid == id).map(|i| (i, at))) {
                Some(t) => Some(t),
                // No remembered cursor: append to the last body text.
                None => fields
                    .iter()
                    .rposition(|(id, _)| (0..body_len).any(|i| *id == Id::new(("body-text", i))))
                    .map(|i| (i, usize::MAX)),
            };
            pick.map(|(i, at)| {
                let t = &mut *fields[i].1;
                let n = t.chars().count();
                let at = at.min(n);
                let byte = t.char_indices().nth(at).map_or(t.len(), |(b, _)| b);
                t.insert_str(byte, &text);
                (fields[i].0, at + text.chars().count())
            })
        };
        self.fx.pad_text.clear();
        if let Some((id, caret)) = placed {
            self.fx.last = None;
            select(ctx, id, caret, caret);
            self.toast("Inserted from the pad");
        }
    }

    pub(super) fn pad_window(&mut self, ctx: &Context) {
        if !self.fx.pad_open || self.show_preview {
            return;
        }
        let mut insert = false;
        let mut close = false;
        let size = ctx.content_rect().size() * Vec2::new(0.55, 0.62);
        egui::Window::new("Pad")
            .id(Id::new("pad-window"))
            .collapsible(false)
            .default_size(size)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, 20.0))
            .show(ctx, |ui| {
                ui.label(
                    theme::muted(
                        "Paste and tidy your text here (find & replace works in this window). \
                         Ctrl+Enter puts it in the prompt where your cursor was.",
                    )
                    .small(),
                );
                let h = (ui.available_height() - 44.0).max(120.0);
                egui::ScrollArea::vertical().max_height(h).show(ui, |ui| {
                    let r = ui.add(
                        full_width_edit(&mut self.fx.pad_text, pad_id())
                            .desired_rows(14)
                            .hint_text("paste here…"),
                    );
                    if self.fx.focus_pad > 0 {
                        r.request_focus();
                        self.fx.focus_pad -= 1;
                        ctx.request_repaint();
                    }
                    line_mover(ui, &r, &mut self.fx.pad_text);
                });
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, "Insert  (Ctrl+Enter)").clicked() {
                        insert = true;
                    }
                    if theme::ghost_button(ui, "Clear").clicked() {
                        self.fx.pad_text.clear();
                    }
                    if theme::ghost_button(ui, "Close  (Esc keeps the draft)").clicked() {
                        close = true;
                    }
                });
            });
        if insert {
            self.insert_pad(ctx);
        } else if close {
            self.fx.pad_open = false;
        }
    }

    // ------------------------------------------------------------------
    // version control
    // ------------------------------------------------------------------

    pub(super) fn refresh_tracking(&mut self) {
        git::prune(&self.dir);
        self.fx.tracked = git::tracked_list(&self.dir).into_iter().map(|r| self.dir.join(r)).collect();
        self.refresh_dirty();
    }

    pub(super) fn refresh_dirty(&mut self) {
        self.fx.dirty = self.is_tracked(&self.doc.path.clone()) && git::is_dirty(&self.dir, &self.doc.path);
    }

    pub(super) fn is_tracked(&self, path: &Path) -> bool {
        self.fx.tracked.contains(path)
    }

    pub(super) fn in_temp(&self) -> bool {
        self.current_folder().as_deref() == Some(store::TEMP)
    }

    /// A prompt file moved or was renamed: carry its history along (or drop tracking when it enters temp).
    pub(super) fn tracked_moved(&mut self, old: &Path, new: &Path) {
        if !self.is_tracked(old) {
            return;
        }
        if store::folder_of(&self.dir, new).as_deref() == Some(store::TEMP) {
            git::untrack(&self.dir, old);
            self.toast("The temp folder is not version-controlled — stopped tracking");
        } else if let Err(e) = git::rename(&self.dir, old, new) {
            self.toast_error(format!("History could not follow the move: {e}"));
        }
        self.refresh_tracking();
    }

    pub(super) fn set_tracking(&mut self, ctx: &Context, on: bool) {
        if on {
            self.autosave(ctx, true);
            if !self.doc.path.exists() {
                self.toast_error("Write something first — the prompt has no file yet");
                return;
            }
            match git::track(&self.dir, &self.doc.path) {
                Ok(()) => self.toast("Tracking this prompt — Ctrl+K commits it"),
                Err(e) => self.toast_error(format!("Could not start tracking: {e}")),
            }
        } else {
            git::untrack(&self.dir, &self.doc.path);
            self.toast("Stopped tracking (history is kept)");
        }
        self.refresh_tracking();
    }

    pub(super) fn open_commit(&mut self, ctx: &Context) {
        if !self.is_tracked(&self.doc.path.clone()) {
            self.toast_error("Not tracked — tick “track” next to the prompt name to keep its history");
            return;
        }
        self.autosave(ctx, true);
        self.refresh_dirty();
        if !self.fx.dirty {
            self.toast("Nothing new to commit");
            return;
        }
        self.fx.commit = Some(String::new());
        self.fx.commit_focus = 3;
    }

    pub(super) fn commit_dialog(&mut self, ctx: &Context) {
        let Some(mut msg) = self.fx.commit.take() else { return };
        let mut go = false;
        let mut cancel = false;
        egui::Window::new("Commit changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(theme::muted(format!("Only “{}” is committed.", self.doc.name)));
                let r = ui.add(
                    TextEdit::singleline(&mut msg)
                        .id(commit_input_id())
                        .desired_width(340.0)
                        .hint_text("what changed?  (Enter to commit)"),
                );
                if self.fx.commit_focus > 0 {
                    r.request_focus();
                    self.fx.commit_focus -= 1;
                    ctx.request_repaint();
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    go = true;
                }
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, "Commit").clicked() {
                        go = true;
                    }
                    if theme::ghost_button(ui, "Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            cancel = true;
        }
        if go {
            match git::commit(&self.dir, &self.doc.path, &msg) {
                Ok(true) => self.toast("Committed"),
                Ok(false) => self.toast("Nothing new to commit"),
                Err(e) => self.toast_error(format!("Commit failed: {e}")),
            }
            self.refresh_dirty();
        } else if !cancel {
            self.fx.commit = Some(msg);
        }
    }

    pub(super) fn open_rollback(&mut self, ctx: &Context, path: PathBuf, name: String) {
        if path == self.doc.path {
            self.autosave(ctx, true);
        }
        let versions = git::history(&self.dir, &path);
        self.fx.rollback = Some(Rollback { path, name, versions, confirm: None });
    }

    pub(super) fn rollback_window(&mut self, ctx: &Context) {
        let Some(mut rb) = self.fx.rollback.take() else { return };
        let mut close = false;
        let mut restore: Option<usize> = None;
        egui::Window::new(format!("Rollback — {}", rb.name))
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(460.0, 320.0))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| match &rb.versions {
                Err(e) => {
                    ui.label(RichText::new(format!("Could not read the history: {e}")).color(theme::DANGER));
                    if theme::tool_button(ui, "Close").clicked() {
                        close = true;
                    }
                }
                Ok(list) if list.is_empty() => {
                    ui.label(theme::muted("No versions yet — press Ctrl+K to commit one."));
                    if theme::tool_button(ui, "Close").clicked() {
                        close = true;
                    }
                }
                Ok(list) => {
                    ui.label(
                        theme::muted("Restoring saves your current state first, so a rollback can always be undone.")
                            .small(),
                    );
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        for (i, v) in list.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(theme::muted(v.local.clone()).small());
                                ui.add(Label::new(&v.subject).truncate());
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if rb.confirm == Some(i) {
                                        if ui
                                            .add(egui::Button::new(RichText::new("Confirm restore").color(theme::WARN)))
                                            .clicked()
                                        {
                                            restore = Some(i);
                                        }
                                    } else if theme::tool_button(ui, "Restore").clicked() {
                                        rb.confirm = Some(i);
                                    }
                                });
                            });
                        }
                    });
                    if theme::ghost_button(ui, "Close").clicked() {
                        close = true;
                    }
                }
            });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            close = true;
        }
        if let (Some(i), Ok(list)) = (restore, &rb.versions) {
            let v = list[i].clone();
            match git::rollback(&self.dir, &rb.path, &v) {
                Ok(()) => {
                    if rb.path == self.doc.path {
                        let p = rb.path.clone();
                        self.open_path(&p);
                    }
                    self.toast(format!("Restored the version from {}", v.local));
                }
                Err(e) => self.toast_error(format!("Rollback failed: {e}")),
            }
            self.refresh_tracking();
            close = true;
        }
        if !close {
            self.fx.rollback = Some(rb);
        }
    }
}

// ---------------------------------------------------------------------
// reusable contexts
// ---------------------------------------------------------------------

impl PromptEditorApp {
    /// `+ Context`: ask which one to use when there are saved ones, otherwise just add an empty one.
    pub(super) fn add_context_clicked(&mut self) {
        if store::list_contexts(&self.dir).is_empty() {
            self.add_context_text(String::new());
        } else {
            self.fx.ctx_chooser = true;
        }
    }

    fn add_context_text(&mut self, text: String) {
        self.doc.prompt.context = Some(text);
        self.view = View::Editor;
        self.st.focus_request = Some(context_id());
    }

    pub(super) fn open_context_manager(&mut self) {
        self.fx.ctx_dialog = Some(CtxDialog::default());
    }

    pub(super) fn context_chooser(&mut self, ctx: &Context) {
        if !self.fx.ctx_chooser {
            return;
        }
        let names = store::list_contexts(&self.dir);
        let mut pick: Option<Option<String>> = None; // Some(None) = empty context
        let mut close = false;
        egui::Window::new("Add a context")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                if theme::primary_button(ui, "Empty context").clicked() {
                    pick = Some(None);
                }
                ui.add_space(6.0);
                ui.label(theme::muted("…or start from a reusable one (it is copied in, so you can tweak it):").small());
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    for n in &names {
                        let snippet = store::read_context(&self.dir, n)
                            .ok()
                            .and_then(|t| t.lines().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_owned()))
                            .unwrap_or_default();
                        ui.horizontal(|ui| {
                            if theme::tool_button(ui, n).clicked() {
                                pick = Some(Some(n.clone()));
                            }
                            ui.add(Label::new(theme::muted(snippet).small()).truncate());
                        });
                    }
                });
                if theme::ghost_button(ui, "Cancel").clicked() {
                    close = true;
                }
            });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            close = true;
        }
        if let Some(choice) = pick {
            let text = match choice {
                None => String::new(),
                Some(n) => store::read_context(&self.dir, &n).unwrap_or_default(),
            };
            self.add_context_text(text);
            close = true;
        }
        if close {
            self.fx.ctx_chooser = false;
        }
    }

    /// File > New > Reusable context.
    pub(super) fn context_manager(&mut self, ctx: &Context) {
        let Some(mut d) = self.fx.ctx_dialog.take() else { return };
        let names = store::list_contexts(&self.dir);
        let mut close = false;
        let mut save = false;
        let mut edit: Option<String> = None;
        let mut delete: Option<String> = None;
        egui::Window::new("Reusable context")
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(520.0, 420.0))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(
                    theme::muted("A context you can drop into any prompt with “+ Context”. Saved on this computer, not inside any prompt.")
                        .small(),
                );
                ui.add(
                    TextEdit::singleline(&mut d.name)
                        .desired_width(f32::INFINITY)
                        .hint_text("name, e.g. “code reviewer”"),
                );
                egui::ScrollArea::vertical().id_salt("ctx-text").max_height(220.0).show(ui, |ui| {
                    ui.add(
                        full_width_edit(&mut d.text, Id::new("ctx-dialog-text"))
                            .desired_rows(9)
                            .hint_text("the context text…"),
                    );
                });
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, "Save").clicked() {
                        save = true;
                    }
                    if theme::ghost_button(ui, "Close").clicked() {
                        close = true;
                    }
                });
                if !names.is_empty() {
                    ui.separator();
                    ui.label(theme::muted("Saved").small());
                    egui::ScrollArea::vertical().id_salt("ctx-list").max_height(120.0).show(ui, |ui| {
                        for n in &names {
                            ui.horizontal(|ui| {
                                ui.label(n);
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if d.confirm_delete.as_deref() == Some(n.as_str()) {
                                        if ui.add(egui::Button::new(RichText::new("Delete for good").color(theme::WARN))).clicked() {
                                            delete = Some(n.clone());
                                        }
                                    } else if theme::ghost_button(ui, "Delete").clicked() {
                                        d.confirm_delete = Some(n.clone());
                                    }
                                    if theme::ghost_button(ui, "Edit").clicked() {
                                        edit = Some(n.clone());
                                    }
                                });
                            });
                        }
                    });
                }
            });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            close = true;
        }
        if let Some(n) = edit {
            d.text = store::read_context(&self.dir, &n).unwrap_or_default();
            d.name = n;
            d.confirm_delete = None;
        }
        if let Some(n) = delete {
            match store::delete_context(&self.dir, &n) {
                Ok(()) => self.toast(format!("Deleted the context “{n}”")),
                Err(e) => self.toast_error(format!("Could not delete: {e}")),
            }
            d.confirm_delete = None;
        }
        if save {
            if d.text.trim().is_empty() {
                self.toast_error("The context is empty — write something first");
            } else {
                match store::write_context(&self.dir, &d.name, &d.text) {
                    Ok(name) => {
                        self.toast(format!("Saved the context “{name}”"));
                        d = CtxDialog::default();
                    }
                    Err(e) => self.toast_error(format!("Could not save: {e}")),
                }
            }
        }
        if !close {
            self.fx.ctx_dialog = Some(d);
        }
    }
}
