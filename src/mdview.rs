//! A small markdown renderer for the preview.
//!
//! Compared to a generic viewer it knows about the prompt: misspelled words and terms from the
//! Definitions section are coloured and clickable, and right-to-left paragraphs get their bullets
//! on the right. Text goes through egui's (bidi-aware) text layout, so mixed-direction text reads correctly.

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{
    self, text::LayoutJob, Color32, FontFamily, FontId, Frame, Galley, Margin, Pos2, RichText, Sense, Stroke,
    TextFormat, TextStyle, Ui, Vec2,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::spell::{Checker, Verdict};
use crate::{textdir, theme};

/// Cyan-blue for words that are defined in the Definitions section.
pub const DEFINED: Color32 = Color32::from_rgb(0x4d, 0xd6, 0xff);
/// Red for misspelled words.
pub const MISSPELLED: Color32 = Color32::from_rgb(0xff, 0x6b, 0x7a);

/// Something the user clicked in the rendered text.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A defined term (index into the prompt's definitions).
    Defined { index: usize, at: Pos2 },
    Misspelled { word: String, at: Pos2 },
    /// A `{{variable}}` tag: jump to its value.
    Variable { name: String },
}

struct Cx<'a> {
    checker: Option<&'a Checker>,
    action: Option<Action>,
    list_depth: usize,
}

/// Render markdown. With a `checker`, misspelled / defined words are highlighted and clickable.
pub fn show(ui: &mut Ui, md: &str, checker: Option<&Checker>) -> Option<Action> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    // Merge neighbouring text events so `{{some_name}}` is never split at the underscore.
    let events: Vec<Event> = pulldown_cmark::TextMergeStream::new(Parser::new_ext(md, opts)).collect();
    let mut cx = Cx {
        checker,
        action: None,
        list_depth: 0,
    };
    blocks(ui, &events, &mut cx);
    cx.action
}

// ---------------------------------------------------------------------
// block structure
// ---------------------------------------------------------------------

fn is_inline(e: &Event) -> bool {
    match e {
        Event::Text(_)
        | Event::Code(_)
        | Event::SoftBreak
        | Event::HardBreak
        | Event::InlineHtml(_)
        | Event::TaskListMarker(_)
        | Event::FootnoteReference(_)
        | Event::InlineMath(_) => true,
        Event::Start(t) => matches!(
            t,
            Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. }
        ),
        Event::End(t) => matches!(
            t,
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image
        ),
        _ => false,
    }
}

/// Index of the `End` matching the `Start` at `i`.
fn matching_end(ev: &[Event], i: usize) -> usize {
    let mut depth = 0usize;
    for (j, e) in ev.iter().enumerate().skip(i) {
        match e {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
    }
    ev.len().saturating_sub(1)
}

fn gap(ui: &mut Ui, cx: &Cx) {
    ui.add_space(if cx.list_depth > 0 { 2.0 } else { 8.0 });
}

fn blocks(ui: &mut Ui, ev: &[Event], cx: &mut Cx) {
    let mut i = 0;
    while i < ev.len() {
        if is_inline(&ev[i]) {
            let mut j = i;
            while j < ev.len() && is_inline(&ev[j]) {
                j += 1;
            }
            let spans = spans(&ev[i..j]);
            inline_label(ui, &spans, Kind::Body, cx);
            gap(ui, cx);
            i = j;
            continue;
        }
        match &ev[i] {
            Event::Start(tag) => {
                let end = matching_end(ev, i);
                let inner = &ev[i + 1..end];
                match tag {
                    Tag::Paragraph => {
                        inline_label(ui, &spans(inner), Kind::Body, cx);
                        gap(ui, cx);
                    }
                    Tag::Heading { level, .. } => {
                        ui.add_space(6.0);
                        inline_label(ui, &spans(inner), Kind::Heading(*level), cx);
                        ui.add_space(4.0);
                    }
                    Tag::List(start) => {
                        list(ui, inner, *start, cx);
                        gap(ui, cx);
                    }
                    Tag::BlockQuote(_) => quote(ui, inner, cx),
                    Tag::CodeBlock(kind) => {
                        let code: String = inner
                            .iter()
                            .filter_map(|e| if let Event::Text(t) = e { Some(t.as_ref()) } else { None })
                            .collect();
                        let lang = match kind {
                            CodeBlockKind::Fenced(l) => l.to_string(),
                            CodeBlockKind::Indented => String::new(),
                        };
                        code_block(ui, &code, &lang);
                        gap(ui, cx);
                    }
                    Tag::HtmlBlock => {
                        let html: String = inner
                            .iter()
                            .filter_map(|e| if let Event::Html(t) = e { Some(t.as_ref()) } else { None })
                            .collect();
                        ui.label(RichText::new(html.trim_end()).monospace().color(theme::TEXT_MUTED));
                        gap(ui, cx);
                    }
                    Tag::Table(_) => {
                        table(ui, inner, cx);
                        gap(ui, cx);
                    }
                    _ => blocks(ui, inner, cx),
                }
                i = end + 1;
            }
            Event::Rule => {
                ui.separator();
                i += 1;
            }
            Event::Html(h) => {
                ui.label(RichText::new(h.trim_end()).monospace().color(theme::TEXT_MUTED));
                i += 1;
            }
            _ => i += 1,
        }
    }
}

fn quote(ui: &mut Ui, inner: &[Event], cx: &mut Cx) {
    let out = Frame::new()
        .inner_margin(Margin {
            left: 12,
            right: 0,
            top: 2,
            bottom: 2,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            blocks(ui, inner, cx);
        });
    let r = out.response.rect;
    ui.painter().line_segment(
        [Pos2::new(r.left() + 3.0, r.top()), Pos2::new(r.left() + 3.0, r.bottom())],
        Stroke::new(2.5, theme::neon()),
    );
    gap(ui, cx);
}

fn code_block(ui: &mut Ui, code: &str, lang: &str) {
    Frame::new()
        .fill(theme::bg_input())
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !lang.is_empty() {
                ui.label(theme::muted(lang).small());
            }
            ui.label(RichText::new(code.trim_end_matches('\n')).monospace());
        });
}

fn list(ui: &mut Ui, ev: &[Event], start: Option<u64>, cx: &mut Cx) {
    let mut number = start.unwrap_or(1);
    let mut i = 0;
    cx.list_depth += 1;
    while i < ev.len() {
        if let Event::Start(Tag::Item) = &ev[i] {
            let end = matching_end(ev, i);
            let inner = &ev[i + 1..end];
            let marker = if start.is_some() {
                format!("{number}.")
            } else if cx.list_depth > 1 {
                "–".to_owned()
            } else {
                "•".to_owned()
            };
            number += 1;
            let rtl = first_text(inner).is_some_and(|t| textdir::is_rtl(&t));
            item(ui, &marker, rtl, inner, cx);
            i = end + 1;
        } else {
            i += 1;
        }
    }
    cx.list_depth -= 1;
}

fn first_text(ev: &[Event]) -> Option<String> {
    ev.iter().find_map(|e| match e {
        Event::Text(t) if t.chars().any(char::is_alphabetic) => Some(t.to_string()),
        _ => None,
    })
}

/// One list item: marker on the left, or on the right for right-to-left text.
fn item(ui: &mut Ui, marker: &str, rtl: bool, inner: &[Event], cx: &mut Cx) {
    const MARKER_W: f32 = 26.0;
    let spacing = 6.0;
    let content_w = (ui.available_width() - MARKER_W - spacing).max(40.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = spacing;
        let marker_label = |ui: &mut Ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(MARKER_W, 0.0),
                egui::Layout::top_down(if rtl { egui::Align::LEFT } else { egui::Align::RIGHT }),
                |ui| ui.label(RichText::new(marker).color(theme::neon_hot())),
            );
        };
        if !rtl {
            marker_label(ui);
        }
        ui.vertical(|ui| {
            ui.set_width(content_w);
            blocks(ui, inner, cx);
        });
        if rtl {
            marker_label(ui);
        }
    });
}

fn table(ui: &mut Ui, ev: &[Event], cx: &mut Cx) {
    let mut rows: Vec<Vec<Vec<Span>>> = Vec::new();
    let mut i = 0;
    while i < ev.len() {
        match &ev[i] {
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
                let end = matching_end(ev, i);
                let mut cells = Vec::new();
                let mut j = i + 1;
                // The head holds cells directly; rows hold cells too.
                while j < end {
                    if let Event::Start(Tag::TableCell) = &ev[j] {
                        let ce = matching_end(ev, j);
                        cells.push(spans(&ev[j + 1..ce]));
                        j = ce + 1;
                    } else {
                        j += 1;
                    }
                }
                rows.push(cells);
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    egui::Grid::new(ui.next_auto_id())
        .striped(true)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for (r, row) in rows.iter().enumerate() {
                for cell in row {
                    let kind = if r == 0 { Kind::Strong } else { Kind::Body };
                    inline_label(ui, cell, kind, cx);
                }
                ui.end_row();
            }
        });
}

// ---------------------------------------------------------------------
// inline content
// ---------------------------------------------------------------------

#[derive(Clone, Default)]
struct Span {
    text: String,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    /// Link or raw html: shown as is, never spell-checked.
    literal: bool,
    link: bool,
}

fn spans(ev: &[Event]) -> Vec<Span> {
    let (mut bold, mut italic, mut strike, mut link) = (0u32, 0u32, 0u32, 0u32);
    let mut out: Vec<Span> = Vec::new();
    for e in ev {
        let mut push = |text: String, code: bool, literal: bool| {
            out.push(Span {
                text,
                bold: bold > 0,
                italic: italic > 0,
                strike: strike > 0,
                code,
                literal: literal || link > 0,
                link: link > 0,
            });
        };
        match e {
            Event::Text(t) => push(t.to_string(), false, false),
            Event::Code(c) => push(c.to_string(), true, true),
            Event::SoftBreak | Event::HardBreak => push("\n".into(), false, false),
            Event::InlineHtml(h) => push(h.to_string(), false, true),
            Event::InlineMath(m) => push(m.to_string(), true, true),
            Event::TaskListMarker(done) => push(if *done { "☑ " } else { "☐ " }.into(), false, true),
            Event::FootnoteReference(_) => {}
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => strike += 1,
            Event::End(TagEnd::Strikethrough) => strike = strike.saturating_sub(1),
            Event::Start(Tag::Link { .. }) => link += 1,
            Event::End(TagEnd::Link) => link = link.saturating_sub(1),
            _ => {}
        }
    }
    out
}

#[derive(Clone, Copy)]
enum Kind {
    Body,
    Strong,
    Heading(HeadingLevel),
}

struct Hit {
    chars: Range<usize>,
    verdict: Verdict,
    word: String,
}

fn inline_label(ui: &mut Ui, spans: &[Span], kind: Kind, cx: &mut Cx) {
    if spans.iter().all(|s| s.text.trim().is_empty()) {
        return;
    }
    let body = ui.style().text_styles[&TextStyle::Body].size;
    let (size, heading) = match kind {
        Kind::Heading(HeadingLevel::H1) => (body * 1.6, true),
        Kind::Heading(HeadingLevel::H2) => (body * 1.35, true),
        Kind::Heading(HeadingLevel::H3) => (body * 1.18, true),
        Kind::Heading(_) => (body * 1.08, true),
        _ => (body, false),
    };
    let strong_color = ui.visuals().strong_text_color();
    let text_color = ui.visuals().text_color();

    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    let mut hits: Vec<Hit> = Vec::new();
    let mut pos = 0usize; // in chars

    for s in spans {
        let strong = s.bold || heading || matches!(kind, Kind::Strong);
        let base = TextFormat {
            font_id: if s.code {
                FontId::new(size * 0.92, FontFamily::Monospace)
            } else {
                FontId::new(size, FontFamily::Proportional)
            },
            color: if s.link {
                ui.visuals().hyperlink_color
            } else if strong {
                strong_color
            } else {
                text_color
            },
            italics: s.italic,
            strikethrough: if s.strike { Stroke::new(1.0, text_color) } else { Stroke::NONE },
            underline: if s.link { Stroke::new(1.0, ui.visuals().hyperlink_color) } else { Stroke::NONE },
            background: if s.code { theme::bg_input() } else { Color32::TRANSPARENT },
            ..Default::default()
        };
        let annotations = match cx.checker {
            Some(c) if !s.literal => c.annotate(&s.text),
            _ => Vec::new(),
        };
        let mut at = 0;
        for (range, verdict) in annotations {
            if at < range.start {
                job.append(&s.text[at..range.start], 0.0, base.clone());
                pos += s.text[at..range.start].chars().count();
            }
            let word = &s.text[range.clone()];
            let color = match verdict {
                Verdict::Defined(_) | Verdict::Variable => DEFINED,
                Verdict::Misspelled => MISSPELLED,
            };
            let mut fmt = base.clone();
            fmt.color = color;
            fmt.underline = Stroke::new(1.2, color);
            job.append(word, 0.0, fmt);
            let n = word.chars().count();
            hits.push(Hit {
                chars: pos..pos + n,
                verdict,
                word: word.to_owned(),
            });
            pos += n;
            at = range.end;
        }
        if at < s.text.len() {
            job.append(&s.text[at..], 0.0, base);
            pos += s.text[at..].chars().count();
        }
    }

    let galley_job = job.clone();
    let label = egui::Label::new(job).selectable(true).sense(Sense::click());
    let resp = ui.add(label);

    if hits.is_empty() {
        return;
    }
    let galley = ui.painter().layout_job(galley_job);
    let hot = resp
        .hover_pos()
        .and_then(|p| char_at(&galley, p - resp.rect.min))
        .and_then(|ci| hits.iter().find(|h| h.chars.contains(&ci)));
    if let Some(h) = hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        let tip = match (h.verdict, cx.checker) {
            (Verdict::Defined(i), Some(c)) => c
                .definition_of(i)
                .map(|(_, d)| short_tip(d))
                .unwrap_or_else(|| "Defined term".to_owned()),
            (Verdict::Variable, _) => "Variable — click to edit its value".to_owned(),
            _ => "Misspelled — click for suggestions".to_owned(),
        };
        resp.clone().on_hover_text(tip);
        if resp.clicked() {
            let at = resp.interact_pointer_pos().unwrap_or(resp.rect.left_bottom());
            cx.action = Some(match h.verdict {
                Verdict::Defined(index) => Action::Defined { index, at },
                Verdict::Variable => Action::Variable {
                    name: crate::model::scan_variables(&h.word)
                        .into_iter()
                        .next()
                        .map(|(_, n)| n)
                        .unwrap_or_default(),
                },
                Verdict::Misspelled => Action::Misspelled {
                    word: h.word.clone(),
                    at,
                },
            });
        }
    }
}

fn short_tip(s: &str) -> String {
    let t = s.trim();
    let mut out: String = t.chars().take(140).collect();
    if t.chars().count() > 140 {
        out.push('…');
    }
    if out.is_empty() {
        "Defined term (no definition yet)".to_owned()
    } else {
        out
    }
}

/// Which character of the galley is under `pos` (galley-relative)?
fn char_at(galley: &Arc<Galley>, pos: Vec2) -> Option<usize> {
    let mut base = 0usize;
    for row in &galley.rows {
        if pos.y >= row.min_y() && pos.y <= row.max_y() {
            for (i, g) in row.glyphs.iter().enumerate() {
                let r = g.logical_rect().translate(row.pos.to_vec2());
                if pos.x >= r.left() && pos.x <= r.right() {
                    return Some(base + i);
                }
            }
            return None;
        }
        base += row.char_count_including_newline().0;
    }
    None
}
