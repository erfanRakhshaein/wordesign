use eframe::egui::{
    self, Align, Context, Frame, Id, Key, Label, Layout, Modifiers, RichText, ScrollArea, Sense,
    TextEdit, Ui, Vec2,
};
mod extras;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::model::{estimate_tokens, Answer, Benchmark, BodyBlock, Definition, Prompt, Rule};
use crate::settings::{self, Settings};
use crate::{cheatsheet, diagram, fonts, linemove, mdview, pricing, preview, spell, store, textdir, theme};

const AUTOSAVE_DELAY: Duration = Duration::from_millis(700);
const CONFIRM_WINDOW: Duration = Duration::from_secs(3);
const TOAST_TIME: Duration = Duration::from_secs(3);
const MAX_COLUMN_WIDTH: f32 = 860.0;
/// How often the running app re-checks the temp folder for expired prompts.
const PURGE_EVERY: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Editor,
    Variables,
    Benchmarks,
}

/// Something the user asked to delete for good (awaiting confirmation).
#[derive(Clone, PartialEq, Eq)]
enum DeleteTarget {
    Folder(String),
    Prompt { path: PathBuf, name: String },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Remove {
    Context,
    Definitions,
    Rules,
    Benchmark(usize),
}

struct PendingExample {
    rule: usize,
    text: String,
    needs_focus: bool,
}

/// The inline "new folder" box in the sidebar.
struct NewFolder {
    text: String,
    /// Frames left in which to (re)request focus; a click in the same frame would otherwise steal it.
    needs_focus: u8,
}

/// Transient UI state that is not part of the prompt itself.
#[derive(Default)]
struct UiState {
    pending_example: Option<PendingExample>,
    confirm: Option<(Remove, Instant)>,
    focus_request: Option<Id>,
    /// Frames left in which `focus_request` is (re)applied.
    focus_left: u8,
    /// Variable cards the user has expanded (filled ones are collapsed unless listed here).
    var_open: HashSet<String>,
}

impl UiState {
    fn confirm_pending(&self, key: Remove) -> bool {
        matches!(self.confirm, Some((k, t)) if k == key && t.elapsed() < CONFIRM_WINDOW)
    }
}

struct OpenDoc {
    prompt: Prompt,
    /// File stem on disk (what the sidebar shows).
    name: String,
    path: PathBuf,
    /// Contents of the name box while it is being edited.
    name_edit: String,
    /// Serialized text last written to disk; `None` while the file does not exist yet.
    last_saved: Option<String>,
    changed_at: Option<Instant>,
    last_body_focus: usize,
}

pub struct PromptEditorApp {
    dir: PathBuf,
    entries: Vec<store::Entry>,
    doc: OpenDoc,
    view: View,
    show_preview: bool,
    spell: Option<spell::Checker>,
    preview_popup: Option<mdview::Action>,
    st: UiState,
    ctrl_enter: bool,
    toast: Option<(String, Instant)>,
    error: Option<String>,
    shown_title: String,
    asked_maximize: bool,
    folders: Vec<String>,
    collapsed: HashSet<String>,
    new_folder: Option<NewFolder>,
    diagrams: diagram::Diagrams,
    settings: Settings,
    fx: extras::Extras,
    prices: Vec<pricing::Price>,
    price_rx: Option<std::sync::mpsc::Receiver<Result<Vec<pricing::Price>, String>>>,
    price_note: String,
    cost_quote: Option<pricing::Quote>,
    /// Scroll requested by the vim keys, applied by whichever scroll area is on top.
    scroll_pending: f32,
    last_purge: Instant,
    /// Folder awaiting the user's confirmation to be deleted for good.
    pending_delete: Option<DeleteTarget>,
    toast_bad: bool,
}

// ---------------------------------------------------------------------
// ids
// ---------------------------------------------------------------------

fn pending_example_id() -> Id {
    Id::new("pending-example")
}
fn flow_src_id(flow: &str) -> Id {
    Id::new(("flow-src", flow))
}
fn flow_title_id(flow: &str) -> Id {
    Id::new(("flow-title", flow))
}
fn context_id() -> Id {
    Id::new("context-text")
}
fn def_term_id(i: usize) -> Id {
    Id::new(("def-term", i))
}
fn rule_id(i: usize) -> Id {
    Id::new(("rule", i))
}
fn bench_q_id(q: usize) -> Id {
    Id::new(("bench-q", q))
}
fn code_src_id(code: &str) -> Id {
    Id::new(("code-src", code.to_owned()))
}
fn code_title_id(code: &str) -> Id {
    Id::new(("code-title", code.to_owned()))
}
fn var_value_id(name: &str) -> Id {
    Id::new(("var-value", name.to_owned()))
}
fn name_id() -> Id {
    Id::new("prompt-name")
}
fn bench_expected_id(q: usize) -> Id {
    Id::new(("bench-expected", q))
}
fn bench_model_id(q: usize, a: usize) -> Id {
    Id::new(("bench-model", q, a))
}
fn bench_answer_id(q: usize, a: usize) -> Id {
    Id::new(("bench-answer", q, a))
}

/// If a toolbar action asked for focus on this widget, grant it (and scroll to it).
fn apply_focus(st: &mut UiState, resp: &egui::Response) {
    if st.focus_request == Some(resp.id) {
        // Ask for a few frames in a row: the click that triggered this may land in the same frame and steal focus.
        if st.focus_left == 0 {
            st.focus_left = 3;
            resp.scroll_to_me(Some(Align::Center));
        }
        resp.request_focus();
        st.focus_left -= 1;
        if st.focus_left == 0 {
            st.focus_request = None;
        } else {
            resp.ctx.request_repaint();
        }
    }
}

/// Alt+Up / Alt+Down (taken in `logic`) moves the lines of the focused multiline editor.
fn line_mover(ui: &Ui, resp: &egui::Response, text: &mut String) {
    use egui::text::{CCursor, CCursorRange};
    // Find & replace jumped to a match in this editor: focus it and scroll it into view.
    if ui.ctx().data(|d| d.get_temp::<Id>(Id::new("find-focus"))) == Some(resp.id) {
        resp.request_focus();
        resp.scroll_to_me(Some(Align::Center));
        ui.ctx().data_mut(|d| d.remove::<Id>(Id::new("find-focus")));
    }
    if !resp.has_focus() {
        return;
    }
    let ctx = ui.ctx();
    let Some(up) = ctx.data(|d| d.get_temp::<bool>(Id::new("line-move"))) else {
        return;
    };
    let Some(mut state) = TextEdit::load_state(ctx, resp.id) else {
        return;
    };
    let (a, h) = state
        .cursor
        .char_range()
        .map(|r| (r.secondary.index.0, r.primary.index.0))
        .unwrap_or((0, 0));
    if let Some((new, a, h)) = linemove::move_lines(text, a, h, up) {
        *text = new;
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(h))));
        state.store(ctx, resp.id);
        resp.request_focus();
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.remove::<bool>(Id::new("line-move")));
}

fn full_width_edit<'a>(text: &'a mut String, id: Id) -> TextEdit<'a> {
    TextEdit::multiline(text)
        .id(id)
        .frame(Frame::NONE)
        .desired_width(f32::INFINITY)
}

// ---------------------------------------------------------------------
// app
// ---------------------------------------------------------------------

/// egui's keyboard zoom moves in steps of 0.1, so five Ctrl+Plus presses = 1.5.
const DEFAULT_ZOOM: f32 = 1.5;

impl PromptEditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        // Handwriting UI font + system fallbacks for CJK / Arabic / ... (shared with the diagram renderer).
        let font_db = std::sync::Arc::new(fonts::load_database());
        fonts::install(&cc.egui_ctx, &font_db);

        let mut error = None;
        let dir = store::base_dir().unwrap_or_else(|e| {
            error = Some(format!("Could not use ~/.wordesign ({e}); using a temporary folder"));
            let d = std::env::temp_dir().join("wordesign");
            let _ = std::fs::create_dir_all(&d);
            d
        });
        let settings = Settings::load(&dir);
        theme::set_accent(egui::Color32::from_rgb(settings.accent[0], settings.accent[1], settings.accent[2]));
        theme::apply(&cc.egui_ctx);
        // The scale the user last zoomed to; otherwise the same as pressing Ctrl+Plus five times.
        cc.egui_ctx.set_zoom_factor(settings.zoom.unwrap_or(DEFAULT_ZOOM));
        if let Some(age) = settings.temp_max_age() {
            store::purge_temp(&dir, age, None);
        }
        let library = store::scan(&dir);
        let (prices, price_note) = match pricing::load_cache(&dir) {
            Some(p) => (p, "saved prices".to_owned()),
            None => (pricing::snapshot(), "bundled prices".to_owned()),
        };
        let dir_for_prices = dir.clone();

        let mut app = Self {
            doc: Self::blank_doc(&dir, &[]),
            dir,
            entries: library.entries,
            collapsed: library.folders.iter().cloned().collect(),
            folders: library.folders,
            new_folder: None,
            diagrams: diagram::Diagrams::new(font_db),
            settings,
            fx: extras::Extras::default(),
            prices,
            price_rx: Some(pricing::spawn_refresh(&dir_for_prices)),
            price_note,
            cost_quote: None,
            scroll_pending: 0.0,
            last_purge: Instant::now(),
            pending_delete: None,
            toast_bad: false,
            view: View::Editor,
            show_preview: false,
            spell: None,
            preview_popup: None,
            st: UiState::default(),
            ctrl_enter: false,
            toast: None,
            error,
            shown_title: String::new(),
            asked_maximize: false,
        };
        // Resume the most recently edited prompt; otherwise start a fresh note.
        if let Some(first) = app.entries.first().cloned() {
            app.open_path(&first.path);
        }
        // First launch: the cheatsheet, already open in the preview.
        if !app.settings.cheatsheet_seen {
            app.settings.cheatsheet_seen = true;
            app.settings.save(&app.dir);
            app.open_cheatsheet();
        }
        app.refresh_tracking();
        app
    }

    /// Open the built-in cheatsheet (creating it in "no folder" if it is missing) and show it in the preview.
    fn open_cheatsheet(&mut self) {
        let path = store::path_for(&self.dir, cheatsheet::NAME);
        if !path.exists() {
            if let Err(e) = store::write_atomic(&path, &cheatsheet::prompt().to_file()) {
                self.error = Some(format!("Could not create the cheatsheet: {e}"));
                return;
            }
        }
        self.refresh_entries();
        self.open_path(&path);
        self.show_preview = true;
    }

    /// Pick up the freshly downloaded price table, if it has arrived.
    fn poll_prices(&mut self, ctx: &Context) {
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = &self.price_rx else { return };
        match rx.try_recv() {
            Ok(Ok(p)) => {
                self.prices = p;
                self.price_note = "live prices from launch".to_owned();
                self.price_rx = None;
                self.cost_quote = None;
            }
            Ok(Err(_)) => {
                self.price_note = format!("offline, using {}", self.price_note);
                self.price_rx = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(500)),
            Err(TryRecvError::Disconnected) => self.price_rx = None,
        }
    }

    /// Remember the scale the user zoomed to (Ctrl+Plus / Ctrl+Minus) for the next launch.
    fn save_zoom_if_changed(&mut self, ctx: &Context) {
        let z = ctx.zoom_factor();
        let saved = self.settings.zoom.unwrap_or(DEFAULT_ZOOM);
        if (z - saved).abs() > 0.001 {
            self.settings.zoom = Some(z);
            self.settings.save(&self.dir);
        }
    }

    fn blank_doc(dir: &std::path::Path, taken: &[&str]) -> OpenDoc {
        let name = store::default_stem(dir, taken);
        OpenDoc {
            prompt: Prompt::new(),
            path: store::unique_path(dir, &name, None),
            name_edit: name.clone(),
            name,
            last_saved: None,
            changed_at: None,
            last_body_focus: 0,
        }
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
        self.toast_bad = false;
    }

    fn toast_error(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
        self.toast_bad = true;
    }

    fn refresh_entries(&mut self) {
        let library = store::scan(&self.dir);
        self.entries = library.entries;
        self.folders = library.folders;
    }

    // ----- persistence -------------------------------------------------

    /// Write the current prompt if it changed. Debounced unless `force`.
    fn autosave(&mut self, ctx: &Context, force: bool) {
        let ser = self.doc.prompt.to_file();
        let unchanged = self.doc.last_saved.as_deref() == Some(ser.as_str());
        let untouched_new = self.doc.last_saved.is_none() && self.doc.prompt.is_blank();
        if unchanged || untouched_new {
            self.doc.changed_at = None;
            return;
        }
        let now = Instant::now();
        let since = *self.doc.changed_at.get_or_insert(now);
        let elapsed = now.duration_since(since);
        if !force && elapsed < AUTOSAVE_DELAY {
            ctx.request_repaint_after(AUTOSAVE_DELAY - elapsed + Duration::from_millis(30));
            return;
        }
        match store::write_atomic(&self.doc.path, &ser) {
            Ok(()) => {
                self.doc.last_saved = Some(ser);
                self.doc.changed_at = None;
                self.error = None;
                self.refresh_entries();
                if self.is_tracked(&self.doc.path.clone()) {
                    self.refresh_dirty();
                }
            }
            Err(e) => {
                self.error = Some(format!("Autosave failed: {e}"));
                self.doc.changed_at = Some(now); // retry after another delay
                ctx.request_repaint_after(AUTOSAVE_DELAY);
            }
        }
    }

    fn open_path(&mut self, path: &std::path::Path) {
        match store::load(path) {
            Ok(prompt) => {
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                self.doc = OpenDoc {
                    last_saved: Some(prompt.to_file()),
                    prompt,
                    name_edit: name.clone(),
                    name,
                    path: path.to_path_buf(),
                    changed_at: None,
                    last_body_focus: 0,
                };
                self.st = UiState::default();
                self.view = View::Editor;
                self.error = None;
                self.refresh_dirty();
            }
            Err(e) => self.error = Some(format!("Could not open {}: {e}", path.display())),
        }
    }

    fn switch_to(&mut self, ctx: &Context, path: PathBuf) {
        if path == self.doc.path {
            return;
        }
        self.autosave(ctx, true);
        self.open_path(&path);
    }

    fn current_folder(&self) -> Option<String> {
        if self.is_template_doc() {
            return None;
        }
        store::folder_of(&self.dir, &self.doc.path)
    }

    /// New prompt next to the one that is open (same folder).
    fn new_doc_in(&mut self, ctx: &Context, folder: Option<String>) {
        self.autosave(ctx, true);
        // An untouched blank note is never written, so repeated clicks reuse its name.
        let dir = store::folder_dir(&self.dir, folder.as_deref());
        self.doc = Self::blank_doc(&dir, &[]);
        self.st = UiState::default();
        self.view = View::Editor;
        self.refresh_entries();
        // Name first; Enter then moves on to the body.
        self.st.focus_request = Some(name_id());
        self.fx.select_name = true;
        self.refresh_dirty();
    }

    fn commit_rename(&mut self) {
        let stem = store::sanitize(&self.doc.name_edit);
        if stem.is_empty() || stem == self.doc.name {
            self.doc.name_edit = self.doc.name.clone();
            return;
        }
        let dir = self
            .doc
            .path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.dir.clone());
        let new_path = store::unique_path(&dir, &stem, Some(&self.doc.path));
        if new_path == self.doc.path {
            self.doc.name_edit = self.doc.name.clone();
            return;
        }
        if self.doc.path.exists() {
            if let Err(e) = std::fs::rename(&self.doc.path, &new_path) {
                self.error = Some(format!("Rename failed: {e}"));
                self.doc.name_edit = self.doc.name.clone();
                return;
            }
        }
        self.doc.name = new_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        self.doc.name_edit = self.doc.name.clone();
        let old_path = std::mem::replace(&mut self.doc.path, new_path.clone());
        self.tracked_moved(&old_path, &new_path);
        self.refresh_entries();
    }

    /// Put the open prompt into `folder` (`None` = no folder).
    fn assign_folder(&mut self, ctx: &Context, folder: Option<String>) {
        if folder == self.current_folder() {
            return;
        }
        self.autosave(ctx, true); // the file must exist and be current before it is moved
        let old = self.doc.path.clone();
        let new_path = if old.exists() {
            match store::move_to_folder(&self.dir, &old, folder.as_deref()) {
                Ok(p) => p,
                Err(e) => {
                    self.toast(format!("Could not move prompt: {e}"));
                    return;
                }
            }
        } else {
            // Still an empty note, nothing on disk: just choose where it will be saved.
            store::retarget(&self.dir, &old, folder.as_deref())
        };
        self.doc.name = new_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        self.doc.name_edit = self.doc.name.clone();
        let moved_from = std::mem::replace(&mut self.doc.path, new_path.clone());
        self.tracked_moved(&moved_from, &new_path);
        self.refresh_entries();
        let place = folder.unwrap_or_else(|| "no folder".to_string());
        self.toast(format!("Moved to {place}"));
    }

    fn commit_new_folder(&mut self) {
        let Some(nf) = self.new_folder.take() else { return };
        if nf.text.trim().is_empty() {
            return;
        }
        match store::create_folder(&self.dir, &nf.text) {
            Ok(name) => {
                self.collapsed.remove(&name);
                self.refresh_entries();
                self.toast(format!("Folder “{name}” is ready"));
            }
            Err(e) => self.toast(format!("Could not create folder: {e}")),
        }
    }

    fn export(&mut self, ctx: &Context) {
        let missing = self.doc.prompt.unaddressed_variables();
        if !missing.is_empty() {
            let list = missing.iter().map(|n| format!("{{{{{n}}}}}")).collect::<Vec<_>>().join(", ");
            self.toast_error(format!(
                "Export blocked: {} unaddressed variable{} ({list}) — give {} a value first",
                missing.len(),
                if missing.len() == 1 { "" } else { "s" },
                if missing.len() == 1 { "it" } else { "them" },
            ));
            if !self.show_preview {
                self.view = View::Variables;
                self.st.var_open.insert(missing[0].clone());
                self.st.focus_request = Some(var_value_id(&missing[0]));
            }
            return;
        }
        let out = self.doc.prompt.export_minified();
        if out.is_empty() {
            self.toast("Nothing to export yet");
        } else {
            let n = out.chars().count();
            ctx.copy_text(out);
            self.toast(format!("Copied minimal prompt to clipboard ({n} chars)"));
        }
    }

    // ----- Ctrl+Enter ----------------------------------------------------

    /// Ctrl+Enter submits whatever is being edited (a flow, a new example),
    /// otherwise it toggles the prompt preview from the main editor.
    fn dispatch_ctrl_enter(&mut self, ctx: &Context) {
        if self.fx.pad_open {
            self.insert_pad(ctx);
            return;
        }
        let focused = ctx.memory(|m| m.focused());
        if let Some(f) = focused {
            if f == pending_example_id() {
                if let Some(pe) = self.st.pending_example.take() {
                    let text = pe.text.trim().to_string();
                    if !text.is_empty() {
                        if let Some(rule) = self.doc.prompt.rules.as_mut().and_then(|r| r.get_mut(pe.rule)) {
                            rule.examples.push(text);
                        }
                    }
                }
                ctx.memory_mut(|m| m.surrender_focus(f));
                return;
            }
            for b in &mut self.doc.prompt.body {
                match b {
                    BodyBlock::Flow(fl) if !fl.collapsed && (f == flow_src_id(&fl.id) || f == flow_title_id(&fl.id)) => {
                        fl.collapsed = true;
                        ctx.memory_mut(|m| m.surrender_focus(f));
                        return;
                    }
                    BodyBlock::Code(c) if !c.collapsed && (f == code_src_id(&c.id) || f == code_title_id(&c.id)) => {
                        c.collapsed = true;
                        ctx.memory_mut(|m| m.surrender_focus(f));
                        return;
                    }
                    _ => {}
                }
            }
            // A variable's value card collapses the same way.
            if let Some(name) = self.doc.prompt.used_variables().into_iter().find(|n| var_value_id(n) == f) {
                if self.doc.prompt.variable_value(&name).is_some_and(|v| !v.trim().is_empty()) {
                    self.st.var_open.remove(&name);
                    ctx.memory_mut(|m| m.surrender_focus(f));
                }
                return;
            }
        }
        if matches!(self.view, View::Editor | View::Variables) {
            self.show_preview = !self.show_preview;
        }
    }
}

impl eframe::App for PromptEditorApp {
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Consume before any widget sees it, so a multiline editor never inserts a newline.
        self.ctrl_enter = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
        self.extras_shortcuts(ctx);
        self.extras_escape(ctx);
        self.vim_keys(ctx);
        // Ctrl+E exports from anywhere, including the preview.
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::E)) {
            self.export(ctx);
        }
        // Alt+Up / Alt+Down: handed to whichever multiline editor has focus (see `line_mover`).
        ctx.data_mut(|d| d.remove::<bool>(Id::new("line-move")));
        // Alt+V: type `{{}}` into the focused text field with the cursor between the braces.
        // Done with input events so it happens in order with whatever else is typed in the same frame.
        if ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::V)) {
            let focused = ctx.memory(|m| m.focused());
            let state = focused.and_then(|id| TextEdit::load_state(ctx, id));
            if let Some(state) = state {
                let has_selection = state.cursor.char_range().is_some_and(|r| r.primary != r.secondary);
                let key = |k: Key| egui::Event::Key {
                    key: k,
                    physical_key: Some(k),
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                };
                ctx.input_mut(|i| {
                    // Some platforms also report the "v" that Alt+V would type: drop it.
                    i.events
                        .retain(|e| !matches!(e, egui::Event::Text(t) if t.eq_ignore_ascii_case("v")));
                    if has_selection {
                        i.events.push(key(Key::ArrowRight)); // go past the selection instead of replacing it
                    }
                    i.events.push(egui::Event::Text("{{}}".to_owned()));
                    i.events.push(key(Key::ArrowLeft));
                    i.events.push(key(Key::ArrowLeft));
                });
            }
        }
        let up = ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowUp));
        let down = ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowDown));
        if up || down {
            ctx.data_mut(|d| d.insert_temp(Id::new("line-move"), up));
        }
        if self.show_preview && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            // Esc closes the word popup first, then the preview.
            if self.preview_popup.take().is_none() {
                self.show_preview = false;
            }
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Some window managers ignore the "start maximized" hint, so ask once more after the window exists.
        if !std::mem::replace(&mut self.asked_maximize, true) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }

        if std::mem::take(&mut self.ctrl_enter) {
            self.dispatch_ctrl_enter(&ctx);
        }

        let title = match self.current_folder() {
            Some(f) => format!("{f} / {} — wordesign", self.doc.name),
            None => format!("{} — wordesign", self.doc.name),
        };
        if title != self.shown_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.shown_title = title;
        }

        if self.last_purge.elapsed() >= PURGE_EVERY {
            self.last_purge = Instant::now();
            self.purge_temp_now(false);
        }
        ctx.request_repaint_after(PURGE_EVERY);

        egui::Panel::top("menubar")
            .frame(
                Frame::new()
                    .fill(theme::bg_panel())
                    .inner_margin(egui::Margin::symmetric(8, 2)),
            )
            .show_separator_line(false)
            .show(ui, |ui| self.menubar(ui, &ctx));

        egui::Panel::top("toolbar")
            .frame(
                Frame::new()
                    .fill(theme::bg_panel())
                    .inner_margin(egui::Margin::symmetric(12, 10)),
            )
            .show_separator_line(false)
            .show(ui, |ui| self.toolbar(ui, &ctx));

        egui::Panel::bottom("status")
            .frame(
                Frame::new()
                    .fill(theme::bg_panel())
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show_separator_line(false)
            .show(ui, |ui| self.status_bar(ui));

        egui::Panel::left("prompts")
            .resizable(false)
            .exact_size(240.0)
            .frame(
                Frame::new()
                    .fill(theme::bg_panel())
                    .inner_margin(egui::Margin::symmetric(10, 10)),
            )
            .show_separator_line(false)
            .show(ui, |ui| self.sidebar(ui, &ctx));

        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme::bg_panel()).inner_margin(egui::Margin {
                left: 10,
                right: 10,
                top: 4,
                bottom: 10,
            }))
            .show(ui, |ui| {
                let Self { doc, st, view, scroll_pending, show_preview, .. } = self;
                let dy = if *show_preview { 0.0 } else { std::mem::take(scroll_pending) };
                let pending = doc.prompt.unaddressed_variables().len();
                let canvas = tabs_and_canvas(ui, view, pending);
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(canvas.shrink2(Vec2::new(10.0, 12.0))),
                    |ui| {
                        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            if dy != 0.0 {
                                ui.scroll_with_delta(Vec2::new(0.0, dy));
                            }
                            let avail = ui.available_width();
                            let w = (avail - 24.0).clamp(280.0, MAX_COLUMN_WIDTH);
                            let pad = ((avail - w) / 2.0).max(8.0);
                            ui.horizontal_top(|ui| {
                                ui.add_space(pad);
                                ui.vertical(|ui| {
                                    ui.set_width(w);
                                    match *view {
                                        View::Editor => editor_column(ui, doc, st),
                                        View::Variables => variables_column(ui, doc, st),
                                        View::Benchmarks => benchmarks_column(ui, doc, st),
                                    }
                                });
                            });
                        });
                    },
                );
            });

        self.preview_window(&ctx);
        self.delete_dialog(&ctx);
        self.pad_window(&ctx);
        self.find_bar(&ctx);
        self.commit_dialog(&ctx);
        self.new_prompt_dialog(&ctx);
        self.save_template_dialog(&ctx);
        self.context_chooser(&ctx);
        self.context_manager(&ctx);
        self.poll_prices(&ctx);
        self.save_zoom_if_changed(&ctx);
        self.rollback_window(&ctx);
        self.autosave(&ctx, false);
        if self.toast.as_ref().is_some_and(|(_, t)| t.elapsed() >= TOAST_TIME) {
            self.toast = None;
        } else if self.toast.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Final flush: never lose the last keystrokes.
        let ser = self.doc.prompt.to_file();
        let unchanged = self.doc.last_saved.as_deref() == Some(ser.as_str());
        let untouched_new = self.doc.last_saved.is_none() && self.doc.prompt.is_blank();
        if !unchanged && !untouched_new {
            let _ = store::write_atomic(&self.doc.path, &ser);
        }
    }
}

// ---------------------------------------------------------------------
// chrome: toolbar, sidebar, status bar, preview
// ---------------------------------------------------------------------

impl PromptEditorApp {
    /// Conventional top-side menu bar.
    fn menubar(&mut self, ui: &mut Ui, ctx: &Context) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                ui.menu_button("New", |ui| {
                    if ui.add(egui::Button::new("Prompt").shortcut_text("Ctrl+N")).clicked() {
                        let folder = self.current_folder();
                        self.request_new(ctx, folder);
                        ui.close();
                    }
                    if ui.button("Template prompt").clicked() {
                        self.new_template_doc(ctx);
                        ui.close();
                    }
                    if ui.button("Reusable context…").clicked() {
                        self.open_context_manager();
                        ui.close();
                    }
                });
                if ui
                    .add(egui::Button::new("Export (copy minimal prompt)").shortcut_text("Ctrl+E"))
                    .clicked()
                {
                    self.export(ctx);
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Preview").shortcut_text("Ctrl+Enter"))
                    .clicked()
                {
                    self.show_preview = !self.show_preview;
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.add(egui::Button::new("Find").shortcut_text("Ctrl+F")).clicked() {
                    self.open_find(false);
                    ui.close();
                }
                if ui.add(egui::Button::new("Replace").shortcut_text("Ctrl+H")).clicked() {
                    self.open_find(true);
                    ui.close();
                }
                if ui.add(egui::Button::new("Pad").shortcut_text("Ctrl+P")).clicked() {
                    self.toggle_pad(ctx);
                    ui.close();
                }
                ui.separator();
                if ui.add(egui::Button::new("Commit prompt").shortcut_text("Ctrl+K")).clicked() {
                    self.open_commit(ctx);
                    ui.close();
                }
            });
            ui.menu_button("Settings", |ui| {
                ui.menu_button("Appearance", |ui| {
                    ui.menu_button("Theme color", |ui| self.theme_color_menu(ui, ctx));
                });
                ui.menu_button("General", |ui| {
                    ui.label(theme::muted("Empty the temp folder").small());
                    let mut changed = false;
                    for d in settings::TEMP_CHOICES {
                        let label = if d == 0 {
                            "Never".to_owned()
                        } else {
                            format!("{} unmodified", settings::describe_days(d).replace("After ", ""))
                        };
                        if ui.radio(self.settings.temp_days == d, label).clicked() {
                            self.settings.temp_days = d;
                            changed = true;
                        }
                    }
                    ui.separator();
                    if ui.button("Empty temp now").clicked() {
                        self.purge_temp_now(true);
                        ui.close();
                    }
                    if changed {
                        self.settings.save(&self.dir);
                        self.purge_temp_now(false);
                        ui.close();
                    }
                });
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Cheatsheet").clicked() {
                    self.autosave(ctx, true);
                    self.open_cheatsheet();
                    ui.close();
                }
            });
        });
    }

    /// Settings > Appearance > Theme color: presets, a custom picker and a reset.
    fn theme_color_menu(&mut self, ui: &mut Ui, ctx: &Context) {
        const PRESETS: [(&str, [u8; 3]); 9] = [
            ("Blue (default)", settings::DEFAULT_ACCENT),
            ("Gray", settings::GRAY),
            ("Violet", settings::VIOLET),
            ("Magenta", [0xff, 0x4c, 0xc4]),
            ("Rose", [0xff, 0x5c, 0x7a]),
            ("Orange", [0xff, 0x9a, 0x3c]),
            ("Lime", [0x9c, 0xe0, 0x3a]),
            ("Green", [0x3d, 0xdc, 0x84]),
            ("Teal", [0x22, 0xd3, 0xc5]),
        ];
        let before = self.settings.accent;
        ui.label(theme::muted("Pick a color").small());
        ui.horizontal(|ui| {
            for (name, rgb) in PRESETS {
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                let col = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                ui.painter().circle_filled(rect.center(), 10.0, col);
                if self.settings.accent == rgb {
                    ui.painter().circle_stroke(rect.center(), 12.5, egui::Stroke::new(2.0, egui::Color32::WHITE));
                } else if resp.hovered() {
                    ui.painter().circle_stroke(rect.center(), 12.5, egui::Stroke::new(1.5, col));
                }
                if resp.on_hover_text(name).clicked() {
                    self.settings.accent = rgb;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Custom");
            egui::color_picker::color_edit_button_srgb(ui, &mut self.settings.accent);
        });
        if self.settings.accent != before {
            let a = self.settings.accent;
            theme::set_accent(egui::Color32::from_rgb(a[0], a[1], a[2]));
            theme::apply(ctx);
            self.settings.save(&self.dir);
        }
    }

    /// Delete expired prompts from the temp folder (`force` = everything, except the open prompt).
    fn purge_temp_now(&mut self, force: bool) {
        let age = if force {
            Some(Duration::ZERO)
        } else {
            self.settings.temp_max_age()
        };
        let Some(age) = age else { return };
        let open = self.doc.path.clone();
        let n = store::purge_temp(&self.dir, age, Some(&open));
        if n > 0 || force {
            self.refresh_entries();
            if force {
                self.toast(format!("Emptied temp ({n} prompt{})", if n == 1 { "" } else { "s" }));
            }
        }
    }

    /// Fresh blank note; the previous one is NOT saved (it was just deleted).
    fn reset_to_blank(&mut self, folder: Option<String>) {
        let dir = store::folder_dir(&self.dir, folder.as_deref());
        self.doc = Self::blank_doc(&dir, &[]);
        self.st = UiState::default();
        self.view = View::Editor;
        self.st.focus_request = Some(Id::new(("body-text", 0usize)));
    }

    /// Confirmation window for permanently deleting a folder or a prompt.
    fn delete_dialog(&mut self, ctx: &Context) {
        let Some(target) = self.pending_delete.clone() else { return };
        let (title, headline, detail) = match &target {
            DeleteTarget::Folder(name) => {
                let count = self.entries.iter().filter(|e| e.folder.as_deref() == Some(name.as_str())).count();
                (
                    "Delete folder",
                    format!("Permanently delete the folder “{name}”?"),
                    if count == 0 {
                        "It is empty.".to_owned()
                    } else {
                        format!(
                            "This also deletes the {count} prompt{} inside it. This cannot be undone.",
                            if count == 1 { "" } else { "s" }
                        )
                    },
                )
            }
            DeleteTarget::Prompt { name, .. } => (
                "Delete prompt",
                format!("Permanently delete the prompt “{name}”?"),
                "This cannot be undone.".to_owned(),
            ),
        };
        let mut close = false;
        let mut confirmed = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(headline);
                ui.label(theme::muted(detail));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new(RichText::new("Delete permanently").color(theme::WARN)))
                        .clicked()
                    {
                        confirmed = true;
                    }
                    if theme::ghost_button(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            close = true;
        }
        if confirmed {
            match &target {
                DeleteTarget::Folder(name) => {
                    // If the open prompt lives in that folder it is about to vanish.
                    if self.current_folder().as_deref() == Some(name.as_str()) {
                        self.reset_to_blank(None);
                    }
                    match store::delete_folder(&self.dir, name) {
                        Ok(()) => self.toast(format!("Deleted folder “{name}”")),
                        Err(e) => self.toast_error(format!("Could not delete: {e}")),
                    }
                    self.collapsed.remove(name);
                }
                DeleteTarget::Prompt { path, name } => {
                    let was_open = *path == self.doc.path;
                    let folder = self.current_folder();
                    // A prompt that was never written has no file; there is just nothing to remove.
                    let res = if path.exists() { store::delete_prompt(&self.dir, path) } else { Ok(()) };
                    match res {
                        Ok(()) => {
                            if was_open {
                                self.reset_to_blank(folder);
                            }
                            self.toast(format!("Deleted “{name}”"));
                        }
                        Err(e) => self.toast_error(format!("Could not delete: {e}")),
                    }
                }
            }
            self.refresh_entries();
            self.refresh_tracking();
            close = true;
        }
        if close {
            self.pending_delete = None;
        }
    }

    /// Top bar: the two tabs (+ preview / export on the right), and below them the name and the "add" buttons.
    fn toolbar(&mut self, ui: &mut Ui, ctx: &Context) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            let name = ui.add(
                { let al = textdir::align_for(&self.doc.name_edit); TextEdit::singleline(&mut self.doc.name_edit).horizontal_align(al) }
                    .desired_width(170.0)
                    .min_size(Vec2::new(170.0, theme::CONTROL_HEIGHT))
                    .id(name_id())
                    .vertical_align(Align::Center)
                    .hint_text("prompt name")
                    .font(egui::TextStyle::Button),
            );
            apply_focus(&mut self.st, &name);
            if self.fx.select_name && name.has_focus() {
                self.fx.select_name = false;
                if let Some(mut s) = TextEdit::load_state(ctx, name_id()) {
                    let n = self.doc.name_edit.chars().count();
                    s.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0usize),
                        egui::text::CCursor::new(n),
                    )));
                    s.store(ctx, name_id());
                }
            }
            if name.lost_focus() {
                let enter = ui.input(|i| i.key_pressed(Key::Enter));
                self.commit_rename();
                if enter {
                    self.st.focus_request = Some(Id::new(("body-text", 0usize)));
                }
            }
            // Version control: not for the throw-away temp folder.
            if !self.in_temp() {
                let mut on = self.is_tracked(&self.doc.path.clone());
                let before = on;
                ui.checkbox(&mut on, "track")
                    .on_hover_text("Keep this prompt's history with git. Ctrl+K commits it; right-click it in the sidebar to roll back.");
                if on != before {
                    self.set_tracking(ctx, on);
                }
            }

            ui.separator();

            match self.view {
                View::Editor => {
                    let p = &mut self.doc.prompt;
                    if theme::tool_button_enabled(ui, p.context.is_none(), "+ Context").clicked() {
                        self.add_context_clicked();
                    }
                    let p = &mut self.doc.prompt;
                    if theme::tool_button_enabled(ui, p.definitions.is_none(), "+ Definitions").clicked() {
                        p.definitions = Some(vec![Definition::default()]);
                        self.st.focus_request = Some(def_term_id(0));
                    }
                    if theme::tool_button_enabled(ui, p.rules.is_none(), "+ Rules").clicked() {
                        p.rules = Some(vec![Rule::default()]);
                        self.st.focus_request = Some(rule_id(0));
                    }
                    if theme::tool_button(ui, "+ Code")
                        .on_hover_text("Paste code after the block you are editing (exported inside a ``` fence)")
                        .clicked()
                    {
                        let after = self.doc.last_body_focus.min(p.body.len().saturating_sub(1));
                        let at = p.insert_code_after(after);
                        if let BodyBlock::Code(c) = &p.body[at] {
                            self.st.focus_request = Some(code_src_id(&c.id));
                        }
                    }
                    if theme::tool_button(ui, "+ Flow")
                        .on_hover_text("Insert a mermaid thought-flow after the block you are editing")
                        .clicked()
                    {
                        let after = self.doc.last_body_focus.min(p.body.len().saturating_sub(1));
                        let at = p.insert_flow_after(after);
                        if let BodyBlock::Flow(f) = &p.body[at] {
                            self.st.focus_request = Some(flow_title_id(&f.id));
                        }
                    }
                }
                View::Variables => {
                    ui.label(theme::muted("Alt+V inserts {{}} in any text field").small());
                }
                View::Benchmarks => {
                    if theme::tool_button(ui, "+ Question").clicked() {
                        let p = &mut self.doc.prompt;
                        p.benchmarks.push(Benchmark::default());
                        self.st.focus_request = Some(bench_q_id(p.benchmarks.len() - 1));
                    }
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::primary_button(ui, "Export")
                    .on_hover_text("Copy a minimal version of the prompt to the clipboard")
                    .clicked()
                {
                    self.export(ctx);
                }
                if theme::tool_button(ui, "Preview")
                    .on_hover_text("Ctrl+Enter")
                    .clicked()
                {
                    self.show_preview = !self.show_preview;
                }
                if !self.is_template_doc()
                    && theme::tool_button(ui, "Save as template")
                        .on_hover_text("Keep a copy of this prompt to start new prompts from")
                        .clicked()
                {
                    self.fx.tmpl_save = Some(self.doc.name.clone());
                }
            });
        });
    }

    fn sidebar(&mut self, ui: &mut Ui, ctx: &Context) {
        // New prompt at the very top.
        if theme::wide_button(ui, ui.available_width(), "+ New prompt")
            .on_hover_text("Start a new empty prompt (in the current folder). Drag prompts onto folders to organise them.")
            .clicked()
        {
            let folder = self.current_folder();
            self.request_new(ctx, folder);
        }
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            theme::small_caps_label(ui, "Prompts");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::ghost_button(ui, "+ Folder").on_hover_text("Create a folder").clicked() {
                    self.new_folder = Some(NewFolder {
                        text: String::new(),
                        needs_focus: 3,
                    });
                }
            });
        });
        ui.add_space(2.0);

        // Inline "new folder" box: Enter creates, Esc cancels.
        let mut finish = false;
        if let Some(nf) = self.new_folder.as_mut() {
            let resp = ui.add(
                { let al = textdir::align_for(&nf.text); TextEdit::singleline(&mut nf.text).horizontal_align(al) }
                    .hint_text("folder name, then Enter")
                    .desired_width(f32::INFINITY),
            );
            if nf.needs_focus > 0 {
                resp.request_focus();
                nf.needs_focus -= 1;
                ui.ctx().request_repaint();
            }
            if resp.lost_focus() {
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    nf.text.clear();
                }
                finish = true;
            }
            ui.add_space(4.0);
        }
        if finish {
            self.commit_new_folder();
        }

        // Group prompts by folder (the open one may not be on disk yet).
        type Row = (String, PathBuf);
        let cur = self.doc.path.clone();
        let cur_folder = self.current_folder();
        let mut grouped: Vec<(String, Vec<Row>)> = self.folders.iter().map(|f| (f.clone(), Vec::new())).collect();
        let mut loose: Vec<Row> = Vec::new();
        for e in &self.entries {
            let row = (e.name.clone(), e.path.clone());
            match e.folder.as_deref().and_then(|f| grouped.iter_mut().find(|(n, _)| n == f)) {
                Some((_, rows)) => rows.push(row),
                None => loose.push(row),
            }
        }
        if !self.entries.iter().any(|e| e.path == cur) && !self.is_template_doc() {
            let row = (self.doc.name.clone(), cur.clone());
            match cur_folder.as_deref().and_then(|f| grouped.iter_mut().find(|(n, _)| n == f)) {
                Some((_, rows)) => rows.insert(0, row),
                None => loose.insert(0, row),
            }
        }

        let mut open: Option<PathBuf> = None;
        let mut toggle: Option<String> = None;
        let mut new_in: Option<Option<String>> = None;
        let mut dropped: Option<(PathBuf, Option<String>)> = None;
        let mut ask_delete: Option<DeleteTarget> = None;
        let mut rollback: Option<(PathBuf, String)> = None;
        let mut empty_temp = false;
        let nav = self.fx.nav;
        let mut nav_items: Vec<extras::NavItem> = Vec::new();
        let mut nav_moved = std::mem::take(&mut self.fx.nav_moved);
        // Marks the row the keyboard cursor (`s`) is on.
        let mut mark = |ui: &mut Ui, item: extras::NavItem, top: f32, items: &mut Vec<extras::NavItem>| {
            let idx = items.len();
            items.push(item);
            if nav == Some(idx) {
                let r = egui::Rect::from_min_max(
                    egui::pos2(ui.max_rect().left(), top),
                    egui::pos2(ui.max_rect().right(), ui.cursor().top() - 2.0),
                );
                ui.painter().rect_stroke(r, egui::CornerRadius::same(theme::RADIUS), egui::Stroke::new(1.5, theme::neon_hot()), egui::StrokeKind::Inside);
                if std::mem::take(&mut nav_moved) {
                    ui.scroll_to_rect(r, None);
                }
            }
        };
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (folder, rows) in &grouped {
                let collapsed = self.collapsed.contains(folder);
                let is_current = cur_folder.as_deref() == Some(folder.as_str());
                let top = ui.cursor().top();
                folder_header(ui, folder, rows.len(), collapsed, is_current, &mut toggle, &mut new_in, &mut dropped, &mut ask_delete, &mut empty_temp);
                mark(ui, extras::NavItem::Folder(folder.clone()), top, &mut nav_items);
                if collapsed {
                    continue;
                }
                if rows.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(26.0);
                        ui.label(theme::muted(if folder == store::TEMP { "scratch space" } else { "empty folder" }).small());
                    });
                }
                for (name, path) in rows {
                    let top = ui.cursor().top();
                    if prompt_row(ui, name, path, *path == cur, 28.0, Some(folder), &mut dropped, &mut ask_delete, &mut rollback, self.fx.tracked.contains(path)) {
                        open = Some(path.clone());
                    }
                    mark(ui, extras::NavItem::Prompt(path.clone()), top, &mut nav_items);
                }
            }
            if !grouped.is_empty() {
                ui.add_space(8.0);
                loose_header(ui, &mut dropped);
            }
            for (name, path) in &loose {
                let top = ui.cursor().top();
                if prompt_row(ui, name, path, *path == cur, 0.0, None, &mut dropped, &mut ask_delete, &mut rollback, self.fx.tracked.contains(path)) {
                    open = Some(path.clone());
                }
                mark(ui, extras::NavItem::Prompt(path.clone()), top, &mut nav_items);
            }
        });
        self.fx.nav_items = nav_items;

        if let Some(t) = ask_delete {
            self.pending_delete = Some(t);
        }
        if let Some((path, name)) = rollback {
            self.open_rollback(ctx, path, name);
        }
        if empty_temp {
            self.purge_temp_now(true);
        }
        if let Some(f) = toggle {
            if !self.collapsed.remove(&f) {
                self.collapsed.insert(f);
            }
        }
        if let Some(folder) = new_in {
            self.request_new(ctx, folder);
        }
        if let Some((path, folder)) = dropped {
            self.move_prompt(ctx, path, folder);
        }
        if let Some(path) = open {
            self.switch_to(ctx, path);
        }
        drag_ghost(ctx);
    }

    /// Drag-and-drop: move any prompt into a folder (or out of all folders).
    fn move_prompt(&mut self, ctx: &Context, path: PathBuf, folder: Option<String>) {
        if path == self.doc.path {
            self.assign_folder(ctx, folder);
            return;
        }
        if store::folder_of(&self.dir, &path) == folder {
            return;
        }
        match store::move_to_folder(&self.dir, &path, folder.as_deref()) {
            Ok(new_path) => {
                self.tracked_moved(&path, &new_path);
                self.refresh_entries();
                let place = folder.unwrap_or_else(|| "no folder".to_string());
                self.toast(format!("Moved to {place}"));
            }
            Err(e) => self.toast(format!("Could not move prompt: {e}")),
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if self.fx.nav.is_some() {
                ui.label(RichText::new("sidebar: j/k move · Enter opens a prompt or folds a folder · Esc leaves").small().color(theme::neon_hot()));
            } else if let Some(err) = &self.error {
                ui.label(RichText::new(err).small().color(theme::WARN));
            } else if let Some((msg, _)) = &self.toast {
                let color = if self.toast_bad { theme::DANGER } else { theme::GOOD };
                ui.label(RichText::new(msg).small().color(color));
            } else if self.doc.changed_at.is_some() {
                ui.label(theme::muted("Saving…").small());
            } else if self.is_template_doc() && self.doc.last_saved.is_some() {
                ui.label(RichText::new("Saved as a template").small().color(theme::GOOD));
            } else if self.doc.last_saved.is_some() {
                ui.label(RichText::new("Saved").small().color(theme::GOOD));
            } else {
                ui.label(theme::muted("New prompt — saved automatically once you start writing").small());
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(theme::muted(self.doc.path.display().to_string()).small());
                if self.is_tracked(&self.doc.path.clone()) {
                    if self.fx.dirty {
                        ui.label(RichText::new("● uncommitted — Ctrl+K").small().color(theme::WARN));
                    } else {
                        ui.label(RichText::new("✓ committed").small().color(theme::GOOD));
                    }
                }
            });
        });
    }

    /// The preview: a fixed-size overlay (80% of the window, centred from its very first frame).
    fn preview_window(&mut self, ctx: &Context) {
        use egui::{Area, Color32, CornerRadius, Margin, Order, Rect, Stroke};

        if !self.show_preview {
            self.preview_popup = None;
            return;
        }
        let mut spell = self.spell.take().unwrap_or_else(|| {
            spell::Checker::new(Some(self.dir.join(".personal_dictionary.txt")))
        });
        let defs: Vec<(usize, String, String)> = self
            .doc
            .prompt
            .definitions
            .iter()
            .flatten()
            .enumerate()
            .map(|(i, d)| (i, d.term.clone(), d.definition.clone()))
            .collect();
        spell.set_definitions(&defs);

        let md = self.doc.prompt.compile_markdown();
        let exported = self.doc.prompt.export_minified();
        let chars = exported.chars().count();
        let tokens = estimate_tokens(&exported);
        let misspelled = spell.count_misspelled(&self.doc.prompt);
        let unaddressed = self.doc.prompt.unaddressed_variables();
        let mut goto_var: Option<String> = None;

        let screen = ctx.content_rect();
        let size = screen.size() * 0.8; // 80% of the window
        let rect = Rect::from_center_size(screen.center(), size);

        let mut close = false;
        let mut fix_all = false;
        let mut clicked: Option<mdview::Action> = None;

        // Dimmed backdrop: clicking outside the card closes the preview.
        Area::new(Id::new("preview-backdrop"))
            .order(Order::Middle)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let r = ui.allocate_rect(screen, Sense::click());
                ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(150));
                close |= r.clicked();
            });

        Area::new(Id::new("preview-card"))
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(theme::bg_panel())
                    .corner_radius(CornerRadius::same(theme::RADIUS + 3))
                    .stroke(Stroke::new(1.5, theme::neon()))
                    .inner_margin(Margin::same(14))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 6],
                        blur: 28,
                        spread: 2,
                        color: theme::neon().gamma_multiply(60.0 / 255.0),
                    })
                    .show(ui, |ui| {
                        let inner = size - Vec2::splat(31.0);
                        ui.set_min_size(inner);
                        ui.set_max_size(inner);

                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Prompt preview").size(21.0).color(theme::neon_hot()));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if theme::ghost_button(ui, "✕").on_hover_text("Close (Esc)").clicked() {
                                    close = true;
                                }
                                let label = if misspelled == 0 {
                                    "No misspellings".to_owned()
                                } else {
                                    format!("Fix {misspelled} misspelled")
                                };
                                if theme::tool_button_enabled(ui, misspelled > 0, &label)
                                    .on_hover_text("Replace every misspelled word with the dictionary's best suggestion")
                                    .clicked()
                                {
                                    fix_all = true;
                                }
                            });
                        });
                        if !unaddressed.is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new("You have unaddressed variables")
                                        .strong()
                                        .size(18.0)
                                        .color(theme::DANGER),
                                );
                                ui.label(
                                    RichText::new(
                                        unaddressed.iter().map(|n| format!("{{{{{n}}}}}")).collect::<Vec<_>>().join("  "),
                                    )
                                    .small()
                                    .color(theme::DANGER),
                                );
                                ui.label(theme::muted("— export is blocked until they have a value").small());
                            });
                        }
                        // Cost of sending this prompt (input tokens only).
                        ui.horizontal_wrapped(|ui| {
                            let selected = self.settings.cost_model.clone();
                            let shown = if selected.is_empty() { "choose a model".to_owned() } else { selected.clone() };
                            egui::ComboBox::from_id_salt("cost-model").selected_text(shown).width(220.0).show_ui(ui, |ui| {
                                let mut last: Option<pricing::Provider> = None;
                                for pr in &self.prices {
                                    if last != Some(pr.provider) {
                                        ui.label(theme::muted(pr.provider.label()).small());
                                        last = Some(pr.provider);
                                    }
                                    if ui.selectable_label(selected == pr.model, &pr.model).clicked() {
                                        self.settings.cost_model = pr.model.clone();
                                        self.settings.save(&self.dir);
                                        self.cost_quote = None;
                                    }
                                }
                            });
                            let chosen = self.prices.iter().find(|pr| pr.model == self.settings.cost_model);
                            if theme::tool_button_enabled(ui, chosen.is_some(), "Calculate cost")
                                .on_hover_text("Count the input tokens of the exported prompt and price them")
                                .clicked()
                            {
                                if let Some(pr) = chosen {
                                    self.cost_quote = Some(pricing::quote(&exported, pr));
                                }
                            }
                            if let Some(q) = &self.cost_quote {
                                let approx = if q.exact { "" } else { "≈ " };
                                ui.label(
                                    RichText::new(format!("{} tokens · {approx}{} input cost", q.tokens, pricing::format_usd(q.usd)))
                                        .color(theme::GOOD),
                                );
                                if !q.exact {
                                    ui.label(theme::muted("estimate: this provider's tokenizer is not public").small());
                                }
                            }
                            ui.label(theme::muted(format!("({})", self.price_note)).small());
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label(theme::muted(format!("{chars} chars · ~{tokens} tokens when exported")).small());
                            ui.label(RichText::new("defined term / {{variable}}").small().color(mdview::DEFINED));
                            ui.label(RichText::new("misspelled").small().color(mdview::MISSPELLED));
                            ui.label(theme::muted("· click a coloured word · diagrams are exported as text").small());
                        });
                        ui.separator();

                        let dy = std::mem::take(&mut self.scroll_pending);
                        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            if dy != 0.0 {
                                ui.scroll_with_delta(Vec2::new(0.0, dy));
                            }
                            if md.trim().is_empty() {
                                ui.label(theme::muted("Nothing to preview yet — start writing."));
                                return;
                            }
                            // Text goes through the markdown view; ```mermaid blocks are drawn as real flowcharts.
                            for (i, seg) in preview::split(&md).iter().enumerate() {
                                ui.push_id(("preview-segment", i), |ui| match seg {
                                    preview::Segment::Markdown(text) => {
                                        if let Some(a) = mdview::show(ui, text, Some(&spell)) {
                                            clicked = Some(a);
                                        }
                                    }
                                    preview::Segment::Mermaid(src) => {
                                        self.diagrams.show(ui, src);
                                    }
                                });
                            }
                        });
                    });
            });

        // Popup for a clicked word.
        if let Some(mdview::Action::Variable { name }) = &clicked {
            goto_var = Some(name.clone());
            clicked = None;
        }
        let opened_now = clicked.is_some();
        if let Some(a) = clicked {
            self.preview_popup = Some(a);
        }
        let mut replace: Option<(String, String)> = None;
        let mut add_word: Option<String> = None;
        let mut goto: Option<usize> = None;
        let mut close_popup = false;
        if let Some(action) = self.preview_popup.clone() {
            let at = match &action {
                mdview::Action::Defined { at, .. } | mdview::Action::Misspelled { at, .. } => *at,
                mdview::Action::Variable { .. } => egui::Pos2::ZERO,
            };
            let area = Area::new(Id::new("preview-popup"))
                .order(Order::Tooltip)
                .fixed_pos(at + Vec2::new(0.0, 10.0))
                .constrain(true)
                .show(ctx, |ui| {
                    Frame::popup(ui.style()).stroke(Stroke::new(1.5, theme::neon())).show(ui, |ui| {
                        ui.set_max_width(380.0);
                        match &action {
                            mdview::Action::Misspelled { word, .. } => {
                                ui.label(RichText::new(word).strong().color(mdview::MISSPELLED));
                                let suggestions = spell.suggest(word);
                                if suggestions.is_empty() {
                                    ui.label(theme::muted("No suggestions"));
                                }
                                for s in &suggestions {
                                    if ui.button(format!("Replace with “{s}”")).clicked() {
                                        replace = Some((word.clone(), s.clone()));
                                    }
                                }
                                ui.separator();
                                if theme::ghost_button(ui, &format!("Add “{word}” to dictionary")).clicked() {
                                    add_word = Some(word.clone());
                                }
                            }
                            mdview::Action::Variable { .. } => {}
                            mdview::Action::Defined { index, .. } => {
                                if let Some((term, def)) = spell.definition_of(*index) {
                                    ui.label(RichText::new(term).strong().size(19.0).color(mdview::DEFINED));
                                    if def.trim().is_empty() {
                                        ui.label(theme::muted("No definition written yet."));
                                    } else {
                                        mdview::show(ui, def, None);
                                    }
                                }
                                if theme::tool_button(ui, "Edit definition").clicked() {
                                    goto = Some(*index);
                                }
                            }
                        }
                    });
                });
            let outside_click = ctx.input(|i| i.pointer.any_click()) && !area.response.contains_pointer();
            if outside_click && !opened_now {
                close_popup = true;
            }
        }
        if close_popup {
            self.preview_popup = None;
        }
        if let Some((from, to)) = replace {
            let n = spell.replace_word(&mut self.doc.prompt, &from, &to);
            self.toast(format!("Replaced {n} × “{from}”"));
            self.preview_popup = None;
        }
        if let Some(w) = add_word {
            spell.add_personal(&w);
            self.toast(format!("Added “{w}” to your dictionary"));
            self.preview_popup = None;
        }
        if fix_all {
            let n = spell.fix_all(&mut self.doc.prompt);
            self.toast(format!("Fixed {n} misspelled word{}", if n == 1 { "" } else { "s" }));
            self.preview_popup = None;
        }
        if let Some(name) = goto_var {
            close = true;
            self.view = View::Variables;
            self.st.var_open.insert(name.clone());
            self.st.focus_request = Some(var_value_id(&name));
        }
        if let Some(index) = goto {
            close = true;
            self.view = View::Editor;
            self.st.focus_request = Some(def_term_id(index));
        }
        self.spell = Some(spell);
        if close {
            self.show_preview = false;
            self.preview_popup = None;
        }
    }
}

// ---------------------------------------------------------------------
// sidebar rows
// ---------------------------------------------------------------------

fn prompt_row(
    ui: &mut Ui,
    name: &str,
    path: &std::path::Path,
    selected: bool,
    indent: f32,
    folder: Option<&str>,
    dropped: &mut Option<(PathBuf, Option<String>)>,
    ask_delete: &mut Option<DeleteTarget>,
    rollback: &mut Option<(PathBuf, String)>,
    tracked: bool,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click_and_drag());
    resp.dnd_set_drag_payload(path.to_path_buf());
    drop_target(ui, &resp, rect, folder, dropped);
    let body = egui::Rect::from_min_max(rect.min + Vec2::new(indent, 0.0), rect.max);
    if indent > 0.0 {
        // tree guide: prompts hang off their folder
        let x = rect.left() + 11.0;
        ui.painter().line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, theme::neon().gamma_multiply(0.35)),
        );
    }
    if selected || resp.hovered() {
        let fill = if selected { theme::bg_button_hot() } else { theme::bg_button() };
        ui.painter().rect_filled(body, egui::CornerRadius::same(theme::RADIUS), fill);
    }
    if selected {
        let bar = egui::Rect::from_min_size(
            body.left_top() + Vec2::new(0.0, 6.0),
            Vec2::new(3.0, body.height() - 12.0),
        );
        ui.painter().rect_filled(bar, egui::CornerRadius::same(2), theme::neon());
    }
    let color = if selected { egui::Color32::WHITE } else { theme::TEXT };
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(body.shrink2(Vec2::new(12.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.add(Label::new(RichText::new(name).color(color)).truncate().selectable(false));
        },
    );
    if resp.hovered() && !resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    resp.context_menu(|ui| {
        if tracked {
            if ui.button("Rollback…").clicked() {
                *rollback = Some((path.to_path_buf(), name.to_owned()));
                ui.close();
            }
        } else {
            ui.add_enabled(false, egui::Button::new("Rollback… (not tracked)"));
        }
        if ui.button(RichText::new("Delete prompt…").color(theme::WARN)).clicked() {
            *ask_delete = Some(DeleteTarget::Prompt {
                path: path.to_path_buf(),
                name: name.to_owned(),
            });
            ui.close();
        }
    });
    resp.clicked() && !selected
}

/// Highlight a drag-and-drop target and collect the dropped prompt.
fn drop_target(
    ui: &mut Ui,
    resp: &egui::Response,
    rect: egui::Rect,
    folder: Option<&str>,
    dropped: &mut Option<(PathBuf, Option<String>)>,
) {
    if resp.dnd_hover_payload::<PathBuf>().is_some() {
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(theme::RADIUS),
            egui::Stroke::new(1.5, theme::neon_hot()),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(p) = resp.dnd_release_payload::<PathBuf>() {
        *dropped = Some(((*p).clone(), folder.map(str::to_owned)));
    }
}

/// "No folder" drop zone (shown above the loose prompts).
fn loose_header(ui: &mut Ui, dropped: &mut Option<(PathBuf, Option<String>)>) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    drop_target(ui, &resp, rect, None, dropped);
    ui.painter().text(
        rect.left_center() + Vec2::new(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "NO FOLDER",
        egui::FontId::proportional(13.0),
        theme::TEXT_MUTED,
    );
}

/// The prompt name following the pointer while dragging.
fn drag_ghost(ctx: &Context) {
    let Some(path) = egui::DragAndDrop::payload::<PathBuf>(ctx) else { return };
    let Some(pos) = ctx.pointer_latest_pos() else { return };
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    egui::Area::new(Id::new("dnd-ghost"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + Vec2::new(14.0, 10.0))
        .interactable(false)
        .show(ctx, |ui| {
            Frame::popup(ui.style())
                .stroke(egui::Stroke::new(1.5, theme::neon()))
                .show(ui, |ui| {
                    ui.label(RichText::new(name).color(theme::TEXT));
                });
        });
}

/// A folder row: arrow, name, prompt count and a "+" to start a prompt inside it.
fn folder_header(
    ui: &mut Ui,
    name: &str,
    count: usize,
    collapsed: bool,
    current: bool,
    toggle: &mut Option<String>,
    new_in: &mut Option<Option<String>>,
    dropped: &mut Option<(PathBuf, Option<String>)>,
    ask_delete: &mut Option<DeleteTarget>,
    empty_temp: &mut bool,
) {
    let is_temp = name == store::TEMP;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
    drop_target(ui, &resp, rect, Some(name), dropped);
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(theme::RADIUS), theme::bg_button());
    }
    let tint = if current { theme::neon_hot() } else if is_temp { theme::WARN } else { theme::TEXT_MUTED };

    // Disclosure triangle, drawn (no dependence on font glyphs).
    let c = egui::pos2(rect.left() + 11.0, rect.center().y);
    let pts = if collapsed {
        vec![c + Vec2::new(-2.5, -4.5), c + Vec2::new(-2.5, 4.5), c + Vec2::new(3.5, 0.0)]
    } else {
        vec![c + Vec2::new(-4.5, -2.5), c + Vec2::new(4.5, -2.5), c + Vec2::new(0.0, 3.5)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, tint, egui::Stroke::NONE));

    let label_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 24.0, rect.top()),
        egui::pos2(rect.right() - 50.0, rect.bottom()),
    );
    let color = if current { theme::neon_hot() } else if is_temp { theme::WARN } else { theme::TEXT };
    // The handwriting font has no bold weight, so the folder name is drawn twice, a hair apart.
    for dx in [0.0, 0.9] {
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(label_rect.translate(Vec2::new(dx, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                let mut text = RichText::new(name).color(color);
                if is_temp {
                    text = text.italics();
                }
                ui.add(Label::new(text).truncate().selectable(false));
            },
        );
    }
    ui.painter().text(
        egui::pos2(rect.right() - 30.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        count.to_string(),
        egui::FontId::proportional(13.0),
        theme::TEXT_MUTED,
    );

    let resp = if is_temp {
        resp.on_hover_text("Built-in scratch folder: prompts not touched for a while are deleted automatically (see Settings)")
    } else {
        resp
    };
    resp.context_menu(|ui| {
        if is_temp {
            if ui.button("Empty temp now").clicked() {
                *empty_temp = true;
                ui.close();
            }
        } else if ui.button(RichText::new("Delete folder…").color(theme::WARN)).clicked() {
            *ask_delete = Some(DeleteTarget::Folder(name.to_string()));
            ui.close();
        }
    });
    let plus_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 13.0, rect.center().y), Vec2::splat(22.0));
    let plus = ui.put(plus_rect, |ui: &mut Ui| theme::ghost_button(ui, "+"));
    if plus.on_hover_text("New prompt in this folder").clicked() {
        *new_in = Some(Some(name.to_string()));
    } else if resp.clicked() {
        *toggle = Some(name.to_string());
    }
}

// ---------------------------------------------------------------------
// editor view
// ---------------------------------------------------------------------

/// Returns true when the user confirmed removal. Non-empty sections need a second click.
fn confirm_remove(ctx: &Context, st: &mut UiState, key: Remove, non_empty: bool) -> bool {
    if !non_empty || st.confirm_pending(key) {
        st.confirm = None;
        return true;
    }
    st.confirm = Some((key, Instant::now()));
    ctx.request_repaint_after(CONFIRM_WINDOW + Duration::from_millis(50));
    false
}

fn section_header(
    ui: &mut Ui,
    st: &mut UiState,
    title: &str,
    key: Remove,
    non_empty: bool,
    copy: Option<&str>,
) -> bool {
    let mut remove = false;
    ui.horizontal(|ui| {
        theme::small_caps_label(ui, title);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::ghost_button(ui, "×").on_hover_text("Remove").clicked() {
                remove = confirm_remove(ui.ctx(), st, key, non_empty);
            }
            if st.confirm_pending(key) {
                ui.label(RichText::new("click × again to remove").small().color(theme::WARN));
            }
            if let Some(text) = copy {
                let flag = ui.id().with("copied-at");
                if theme::ghost_button(ui, "Copy").on_hover_text("Copy this question to the clipboard").clicked() {
                    ui.ctx().copy_text(text.to_owned());
                    ui.ctx().data_mut(|d| d.insert_temp(flag, Instant::now()));
                }
                let when: Option<Instant> = ui.ctx().data(|d| d.get_temp(flag));
                if let Some(t) = when.filter(|t| t.elapsed() < Duration::from_millis(1500)) {
                    let _ = t;
                    ui.label(RichText::new("copied").small().color(theme::GOOD));
                    ui.ctx().request_repaint_after(Duration::from_millis(300));
                }
            }
        });
    });
    remove
}

fn editor_column(ui: &mut Ui, doc: &mut OpenDoc, st: &mut UiState) {
    let structured =
        doc.prompt.context.is_some() || doc.prompt.definitions.is_some() || doc.prompt.rules.is_some();

    if doc.prompt.context.is_some() {
        let mut remove = false;
        theme::card(ui, |ui| {
            let ctx_text = doc.prompt.context.as_mut().unwrap();
            let non_empty = !ctx_text.trim().is_empty();
            remove = section_header(ui, st, "Context", Remove::Context, non_empty, None);
            let resp = ui.add(
                full_width_edit(ctx_text, context_id())
                    .desired_rows(4)
                    .hint_text("Set the conceptual space: who is the model, what world does it operate in?"),
            );
            apply_focus(st, &resp);
            line_mover(ui, &resp, ctx_text);
        });
        if remove {
            doc.prompt.context = None;
        }
        ui.add_space(6.0);
    }

    if doc.prompt.definitions.is_some() {
        let mut remove = false;
        theme::card(ui, |ui| {
            let defs = doc.prompt.definitions.as_mut().unwrap();
            remove = definitions_ui(ui, defs, st);
        });
        if remove {
            doc.prompt.definitions = None;
        }
        ui.add_space(6.0);
    }

    if structured {
        theme::small_caps_label(ui, "Main");
    }
    note_card(ui, doc, st, structured);

    if doc.prompt.rules.is_some() {
        ui.add_space(6.0);
        let mut remove = false;
        theme::card(ui, |ui| {
            let rules = doc.prompt.rules.as_mut().unwrap();
            remove = rules_ui(ui, rules, st);
        });
        if remove {
            doc.prompt.rules = None;
        }
    }
    ui.add_space(24.0);
}

fn note_card(ui: &mut Ui, doc: &mut OpenDoc, st: &mut UiState, structured: bool) {
    let mut remove_flow: Option<usize> = None;
    theme::card(ui, |ui| {
        let n = doc.prompt.body.len();
        for i in 0..n {
            match &mut doc.prompt.body[i] {
                BodyBlock::Text(t) => {
                    let rows = if n == 1 {
                        if structured { 8 } else { 22 }
                    } else {
                        3
                    };
                    let id = Id::new(("body-text", i));
                    let hint = if i == 0 { "Start writing your prompt…" } else { "" };
                    let resp = ui.add(full_width_edit(t, id).desired_rows(rows).hint_text(hint));
                    if resp.has_focus() {
                        doc.last_body_focus = i;
                    }
                    apply_focus(st, &resp);
                    line_mover(ui, &resp, t);
                }
                BodyBlock::Code(c) => {
                    if code_ui(ui, c, st) {
                        remove_flow = Some(i);
                    }
                    let f = ui.memory(|m| m.focused());
                    if f == Some(code_src_id(&c.id)) || f == Some(code_title_id(&c.id)) {
                        doc.last_body_focus = i;
                    }
                }
                BodyBlock::Flow(f) => {
                    if flow_ui(ui, f, st) {
                        remove_flow = Some(i);
                    }
                    if ui.memory(|m| m.focused()) == Some(flow_src_id(&f.id))
                        || ui.memory(|m| m.focused()) == Some(flow_title_id(&f.id))
                    {
                        doc.last_body_focus = i;
                    }
                }
            }
        }
    });

    if let Some(i) = remove_flow {
        let body = &mut doc.prompt.body;
        body.remove(i);
        // Two text blocks that were separated by the flow become one again.
        if i > 0 && i < body.len() {
            if let (BodyBlock::Text(_), BodyBlock::Text(_)) = (&body[i - 1], &body[i]) {
                if let BodyBlock::Text(next) = body.remove(i) {
                    if let BodyBlock::Text(prev) = &mut body[i - 1] {
                        if !prev.is_empty() && !next.is_empty() {
                            prev.push_str("\n\n");
                        }
                        prev.push_str(&next);
                    }
                }
            }
        }
        if body.is_empty() {
            body.push(BodyBlock::Text(String::new()));
        }
        doc.last_body_focus = doc.last_body_focus.min(body.len() - 1);
    }
}

/// A pasted code block: collapses like a flow and is exported inside a ``` fence.
/// Returns true if the block should be removed.
fn code_ui(ui: &mut Ui, c: &mut crate::model::CodeBlock, st: &mut UiState) -> bool {
    let mut remove = false;
    ui.add_space(2.0);
    if c.collapsed {
        theme::chip(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("CODE").small().strong().color(theme::neon_hot()));
                let first = c.code.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
                let (label, dim) = if !c.title.trim().is_empty() {
                    (c.title.trim().to_owned(), false)
                } else if !first.is_empty() {
                    (first.to_owned(), true)
                } else {
                    ("empty code block".to_owned(), true)
                };
                let text = if dim {
                    RichText::new(label).monospace().color(theme::TEXT_MUTED)
                } else {
                    RichText::new(label).strong()
                };
                let title = ui.add(Label::new(text).truncate().sense(Sense::click()));
                if title.on_hover_text("Click to edit").clicked() {
                    c.collapsed = false;
                    st.focus_request = Some(code_src_id(&c.id));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::ghost_button(ui, "×").on_hover_text("Remove code block").clicked() {
                        remove = true;
                    }
                    if theme::ghost_button(ui, "Edit").clicked() {
                        c.collapsed = false;
                        st.focus_request = Some(code_src_id(&c.id));
                    }
                    ui.label(theme::muted({ let n = c.code.lines().count(); format!("{n} line{}", if n == 1 { "" } else { "s" }) }).small());
                });
            });
        });
    } else {
        theme::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::small_caps_label(ui, "Code");
                let title = ui.add(
                    { let al = textdir::align_for(&c.title); TextEdit::singleline(&mut c.title).horizontal_align(al) }
                        .id(code_title_id(&c.id))
                        .desired_width(240.0)
                        .hint_text("title, e.g. main.rs (optional)"),
                );
                apply_focus(st, &title);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::ghost_button(ui, "×").on_hover_text("Remove code block").clicked() {
                        remove = true;
                    }
                    ui.label(theme::muted("exported in a ``` fence · Ctrl+Enter to collapse").small());
                });
            });
            let src = ui.add(
                TextEdit::multiline(&mut c.code)
                    .id(code_src_id(&c.id))
                    .code_editor()
                    .frame(Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .desired_rows(8)
                    .hint_text("paste your code here"),
            );
            apply_focus(st, &src);
            line_mover(ui, &src, &mut c.code);
        });
    }
    ui.add_space(2.0);
    remove
}

/// Returns true if the flow should be removed.
fn flow_ui(ui: &mut Ui, f: &mut crate::model::Flow, st: &mut UiState) -> bool {
    let mut remove = false;
    ui.add_space(2.0);
    if f.collapsed {
        theme::chip(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("FLOW").small().strong().color(theme::neon_hot()));
                let lines = f.mermaid.lines().count();
                let title = ui.add(
                    Label::new(RichText::new(&f.title).strong())
                        .truncate()
                        .sense(Sense::click()),
                );
                if title.on_hover_text("Click to edit").clicked() {
                    f.collapsed = false;
                    st.focus_request = Some(flow_src_id(&f.id));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::ghost_button(ui, "×").on_hover_text("Remove flow").clicked() {
                        remove = true;
                    }
                    if theme::ghost_button(ui, "Edit").clicked() {
                        f.collapsed = false;
                        st.focus_request = Some(flow_src_id(&f.id));
                    }
                    ui.label(theme::muted(format!("{lines} line{}", if lines == 1 { "" } else { "s" })).small());
                });
            });
        });
    } else {
        theme::card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::small_caps_label(ui, "Flow");
                let title = ui.add(
                    { let al = textdir::align_for(&f.title); TextEdit::singleline(&mut f.title).horizontal_align(al) }
                        .id(flow_title_id(&f.id))
                        .desired_width(240.0)
                        .hint_text("flow title"),
                );
                apply_focus(st, &title);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::ghost_button(ui, "×").on_hover_text("Remove flow").clicked() {
                        remove = true;
                    }
                    ui.label(theme::muted("mermaid · Ctrl+Enter to collapse").small());
                });
            });
            let src = ui.add(
                TextEdit::multiline(&mut f.mermaid)
                    .id(flow_src_id(&f.id))
                    .code_editor()
                    .frame(Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .desired_rows(6)
                    .hint_text("graph TD\n    A[Start] --> B{Decision}"),
            );
            apply_focus(st, &src);
            line_mover(ui, &src, &mut f.mermaid);
        });
    }
    ui.add_space(2.0);
    remove
}

fn definitions_ui(ui: &mut Ui, defs: &mut Vec<Definition>, st: &mut UiState) -> bool {
    let non_empty = defs
        .iter()
        .any(|d| !d.term.trim().is_empty() || !d.definition.trim().is_empty());
    let remove = section_header(ui, st, "Definitions", Remove::Definitions, non_empty, None);

    let mut del: Option<usize> = None;
    for (i, d) in defs.iter_mut().enumerate() {
        ui.horizontal_top(|ui| {
            let term = ui.add(
                { let al = textdir::align_for(&d.term); TextEdit::singleline(&mut d.term).horizontal_align(al) }
                    .id(def_term_id(i))
                    .desired_width(160.0)
                    .hint_text("term")
                    .font(egui::TextStyle::Button),
            );
            apply_focus(st, &term);
            let w = (ui.available_width() - 34.0).max(80.0);
            let dr = ui.add(
                TextEdit::multiline(&mut d.definition)
                    .id(Id::new(("def-text", i)))
                    .desired_width(w)
                    .desired_rows(1)
                    .hint_text("definition — grows as you type"),
            );
            line_mover(ui, &dr, &mut d.definition);
            if theme::ghost_button(ui, "×").on_hover_text("Remove term").clicked() {
                del = Some(i);
            }
        });
    }
    if let Some(i) = del {
        defs.remove(i);
    }
    if theme::ghost_button(ui, "+ Add term").clicked() {
        defs.push(Definition::default());
        st.focus_request = Some(def_term_id(defs.len() - 1));
    }
    remove
}

fn rules_ui(ui: &mut Ui, rules: &mut Vec<Rule>, st: &mut UiState) -> bool {
    let non_empty = rules
        .iter()
        .any(|r| !r.text.trim().is_empty() || !r.examples.is_empty());
    let remove = section_header(ui, st, "Rules", Remove::Rules, non_empty, None);

    // A pending example that points at a vanished rule is dropped.
    if st.pending_example.as_ref().is_some_and(|p| p.rule >= rules.len()) {
        st.pending_example = None;
    }

    let mut del_rule: Option<usize> = None;
    let mut del_example: Option<(usize, usize)> = None;
    let mut start_example: Option<usize> = None;
    let mut cancel_example = false;

    for (i, rule) in rules.iter_mut().enumerate() {
        ui.horizontal_top(|ui| {
            ui.label(RichText::new(format!("{}.", i + 1)).strong().color(theme::neon_hot()));
            let w = (ui.available_width() - 34.0).max(80.0);
            let resp = ui.add(
                TextEdit::multiline(&mut rule.text)
                    .id(rule_id(i))
                    .desired_width(w)
                    .desired_rows(1)
                    .hint_text("State a rule…"),
            );
            apply_focus(st, &resp);
            line_mover(ui, &resp, &mut rule.text);
            if theme::ghost_button(ui, "×").on_hover_text("Remove rule").clicked() {
                del_rule = Some(i);
            }
        });

        for (ei, ex) in rule.examples.iter_mut().enumerate() {
            ui.horizontal_top(|ui| {
                ui.add_space(24.0);
                ui.label(theme::muted("e.g."));
                let w = (ui.available_width() - 34.0).max(80.0);
                let er = ui.add(
                    TextEdit::multiline(ex)
                        .id(Id::new(("rule-example", i, ei)))
                        .desired_width(w)
                        .desired_rows(1),
                );
                line_mover(ui, &er, ex);
                if theme::ghost_button(ui, "×").on_hover_text("Remove example").clicked() {
                    del_example = Some((i, ei));
                }
            });
        }

        ui.horizontal_top(|ui| {
            ui.add_space(24.0);
            match st.pending_example.as_mut().filter(|p| p.rule == i) {
                Some(pe) => {
                    let w = (ui.available_width() - 34.0).max(80.0);
                    let resp = ui.add(
                        TextEdit::multiline(&mut pe.text)
                            .id(pending_example_id())
                            .desired_width(w)
                            .desired_rows(1)
                            .hint_text("new example — Ctrl+Enter to add"),
                    );
                    line_mover(ui, &resp, &mut pe.text);
                    if pe.needs_focus {
                        resp.request_focus();
                        pe.needs_focus = false;
                    }
                    if theme::ghost_button(ui, "×").on_hover_text("Cancel").clicked() {
                        cancel_example = true;
                    }
                }
                None => {
                    if theme::ghost_button(ui, "+ example").clicked() {
                        start_example = Some(i);
                    }
                }
            }
        });
        ui.add_space(4.0);
    }

    if cancel_example {
        st.pending_example = None;
    }
    if let Some(i) = start_example {
        st.pending_example = Some(PendingExample {
            rule: i,
            text: String::new(),
            needs_focus: true,
        });
    }
    if let Some((r, e)) = del_example {
        rules[r].examples.remove(e);
    }
    if let Some(i) = del_rule {
        rules.remove(i);
        st.pending_example = None;
    }
    if theme::ghost_button(ui, "+ Add rule").clicked() {
        rules.push(Rule::default());
        st.focus_request = Some(rule_id(rules.len() - 1));
    }
    remove
}

// ---------------------------------------------------------------------
// benchmarks view
// ---------------------------------------------------------------------

/// The Variables tab: one card per `{{name}}` used in the prompt.
/// Filled cards collapse into a one-line chip (Ctrl+Enter), so the tab reads as a list of names.
fn variables_column(ui: &mut Ui, doc: &mut OpenDoc, st: &mut UiState) {
    ui.label(theme::muted(
        "Write {{name}} anywhere in the prompt (Alt+V inserts the braces). Each name gets a card here; \
         on export the tag is replaced by its value, and export is blocked while a value is missing.",
    ));
    ui.add_space(8.0);

    let names = doc.prompt.used_variables();
    if names.is_empty() {
        theme::card(ui, |ui| {
            ui.label(theme::muted("No variables yet — type {{codechunk_1}} in the Context, Main, Rules …"));
        });
    }
    for name in &names {
        let filled = doc.prompt.variable_value(name).is_some_and(|v| !v.trim().is_empty());
        if !filled {
            st.var_open.insert(name.clone()); // an empty card always stays open
        }
        let tag = format!("{{{{{name}}}}}");
        if !st.var_open.contains(name) {
            let value = doc.prompt.variable_value(name).unwrap_or("").to_owned();
            theme::chip(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    let tagr = ui.add(
                        Label::new(RichText::new(&tag).strong().color(mdview::DEFINED)).sense(Sense::click()),
                    );
                    let first = value.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
                    ui.add(Label::new(theme::muted(first).small()).truncate().selectable(false));
                    let mut edit = tagr.on_hover_text("Click to edit").clicked();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::ghost_button(ui, "Edit").clicked() {
                            edit = true;
                        }
                        ui.label(theme::muted({ let n = value.lines().count(); format!("{n} line{}", if n == 1 { "" } else { "s" }) }).small());
                    });
                    if edit {
                        st.var_open.insert(name.clone());
                        st.focus_request = Some(var_value_id(name));
                    }
                });
            });
        } else {
            theme::card(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&tag).strong().color(mdview::DEFINED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if filled {
                            ui.label(theme::muted("Ctrl+Enter to collapse").small());
                        } else {
                            ui.label(RichText::new("needs a value").small().strong().color(theme::DANGER));
                        }
                    });
                });
                let value = doc.prompt.variable_value_mut(name);
                let resp = ui.add(
                    full_width_edit(value, var_value_id(name))
                        .desired_rows(4)
                        .hint_text("value — pasted text, code, a directory listing …"),
                );
                apply_focus(st, &resp);
                line_mover(ui, &resp, value);
            });
        }
        ui.add_space(6.0);
    }

    // Values whose tag no longer appears anywhere (kept in the file until forgotten).
    let unused: Vec<String> = doc
        .prompt
        .variables
        .iter()
        .filter(|v| !v.value.trim().is_empty() && !names.contains(&v.name))
        .map(|v| v.name.clone())
        .collect();
    if !unused.is_empty() {
        ui.add_space(8.0);
        ui.label(theme::muted("Not used in the prompt any more").small());
        for n in unused {
            ui.horizontal(|ui| {
                ui.label(theme::muted(format!("{{{{{n}}}}}")));
                if theme::ghost_button(ui, "× forget").on_hover_text("Delete this stored value").clicked() {
                    doc.prompt.variables.retain(|v| v.name != n);
                }
            });
        }
    }
    ui.add_space(24.0);
}

fn benchmarks_column(ui: &mut Ui, doc: &mut OpenDoc, st: &mut UiState) {
    ui.label(theme::muted(
        "Questions to test this prompt with, and the answers different models gave. \
         Stored with the prompt, never part of the preview or export.",
    ));
    ui.add_space(8.0);

    let models = doc.prompt.known_models();
    let benches = &mut doc.prompt.benchmarks;

    if benches.is_empty() {
        theme::card(ui, |ui| {
            ui.label(theme::muted("No questions yet — use “+ Question” in the toolbar."));
        });
    }

    let mut remove_q: Option<usize> = None;
    for (qi, b) in benches.iter_mut().enumerate() {
        theme::card(ui, |ui| {
            let non_empty =
                !b.question.trim().is_empty() || b.answers.iter().any(|a| !a.text.trim().is_empty());
            if section_header(ui, st, &format!("Q{}", qi + 1), Remove::Benchmark(qi), non_empty, Some(&b.question.clone())) {
                remove_q = Some(qi);
            }
            let q = ui.add(
                full_width_edit(&mut b.question, bench_q_id(qi))
                    .desired_rows(2)
                    .hint_text("Benchmark question…"),
            );
            apply_focus(st, &q);
            line_mover(ui, &q, &mut b.question);

            // Optional expected answer, for reviewing the model answers later.
            let mut drop_expected = false;
            match b.expected.as_mut() {
                Some(exp) => {
                    ui.separator();
                    ui.horizontal_top(|ui| {
                        ui.label(RichText::new("Expected").color(theme::GOOD));
                        let w = (ui.available_width() - 34.0).max(80.0);
                        let r = ui.add(
                            TextEdit::multiline(exp)
                                .id(bench_expected_id(qi))
                                .desired_width(w)
                                .desired_rows(2)
                                .hint_text("what a good answer looks like (optional)"),
                        );
                        apply_focus(st, &r);
                        line_mover(ui, &r, exp);
                        if theme::ghost_button(ui, "×").on_hover_text("Remove expected answer").clicked() {
                            drop_expected = true;
                        }
                    });
                }
                None => {
                    if theme::ghost_button(ui, "+ Expected answer").on_hover_text("Optional: note what a good answer should contain").clicked() {
                        b.expected = Some(String::new());
                        st.focus_request = Some(bench_expected_id(qi));
                    }
                }
            }
            if drop_expected {
                b.expected = None;
            }

            let mut del_answer: Option<usize> = None;
            for (ai, a) in b.answers.iter_mut().enumerate() {
                ui.separator();
                ui.horizontal_top(|ui| {
                    let m = ui.add(
                        { let al = textdir::align_for(&a.model); TextEdit::singleline(&mut a.model).horizontal_align(al) }
                            .id(bench_model_id(qi, ai))
                            .desired_width(150.0)
                            .hint_text("model")
                            .font(egui::TextStyle::Button),
                    );
                    apply_focus(st, &m);
                    let w = (ui.available_width() - 34.0).max(80.0);
                    let t = ui.add(
                        TextEdit::multiline(&mut a.text)
                            .id(bench_answer_id(qi, ai))
                            .desired_width(w)
                            .desired_rows(2)
                            .hint_text("answer"),
                    );
                    apply_focus(st, &t);
                    line_mover(ui, &t, &mut a.text);
                    if theme::ghost_button(ui, "×").on_hover_text("Remove answer").clicked() {
                        del_answer = Some(ai);
                    }
                });
            }
            if let Some(ai) = del_answer {
                b.answers.remove(ai);
            }

            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if theme::ghost_button(ui, "+ Answer").clicked() {
                    b.answers.push(Answer::default());
                    st.focus_request = Some(bench_model_id(qi, b.answers.len() - 1));
                }
                // One click to add an answer slot for a model already used elsewhere.
                let unused: Vec<&String> = models
                    .iter()
                    .filter(|m| !b.answers.iter().any(|a| a.model.trim() == m.as_str()))
                    .collect();
                for m in unused {
                    let chip = egui::Button::new(RichText::new(format!("+ {m}")).small())
                        .corner_radius(egui::CornerRadius::same(theme::RADIUS));
                    if ui.add(chip).on_hover_text("Add an answer from this model").clicked() {
                        b.answers.push(Answer {
                            model: m.clone(),
                            text: String::new(),
                        });
                        st.focus_request = Some(bench_answer_id(qi, b.answers.len() - 1));
                    }
                }
            });
        });
        ui.add_space(6.0);
    }
    if let Some(i) = remove_q {
        benches.remove(i);
    }
    ui.add_space(24.0);
}

// ---------------------------------------------------------------------
// tabs attached to the working canvas
// ---------------------------------------------------------------------

const TAB_H: f32 = 34.0;

/// Points of a tab outline: up the left side, round the top corners, down the right side (no bottom edge).
fn tab_outline(rect: egui::Rect, r: f32) -> Vec<egui::Pos2> {
    use std::f32::consts::{FRAC_PI_2, PI};
    let mut pts = vec![egui::pos2(rect.left(), rect.bottom())];
    let arc = |pts: &mut Vec<egui::Pos2>, c: egui::Pos2, from: f32| {
        for i in 0..=8 {
            let a = from + FRAC_PI_2 * i as f32 / 8.0;
            pts.push(c + Vec2::new(a.cos(), a.sin()) * r);
        }
    };
    arc(&mut pts, egui::pos2(rect.left() + r, rect.top() + r), PI);
    arc(&mut pts, egui::pos2(rect.right() - r, rect.top() + r), PI + FRAC_PI_2);
    pts.push(egui::pos2(rect.right(), rect.bottom()));
    pts
}

/// Draws the canvas (outlined surface) with the tabs sitting on its top-left edge; the active tab
/// has no bottom border and the canvas colour, so it reads as one piece with the canvas.
/// Returns the canvas rectangle.
fn tabs_and_canvas(ui: &mut Ui, view: &mut View, pending: usize) -> egui::Rect {
    use egui::{CornerRadius, Stroke, StrokeKind};
    let (strip, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TAB_H), Sense::hover());
    let canvas = egui::Rect::from_min_max(egui::pos2(strip.left(), strip.bottom()), ui.max_rect().right_bottom());
    let line_color = theme::neon().gamma_multiply(0.75);
    let line = Stroke::new(1.5, line_color);
    let painter = ui.painter().clone();

    let radius = CornerRadius::same(theme::RADIUS + 1);
    painter.rect_filled(canvas, radius, theme::bg_root());
    painter.rect_stroke(canvas, radius, line, StrokeKind::Inside);

    let mut x = canvas.left() + 16.0;
    let var_label = if pending > 0 { format!("Variables ({pending})") } else { "Variables".to_owned() };
    for (v, label) in [
        (View::Editor, "Editor".to_owned()),
        (View::Variables, var_label),
        (View::Benchmarks, "Benchmarks".to_owned()),
    ] {
        let label = label.as_str();
        let active = *view == v;
        let color = if active {
            egui::Color32::WHITE
        } else if v == View::Variables && pending > 0 {
            theme::DANGER
        } else {
            theme::TEXT_MUTED
        };
        let galley = painter.layout_no_wrap(label.to_owned(), egui::FontId::proportional(17.0), color);
        let w = galley.size().x + 38.0;
        let top = strip.top() + if active { 0.0 } else { 5.0 };
        let rect = egui::Rect::from_min_max(egui::pos2(x, top), egui::pos2(x + w, strip.bottom()));
        let resp = ui.interact(rect, ui.id().with(("tab", label)), Sense::click());
        if resp.clicked() {
            *view = v;
        }
        let corners = CornerRadius { nw: 9, ne: 9, sw: 0, se: 0 };
        if active {
            // Cover the canvas border under the tab so the two connect.
            let cover = egui::Rect::from_min_max(rect.min, rect.max + Vec2::new(0.0, 2.0));
            painter.rect_filled(cover, corners, theme::bg_root());
            painter.add(egui::Shape::line(tab_outline(rect, 9.0), line));
        } else {
            let fill = if resp.hovered() { theme::bg_button_hot() } else { theme::bg_button() };
            painter.rect_filled(rect, corners, fill);
            painter.add(egui::Shape::line(
                tab_outline(rect, 9.0),
                Stroke::new(1.0, theme::neon().gamma_multiply(0.35)),
            ));
        }
        painter.galley(
            egui::pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0 + if active { 1.0 } else { 0.0 }),
            galley,
            color,
        );
        match v {
            View::Benchmarks => {
                resp.on_hover_text("Test questions and per-model answers. Never exported.");
            }
            View::Variables => {
                resp.on_hover_text("Values for {{placeholders}} used anywhere in the prompt. They replace the tags on export.");
            }
            View::Editor => {}
        }
        x += w + 6.0;
    }
    canvas
}
