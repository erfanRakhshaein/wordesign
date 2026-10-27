//! Data model for a prompt.
//!
//! A prompt starts life as an empty note. The user can then opt in to extra
//! structure: a Context, a list of Definitions, a list of Rules (with
//! examples) and inline mermaid "thought flows". Independently of all that, a
//! prompt carries a private set of benchmark questions with per-model answers
//! that are NEVER part of the compiled / exported prompt.
//!
//! # File format (`.aiprompt`, plain text, one file per prompt)
//!
//! ```text
//! @@AIPROMPT 2
//! @@BODY
//! @TEXT
//! free markdown text...
//! @FLOW flow1 :: Title shown when collapsed
//! graph TD; A-->B
//! @TEXT
//! more text
//! @CODE code1 :: Optional title
//! pasted code, exported inside a ``` fence
//! @@CONTEXT            (only present if the section exists)
//! text
//! @@DEFINITIONS
//! @TERM some term
//! its definition
//! @@RULES
//! @RULE
//! rule text
//! @EXAMPLE
//! example text
//! @@VARIABLES          (values for {{name}} placeholders; only filled ones are stored)
//! @VAR name
//! value
//! @@BENCHMARKS
//! @Q
//! question text
//! @EXPECTED           (optional: what a good answer should contain)
//! expected answer
//! @A model name
//! answer text
//! ```
//!
//! A line starting with `@` is a marker. Content lines that would otherwise
//! look like a marker (start with `@`) or like an escape (start with `\`) are
//! written with one extra leading backslash and un-escaped when read, so any
//! text round-trips safely.

#[derive(Clone, Debug, Default)]
pub struct Definition {
    pub term: String,
    pub definition: String,
}

#[derive(Clone, Debug)]
pub struct Flow {
    pub id: String,
    pub title: String,
    pub mermaid: String,
    /// UI only (not stored): collapsed flows show just their title.
    pub collapsed: bool,
}

/// A pasted piece of code. Exported inside a fenced block, so the user never types the fences.
#[derive(Clone, Debug)]
pub struct CodeBlock {
    pub id: String,
    pub title: String,
    pub code: String,
    /// UI only (not stored): collapsed blocks show just their title.
    pub collapsed: bool,
}

#[derive(Clone, Debug)]
pub enum BodyBlock {
    Text(String),
    Flow(Flow),
    Code(CodeBlock),
}

/// The value given to a `{{name}}` placeholder.
#[derive(Clone, Debug, Default)]
pub struct Variable {
    pub name: String,
    pub value: String,
}

/// Every well-formed `{{name}}` in `text`: byte range of the whole tag, and the name.
/// Names may contain letters, digits, `_`, `-` and `.`; spaces around the name are allowed.
pub fn scan_variables(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = text[i..].find("{{") {
        let start = i + off;
        let Some(end_off) = text[start + 2..].find("}}") else { break };
        let inner = &text[start + 2..start + 2 + end_off];
        let name = inner.trim();
        let valid = !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'));
        if valid {
            let end = start + 2 + end_off + 2;
            out.push((start..end, name.to_owned()));
            i = end;
        } else {
            i = start + 1;
        }
    }
    out
}

/// Byte ranges of code in markdown text: fenced blocks and `inline code` spans.
fn code_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut fence = FenceState::default();
    let mut at = 0usize;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        if fence.feed(content) {
            out.push(at..at + line.len());
        } else {
            // inline spans: a run of n backticks closes at the next run of exactly n
            let bytes = content.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] != b'`' {
                    i += 1;
                    continue;
                }
                let n = bytes[i..].iter().take_while(|b| **b == b'`').count();
                let mut j = i + n;
                let mut close = None;
                while j < bytes.len() {
                    if bytes[j] == b'`' {
                        let m = bytes[j..].iter().take_while(|b| **b == b'`').count();
                        if m == n {
                            close = Some(j + m);
                            break;
                        }
                        j += m;
                    } else {
                        j += 1;
                    }
                }
                match close {
                    Some(end) => {
                        out.push(at + i..at + end);
                        i = end;
                    }
                    None => i += n,
                }
            }
        }
        at += line.len();
    }
    out
}

/// Like `scan_variables`, but tags inside code (fences, `backticks`) are literal text, not variables.
pub fn scan_prose_variables(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let tags = scan_variables(text);
    if tags.is_empty() || !text.contains('`') {
        return tags;
    }
    let code = code_ranges(text);
    tags.into_iter()
        .filter(|(r, _)| !code.iter().any(|c| c.start < r.end && r.start < c.end))
        .collect()
}

/// Replace every `{{name}}` using `lookup` (single pass: inserted values are never re-expanded).
/// Tags `lookup` does not know are left untouched.
fn substitute_variables(text: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (range, name) in scan_prose_variables(text) {
        out.push_str(&text[at..range.start]);
        match lookup(&name) {
            Some(v) => out.push_str(&v),
            None => out.push_str(&text[range.clone()]),
        }
        at = range.end;
    }
    out.push_str(&text[at..]);
    out
}

#[derive(Clone, Debug, Default)]
pub struct Rule {
    pub text: String,
    pub examples: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Answer {
    pub model: String,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Benchmark {
    pub question: String,
    /// Optional reference answer, for reviewing the model answers later.
    pub expected: Option<String>,
    pub answers: Vec<Answer>,
}

#[derive(Clone, Debug)]
pub struct Prompt {
    pub body: Vec<BodyBlock>,
    pub context: Option<String>,
    pub definitions: Option<Vec<Definition>>,
    pub rules: Option<Vec<Rule>>,
    /// Values of `{{name}}` placeholders (may include names no longer used).
    pub variables: Vec<Variable>,
    pub benchmarks: Vec<Benchmark>,
}

impl Default for Prompt {
    fn default() -> Self {
        Self::new()
    }
}

impl Prompt {
    /// A fresh prompt: one empty note, nothing else.
    pub fn new() -> Self {
        Self {
            body: vec![BodyBlock::Text(String::new())],
            context: None,
            definitions: None,
            rules: None,
            variables: Vec::new(),
            benchmarks: Vec::new(),
        }
    }

    /// True when nothing has been written yet (used to avoid saving empty files).
    pub fn is_blank(&self) -> bool {
        self.to_file() == Prompt::new().to_file()
    }

    pub fn next_flow_id(&self) -> String {
        let max = self
            .body
            .iter()
            .filter_map(|b| match b {
                BodyBlock::Flow(f) => f.id.strip_prefix("flow")?.parse::<u32>().ok(),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        format!("flow{}", max + 1)
    }

    /// Insert a new (expanded) flow after body block `after`, making sure
    /// there is always a text block after it to keep typing in.
    /// Returns the index of the new flow.
    pub fn insert_flow_after(&mut self, after: usize) -> usize {
        let at = (after + 1).min(self.body.len());
        let flow = Flow {
            id: self.next_flow_id(),
            title: "Untitled flow".to_string(),
            mermaid: "graph TD\n    A[Start] --> B[End]".to_string(),
            collapsed: false,
        };
        self.body.insert(at, BodyBlock::Flow(flow));
        if !matches!(self.body.get(at + 1), Some(BodyBlock::Text(_))) {
            self.body.insert(at + 1, BodyBlock::Text(String::new()));
        }
        at
    }

    pub fn next_code_id(&self) -> String {
        let max = self
            .body
            .iter()
            .filter_map(|b| match b {
                BodyBlock::Code(c) => c.id.strip_prefix("code")?.parse::<u32>().ok(),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        format!("code{}", max + 1)
    }

    /// Insert a new (expanded) code block after body block `after`, keeping a text block after it.
    pub fn insert_code_after(&mut self, after: usize) -> usize {
        let at = (after + 1).min(self.body.len());
        let block = CodeBlock {
            id: self.next_code_id(),
            title: String::new(),
            code: String::new(),
            collapsed: false,
        };
        self.body.insert(at, BodyBlock::Code(block));
        if !matches!(self.body.get(at + 1), Some(BodyBlock::Text(_))) {
            self.body.insert(at + 1, BodyBlock::Text(String::new()));
        }
        at
    }

    /// Every text that is prose of the prompt (and so may contain `{{variables}}`).
    /// Flows and code blocks are skipped: mermaid uses `{{…}}` for hexagons and code may be a template.
    fn variable_texts(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        if let Some(c) = &self.context {
            v.push(c);
        }
        for d in self.definitions.iter().flatten() {
            v.push(&d.term);
            v.push(&d.definition);
        }
        for b in &self.body {
            if let BodyBlock::Text(t) = b {
                v.push(t);
            }
        }
        for r in self.rules.iter().flatten() {
            v.push(&r.text);
            v.extend(r.examples.iter().map(String::as_str));
        }
        v
    }

    /// Distinct variable names used in the prompt, in order of first appearance
    /// (context, definitions, main, rules — the order they are exported in).
    pub fn used_variables(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for t in self.variable_texts() {
            for (_, name) in scan_prose_variables(t) {
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        out
    }

    pub fn variable_value(&self, name: &str) -> Option<&str> {
        self.variables.iter().find(|v| v.name == name).map(|v| v.value.as_str())
    }

    /// Mutable access to a variable's value, creating it when missing.
    pub fn variable_value_mut(&mut self, name: &str) -> &mut String {
        let i = match self.variables.iter().position(|v| v.name == name) {
            Some(i) => i,
            None => {
                self.variables.push(Variable { name: name.to_owned(), value: String::new() });
                self.variables.len() - 1
            }
        };
        &mut self.variables[i].value
    }

    /// Used variables that have no value yet.
    pub fn unaddressed_variables(&self) -> Vec<String> {
        self.used_variables()
            .into_iter()
            .filter(|n| self.variable_value(n).is_none_or(|v| v.trim().is_empty()))
            .collect()
    }

    /// A copy with every `{{name}}` replaced by its value (only in prose, see `variable_texts`).
    fn with_variables_filled(&self) -> Prompt {
        let mut p = self.clone();
        let lookup = |n: &str| self.variable_value(n).map(|v| v.trim_end().to_owned());
        let fill = |s: &mut String| *s = substitute_variables(s, &lookup);
        p.context.iter_mut().for_each(fill);
        for d in p.definitions.iter_mut().flatten() {
            fill(&mut d.term);
            fill(&mut d.definition);
        }
        for b in &mut p.body {
            if let BodyBlock::Text(t) = b {
                fill(t);
            }
        }
        for r in p.rules.iter_mut().flatten() {
            fill(&mut r.text);
            r.examples.iter_mut().for_each(fill);
        }
        p
    }

    /// Distinct, non-empty model names used in benchmark answers, in order of first use.
    pub fn known_models(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for b in &self.benchmarks {
            for a in &b.answers {
                let m = a.model.trim();
                if !m.is_empty() && !out.iter().any(|x| x == m) {
                    out.push(m.to_string());
                }
            }
        }
        out
    }

    // ------------------------------------------------------------------
    // Serialization
    // ------------------------------------------------------------------

    pub fn to_file(&self) -> String {
        let mut o = String::from("@@AIPROMPT 2\n@@BODY\n");
        for b in &self.body {
            match b {
                BodyBlock::Text(t) => {
                    o.push_str("@TEXT\n");
                    push_content(&mut o, t);
                }
                BodyBlock::Flow(f) => {
                    o.push_str(&format!("@FLOW {} :: {}\n", f.id, one_line(&f.title)));
                    push_content(&mut o, &f.mermaid);
                }
                BodyBlock::Code(c) => {
                    o.push_str(&format!("@CODE {} :: {}\n", c.id, one_line(&c.title)));
                    push_content(&mut o, &c.code);
                }
            }
        }
        if let Some(c) = &self.context {
            o.push_str("@@CONTEXT\n");
            push_content(&mut o, c);
        }
        if let Some(defs) = &self.definitions {
            o.push_str("@@DEFINITIONS\n");
            for d in defs {
                o.push_str(&format!("@TERM {}\n", one_line(&d.term)));
                push_content(&mut o, &d.definition);
            }
        }
        if let Some(rules) = &self.rules {
            o.push_str("@@RULES\n");
            for r in rules {
                o.push_str("@RULE\n");
                push_content(&mut o, &r.text);
                for ex in &r.examples {
                    o.push_str("@EXAMPLE\n");
                    push_content(&mut o, ex);
                }
            }
        }
        let filled: Vec<&Variable> = self.variables.iter().filter(|v| !v.value.trim().is_empty()).collect();
        if !filled.is_empty() {
            o.push_str("@@VARIABLES\n");
            for v in filled {
                o.push_str(&format!("@VAR {}\n", one_line(&v.name)));
                push_content(&mut o, &v.value);
            }
        }
        if !self.benchmarks.is_empty() {
            o.push_str("@@BENCHMARKS\n");
            for b in &self.benchmarks {
                o.push_str("@Q\n");
                push_content(&mut o, &b.question);
                if let Some(e) = b.expected.as_deref().filter(|e| !e.trim().is_empty()) {
                    o.push_str("@EXPECTED\n");
                    push_content(&mut o, e);
                }
                for a in &b.answers {
                    o.push_str(&format!("@A {}\n", one_line(&a.model)));
                    push_content(&mut o, &a.text);
                }
            }
        }
        o
    }

    pub fn from_file(src: &str) -> Self {
        let src = src.strip_prefix('\u{feff}').unwrap_or(src);

        // Not one of ours (no markers at all): treat the whole thing as a note.
        if !src.trim_start().starts_with("@@") {
            let mut p = Prompt::new();
            p.body = vec![BodyBlock::Text(src.trim_end().to_string())];
            return p;
        }

        #[derive(Clone, Copy)]
        enum T {
            None,
            BodyText,
            FlowSrc,
            CodeSrc,
            VarValue,
            Context,
            DefText,
            RuleText,
            RuleExample,
            BenchQ,
            BenchExpected,
            BenchA,
        }

        let mut p = Prompt {
            body: Vec::new(),
            context: None,
            definitions: None,
            rules: None,
            variables: Vec::new(),
            benchmarks: Vec::new(),
        };
        let mut t = T::None;

        for line in src.lines() {
            if let Some(name) = line.strip_prefix("@@") {
                t = T::None;
                match name.trim() {
                    "BODY" | "TASK" => {}
                    "CONTEXT" => {
                        p.context.get_or_insert_with(String::new);
                        t = T::Context;
                    }
                    "DEFINITIONS" => {
                        p.definitions.get_or_insert_with(Vec::new);
                    }
                    "RULES" => {
                        p.rules.get_or_insert_with(Vec::new);
                    }
                    "BENCHMARKS" => {}
                    _ => {}
                }
            } else if let Some(rest) = line.strip_prefix('@') {
                let (kind, arg) = rest.split_once(' ').unwrap_or((rest, ""));
                let arg = arg.trim();
                t = T::None;
                match kind.trim_end() {
                    "TEXT" => {
                        p.body.push(BodyBlock::Text(String::new()));
                        t = T::BodyText;
                    }
                    "FLOW" => {
                        let (id, title) = match arg.split_once(" :: ") {
                            Some((i, ti)) => (i.trim().to_string(), ti.trim().to_string()),
                            None => (arg.to_string(), "Untitled flow".to_string()),
                        };
                        p.body.push(BodyBlock::Flow(Flow {
                            id,
                            title,
                            mermaid: String::new(),
                            collapsed: true,
                        }));
                        t = T::FlowSrc;
                    }
                    "CODE" => {
                        let (id, title) = match arg.split_once(" :: ") {
                            Some((i, ti)) => (i.trim().to_string(), ti.trim().to_string()),
                            None => (arg.trim_end_matches("::").trim().to_string(), String::new()),
                        };
                        p.body.push(BodyBlock::Code(CodeBlock {
                            id,
                            title,
                            code: String::new(),
                            collapsed: true,
                        }));
                        t = T::CodeSrc;
                    }
                    "VAR" => {
                        p.variables.push(Variable { name: arg.to_string(), value: String::new() });
                        t = T::VarValue;
                    }
                    "TERM" => {
                        p.definitions.get_or_insert_with(Vec::new).push(Definition {
                            term: arg.to_string(),
                            definition: String::new(),
                        });
                        t = T::DefText;
                    }
                    "RULE" => {
                        p.rules.get_or_insert_with(Vec::new).push(Rule::default());
                        t = T::RuleText;
                    }
                    "EXAMPLE" => {
                        if let Some(r) = p.rules.as_mut().and_then(|r| r.last_mut()) {
                            r.examples.push(String::new());
                            t = T::RuleExample;
                        }
                    }
                    "Q" => {
                        p.benchmarks.push(Benchmark::default());
                        t = T::BenchQ;
                    }
                    "EXPECTED" => {
                        if let Some(b) = p.benchmarks.last_mut() {
                            b.expected = Some(String::new());
                            t = T::BenchExpected;
                        }
                    }
                    "A" => {
                        if let Some(b) = p.benchmarks.last_mut() {
                            b.answers.push(Answer {
                                model: arg.to_string(),
                                text: String::new(),
                            });
                            t = T::BenchA;
                        }
                    }
                    _ => {} // unknown marker (or v1 "ENDFLOW"): ignore its content
                }
            } else {
                let content = line.strip_prefix('\\').map_or(line, |_| &line[1..]);
                let dst: Option<&mut String> = match t {
                    T::None => None,
                    T::BodyText => match p.body.last_mut() {
                        Some(BodyBlock::Text(s)) => Some(s),
                        _ => None,
                    },
                    T::FlowSrc => match p.body.last_mut() {
                        Some(BodyBlock::Flow(f)) => Some(&mut f.mermaid),
                        _ => None,
                    },
                    T::CodeSrc => match p.body.last_mut() {
                        Some(BodyBlock::Code(c)) => Some(&mut c.code),
                        _ => None,
                    },
                    T::VarValue => p.variables.last_mut().map(|v| &mut v.value),
                    T::Context => p.context.as_mut(),
                    T::DefText => p
                        .definitions
                        .as_mut()
                        .and_then(|d| d.last_mut())
                        .map(|d| &mut d.definition),
                    T::RuleText => p.rules.as_mut().and_then(|r| r.last_mut()).map(|r| &mut r.text),
                    T::RuleExample => p
                        .rules
                        .as_mut()
                        .and_then(|r| r.last_mut())
                        .and_then(|r| r.examples.last_mut()),
                    T::BenchQ => p.benchmarks.last_mut().map(|b| &mut b.question),
                    T::BenchExpected => p.benchmarks.last_mut().and_then(|b| b.expected.as_mut()),
                    T::BenchA => p
                        .benchmarks
                        .last_mut()
                        .and_then(|b| b.answers.last_mut())
                        .map(|a| &mut a.text),
                };
                if let Some(d) = dst {
                    d.push_str(content);
                    d.push('\n');
                }
            }
        }

        // Trim the trailing newline we added after every content line.
        let trim = |s: &mut String| {
            let n = s.trim_end().len();
            s.truncate(n);
        };
        for b in &mut p.body {
            match b {
                BodyBlock::Text(s) => trim(s),
                BodyBlock::Flow(f) => trim(&mut f.mermaid),
                BodyBlock::Code(c) => trim(&mut c.code),
            }
        }
        for v in &mut p.variables {
            trim(&mut v.value);
        }
        if let Some(c) = p.context.as_mut() {
            trim(c);
        }
        for d in p.definitions.iter_mut().flatten() {
            trim(&mut d.definition);
        }
        for r in p.rules.iter_mut().flatten() {
            trim(&mut r.text);
            r.examples.iter_mut().for_each(trim);
        }
        for b in &mut p.benchmarks {
            trim(&mut b.question);
            if let Some(e) = b.expected.as_mut() {
                trim(e);
            }
            for a in &mut b.answers {
                trim(&mut a.text);
            }
        }
        if p.body.is_empty() {
            p.body.push(BodyBlock::Text(String::new()));
        }
        p
    }

    // ------------------------------------------------------------------
    // Compilation (preview + export). Benchmarks are deliberately absent.
    // ------------------------------------------------------------------

    pub fn compile_markdown(&self) -> String {
        let context = self.context.as_deref().filter(|c| !c.trim().is_empty());
        let defs: Vec<&Definition> = self
            .definitions
            .iter()
            .flatten()
            .filter(|d| !d.term.trim().is_empty())
            .collect();
        let rules: Vec<&Rule> = self
            .rules
            .iter()
            .flatten()
            .filter(|r| !r.text.trim().is_empty())
            .collect();

        // Section headings only appear once the prompt has real structure.
        let structured = context.is_some() || !defs.is_empty() || !rules.is_empty();

        let mut parts: Vec<String> = Vec::new();

        if let Some(c) = context {
            parts.push(format!("# Context\n\n{}", demote_headers(c.trim_end())));
        }

        if !defs.is_empty() {
            let items: Vec<String> = defs
                .iter()
                .map(|d| list_item("", "- ", &format!("**{}**: {}", d.term.trim(), d.definition.trim())))
                .collect();
            parts.push(format!("# Definitions\n\n{}", items.join("\n")));
        }

        let mut body_parts: Vec<String> = Vec::new();
        for b in &self.body {
            match b {
                BodyBlock::Text(t) if !t.trim().is_empty() => {
                    body_parts.push(demote_headers(t.trim_end()));
                }
                BodyBlock::Flow(f) if !f.mermaid.trim().is_empty() => {
                    let mut s = String::new();
                    if !f.title.trim().is_empty() {
                        s.push_str(&format!("## {}\n\n", f.title.trim()));
                    }
                    s.push_str(&format!("```mermaid\n{}\n```", f.mermaid.trim_end()));
                    body_parts.push(s);
                }
                BodyBlock::Code(c) if !c.code.trim().is_empty() => {
                    let mut s = String::new();
                    if !c.title.trim().is_empty() {
                        s.push_str(&format!("**{}**\n\n", c.title.trim()));
                    }
                    s.push_str(&fenced(&c.code));
                    body_parts.push(s);
                }
                _ => {}
            }
        }
        if !body_parts.is_empty() {
            let body = body_parts.join("\n\n");
            parts.push(if structured {
                format!("# Main\n\n{body}")
            } else {
                body
            });
        }

        if !rules.is_empty() {
            let mut items: Vec<String> = Vec::new();
            for (i, r) in rules.iter().enumerate() {
                let marker = format!("{}. ", i + 1);
                let mut item = list_item("", &marker, r.text.trim());
                for ex in r.examples.iter().filter(|e| !e.trim().is_empty()) {
                    item.push('\n');
                    item.push_str(&list_item(&" ".repeat(marker.len()), "- Example: ", ex.trim()));
                }
                items.push(item);
            }
            parts.push(format!("# Rules\n\n{}", items.join("\n")));
        }

        parts.join("\n\n")
    }

    /// What Export puts on the clipboard: the compiled markdown with all
    /// purely-visual whitespace removed.
    /// Variables are replaced by their values. Check `unaddressed_variables` first: tags without a value stay as written.
    pub fn export_minified(&self) -> String {
        minify_markdown(&self.with_variables_filled().compile_markdown())
    }
}

/// Rough token count for the preview footer. Latin text is ~4 characters per
/// token, but CJK text is ~1 token per character and other scripts (Arabic,
/// Cyrillic, accented Latin, ...) sit in between.
pub fn estimate_tokens(text: &str) -> usize {
    let (mut ascii, mut cjk, mut other) = (0usize, 0usize, 0usize);
    for c in text.chars() {
        if c.is_ascii() || c.is_whitespace() {
            ascii += 1;
        } else if matches!(
            c as u32,
            0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF
        ) {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    ascii.div_ceil(4) + cjk + other.div_ceil(2)
}

// ----------------------------------------------------------------------
// helpers
// ----------------------------------------------------------------------

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn push_content(out: &mut String, text: &str) {
    for line in text.trim_end().lines() {
        if line.starts_with('@') || line.starts_with('\\') {
            out.push('\\');
        }
        out.push_str(line);
        out.push('\n');
    }
}

/// Format `text` as a markdown list item. Continuation lines are indented so
/// multi-paragraph definitions / examples stay inside their list item.
fn list_item(indent: &str, marker: &str, text: &str) -> String {
    let cont = " ".repeat(indent.len() + marker.len());
    let mut out = String::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 {
            out.push_str(indent);
            out.push_str(marker);
            out.push_str(line.trim_end());
        } else {
            out.push('\n');
            if !line.trim().is_empty() {
                out.push_str(&cont);
                out.push_str(line.trim_end());
            }
        }
    }
    out
}

/// `(fence char, fence length, rest of the line)` if `line` is a code fence marker.
fn fence_of(line: &str) -> Option<(char, usize, &str)> {
    let t = line.trim_start();
    let ch = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = t.chars().take_while(|c| *c == ch).count();
    (len >= 3).then(|| (ch, len, &t[len..]))
}

/// Tracks whether a line is inside a fenced code block (length-aware, so a ```` fence may contain ```).
#[derive(Default)]
struct FenceState(Option<(char, usize)>);

impl FenceState {
    /// Feed the next line; returns true when the line belongs to a fence (marker or content).
    fn feed(&mut self, line: &str) -> bool {
        match (self.0, fence_of(line)) {
            (None, Some((c, n, _))) => {
                self.0 = Some((c, n));
                true
            }
            (Some((oc, on)), Some((c, n, rest))) if c == oc && n >= on && rest.trim().is_empty() => {
                self.0 = None;
                true
            }
            (Some(_), _) => true,
            (None, None) => false,
        }
    }
}

/// Wrap code in a fence that is longer than any backtick run inside it. A pasted block that
/// already carries its own opening/closing fence is not wrapped twice.
fn fenced(code: &str) -> String {
    let mut lines: Vec<&str> = code.trim_matches('\n').lines().collect();
    if lines.len() >= 2
        && lines.first().is_some_and(|l| fence_of(l).is_some_and(|(c, ..)| c == '`'))
        && lines.last().is_some_and(|l| fence_of(l).is_some_and(|(c, _, rest)| c == '`' && rest.trim().is_empty()))
    {
        lines = lines[1..lines.len() - 1].to_vec();
    }
    let body = lines.join("\n");
    let longest = body
        .lines()
        .map(|l| l.trim_start().chars().take_while(|c| *c == '`').count())
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}\n{}\n{fence}", body.trim_end())
}

/// User-written `# Heading` (h1) becomes `## Heading` so it never competes
/// with the auto-generated h1 section headings. Code fences are left alone.
fn demote_headers(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut fence = FenceState::default();
    for line in text.lines() {
        if fence.feed(line) {
            out.push(line.to_string());
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let t = line.trim_start();
        let is_h1 = indent <= 3 && (t == "#" || t.starts_with("# "));
        if is_h1 {
            out.push(format!("{}#{}", &line[..indent], t));
        } else {
            out.push(line.to_string());
        }
    }
    out.join("\n")
}

/// Remove whitespace that exists only for the human reader: trailing spaces,
/// tabs and runs of blank lines. Fenced code blocks keep their internal
/// layout (tabs / blank lines can be significant there).
fn minify_markdown(md: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut fence = FenceState::default();
    let mut prev_blank = true; // also drops leading blank lines
    for raw in md.lines() {
        if fence.feed(raw) {
            lines.push(raw.trim_end().to_string());
            prev_blank = false;
            continue;
        }
        let cleaned = raw.replace('\t', " ");
        let cleaned = cleaned.trim_end();
        if cleaned.is_empty() {
            if !prev_blank {
                lines.push(String::new());
            }
            prev_blank = true;
        } else {
            lines.push(cleaned.to_string());
            prev_blank = false;
        }
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(p: &Prompt) -> String {
        match &p.body[0] {
            BodyBlock::Text(t) => t.clone(),
            _ => panic!("first block is not text"),
        }
    }

    #[test]
    fn new_prompt_is_an_empty_note_without_headers() {
        let p = Prompt::new();
        assert!(p.is_blank());
        assert_eq!(p.compile_markdown(), "");
        assert_eq!(p.export_minified(), "");
    }

    #[test]
    fn plain_note_exports_with_no_headers() {
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("Write a haiku about rust.\n\nKeep it short.".into())];
        assert_eq!(p.export_minified(), "Write a haiku about rust.\n\nKeep it short.");
        assert!(!p.compile_markdown().contains("# "));
    }

    #[test]
    fn sections_appear_only_when_added_and_note_becomes_task() {
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("Do the thing.".into())];
        p.context = Some("You are a careful editor.".into());
        p.rules = Some(vec![Rule {
            text: "Be brief.".into(),
            examples: vec!["Good: ok".into()],
        }]);
        let md = p.compile_markdown();
        assert_eq!(
            md,
            "# Context\n\nYou are a careful editor.\n\n# Main\n\nDo the thing.\n\n# Rules\n\n1. Be brief.\n   - Example: Good: ok"
        );
    }

    #[test]
    fn empty_sections_are_skipped_in_export() {
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("Only a note".into())];
        p.context = Some("   ".into());
        p.definitions = Some(vec![Definition::default()]);
        p.rules = Some(vec![Rule::default()]);
        assert_eq!(p.export_minified(), "Only a note");
    }

    #[test]
    fn user_h1_is_demoted_but_code_fences_are_untouched() {
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text(
            "# My header\n## Already h2\n```python\n# a comment\nx = 1\n```\n    # indented code".into(),
        )];
        let md = p.compile_markdown();
        assert!(md.contains("## My header"));
        assert!(md.contains("## Already h2"));
        assert!(md.contains("```python\n# a comment\nx = 1\n```"));
        assert!(md.contains("    # indented code"));
    }

    #[test]
    fn rule_numbering_has_no_gaps_when_empty_rules_exist() {
        let mut p = Prompt::new();
        p.rules = Some(vec![
            Rule { text: "one".into(), examples: vec![] },
            Rule { text: "  ".into(), examples: vec![] },
            Rule { text: "two".into(), examples: vec![] },
        ]);
        let md = p.compile_markdown();
        assert!(md.contains("1. one\n2. two"), "{md}");
    }

    #[test]
    fn multiline_definitions_keep_their_structure() {
        let mut p = Prompt::new();
        p.definitions = Some(vec![Definition {
            term: "RAG".into(),
            definition: "Retrieval augmented generation.\n\nUses:\n- search\n- rank".into(),
        }]);
        let md = p.compile_markdown();
        assert!(md.contains("- **RAG**: Retrieval augmented generation.\n\n  Uses:\n  - search\n  - rank"), "{md}");
    }

    #[test]
    fn flows_export_as_titled_mermaid_blocks() {
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("Plan:".into())];
        let idx = p.insert_flow_after(0);
        if let BodyBlock::Flow(f) = &mut p.body[idx] {
            f.title = "Decision".into();
            f.mermaid = "graph TD\n  A-->B".into();
        }
        let md = p.compile_markdown();
        assert!(md.contains("## Decision\n\n```mermaid\ngraph TD\n  A-->B\n```"), "{md}");
        // inserting a flow always leaves a text block to continue typing in
        assert!(matches!(p.body.last(), Some(BodyBlock::Text(_))));
    }

    #[test]
    fn flow_ids_are_unique() {
        let mut p = Prompt::new();
        let a = p.insert_flow_after(0);
        let b = p.insert_flow_after(a);
        let ids: Vec<_> = p
            .body
            .iter()
            .filter_map(|x| if let BodyBlock::Flow(f) = x { Some(f.id.clone()) } else { None })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        assert!(b > a);
    }

    #[test]
    fn benchmarks_never_leak_into_preview_or_export() {
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("Summarise.".into())];
        p.benchmarks.push(Benchmark {
            expected: None, question: "SECRET-QUESTION".into(),
            answers: vec![
                Answer { model: "model-one".into(), text: "SECRET-ANSWER-1".into() },
                Answer { model: "model-two".into(), text: "SECRET-ANSWER-2".into() },
            ],
        });
        for out in [p.compile_markdown(), p.export_minified()] {
            assert_eq!(out, "Summarise.");
            assert!(!out.contains("SECRET") && !out.contains("model-"));
        }
        // ...but they are persisted
        let f = p.to_file();
        assert!(f.contains("SECRET-QUESTION") && f.contains("@A model-two"));
    }

    #[test]
    fn known_models_are_distinct_and_ordered() {
        let mut p = Prompt::new();
        p.benchmarks = vec![
            Benchmark { expected: None, question: "q1".into(), answers: vec![
                Answer { model: "b".into(), text: "".into() },
                Answer { model: "a".into(), text: "".into() },
            ]},
            Benchmark { expected: None, question: "q2".into(), answers: vec![
                Answer { model: "a".into(), text: "".into() },
                Answer { model: " ".into(), text: "".into() },
                Answer { model: "c".into(), text: "".into() },
            ]},
        ];
        assert_eq!(p.known_models(), vec!["b", "a", "c"]);
    }

    #[test]
    fn full_roundtrip_through_file_format() {
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("Task line 1\n\n  indented\nlast".into())];
        let i = p.insert_flow_after(0);
        if let BodyBlock::Flow(f) = &mut p.body[i] {
            f.title = "Flow: with :: colons".into();
            f.mermaid = "graph LR\n  A --> B".into();
        }
        p.body[i + 1] = BodyBlock::Text("after flow".into());
        p.context = Some("ctx para 1\n\nctx para 2".into());
        p.definitions = Some(vec![
            Definition { term: "alpha".into(), definition: "first\nsecond".into() },
            Definition { term: "beta".into(), definition: "".into() },
        ]);
        p.rules = Some(vec![
            Rule { text: "r1".into(), examples: vec!["e1".into(), "e2\nline2".into()] },
            Rule { text: "r2".into(), examples: vec![] },
        ]);
        p.benchmarks = vec![Benchmark {
            expected: None, question: "What is 2+2?".into(),
            answers: vec![
                Answer { model: "gpt-x".into(), text: "4".into() },
                Answer { model: "claude y".into(), text: "four\nor 4".into() },
            ],
        }];

        let s1 = p.to_file();
        let q = Prompt::from_file(&s1);
        let s2 = q.to_file();
        assert_eq!(s1, s2, "serialize -> parse -> serialize must be stable");
        assert_eq!(q.context.as_deref(), Some("ctx para 1\n\nctx para 2"));
        assert_eq!(q.definitions.as_ref().unwrap().len(), 2);
        assert_eq!(q.rules.as_ref().unwrap()[0].examples[1], "e2\nline2");
        assert_eq!(q.benchmarks[0].answers[1].model, "claude y");
        assert_eq!(q.benchmarks[0].answers[1].text, "four\nor 4");
        assert_eq!(q.compile_markdown(), p.compile_markdown());
        // loaded flows come back collapsed
        assert!(q.body.iter().any(|b| matches!(b, BodyBlock::Flow(f) if f.collapsed && f.title == "Flow: with :: colons")));
    }

    #[test]
    fn marker_lookalike_content_survives_roundtrip() {
        let nasty = "@mention at line start\n@@CONTEXT not a section\n\\ backslash first\n\\@both\n@TEXT";
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text(nasty.into())];
        p.context = Some("@@RULES\n@RULE".into());
        p.benchmarks = vec![Benchmark {
            expected: None, question: "@Q?".into(),
            answers: vec![Answer { model: "m".into(), text: "@A fake\n\\".into() }],
        }];
        let q = Prompt::from_file(&p.to_file());
        assert_eq!(text(&q), nasty);
        assert_eq!(q.context.as_deref(), Some("@@RULES\n@RULE"));
        assert!(q.rules.is_none(), "escaped text must not create sections");
        assert_eq!(q.benchmarks.len(), 1);
        assert_eq!(q.benchmarks[0].question, "@Q?");
        assert_eq!(q.benchmarks[0].answers[0].text, "@A fake\n\\");
    }

    #[test]
    fn legacy_v1_files_still_load() {
        let v1 = "@@CONTEXT\nOld context\n\n@@DEFINITIONS\n@TERM x\nthe x\n\n@@TASK\n@TEXT\nOld task\n@FLOW flow1 :: Old flow\ngraph TD; A-->B\n@ENDFLOW\n@TEXT\ntail\n\n@@RULES\n@RULE\nold rule\n@EXAMPLE\nold example\n";
        let p = Prompt::from_file(v1);
        assert_eq!(p.context.as_deref(), Some("Old context"));
        assert_eq!(p.definitions.as_ref().unwrap()[0].definition, "the x");
        assert_eq!(p.body.len(), 3);
        assert!(matches!(&p.body[1], BodyBlock::Flow(f) if f.mermaid == "graph TD; A-->B"));
        assert_eq!(p.rules.as_ref().unwrap()[0].examples[0], "old example");
    }

    #[test]
    fn plain_text_file_opens_as_a_note() {
        let p = Prompt::from_file("just some words\nacross lines\n");
        assert_eq!(text(&p), "just some words\nacross lines");
        assert!(p.context.is_none());
    }

    #[test]
    fn crlf_and_bom_are_tolerated() {
        let src = "\u{feff}@@AIPROMPT 2\r\n@@BODY\r\n@TEXT\r\nhello\r\nworld\r\n";
        assert_eq!(text(&Prompt::from_file(src)), "hello\nworld");
    }

    #[test]
    fn minify_strips_visual_whitespace_but_not_code_layout() {
        let md = "Line with trailing   \n\n\n\n\tTabbed\t\n\n```make\ntarget:\n\n\n\trecipe  \n```\n\n\n";
        let m = minify_markdown(md);
        assert_eq!(m, "Line with trailing\n\n Tabbed\n\n```make\ntarget:\n\n\n\trecipe\n```");
    }

    #[test]
    fn blank_detection() {
        let mut p = Prompt::new();
        assert!(p.is_blank());
        p.body = vec![BodyBlock::Text("x".into())];
        assert!(!p.is_blank());
        let mut q = Prompt::new();
        q.context = Some(String::new());
        assert!(!q.is_blank(), "an added (even empty) section is structure worth keeping");
        let mut r = Prompt::new();
        r.benchmarks.push(Benchmark::default());
        assert!(!r.is_blank());
    }
}

#[cfg(test)]
mod unicode_tests {
    use super::*;

    const ZH: &str = "你好，世界！这是一个测试。";
    const AR: &str = "مرحبا بالعالم، هذا اختبار";
    const FA: &str = "سلام دنیا، این یک آزمایش است — پ چ ژ گ";
    const MIX: &str = "Use 「引号」 and العربية 🙂 together ｜ 全角　空格";

    fn rich() -> Prompt {
        let mut p = Prompt::new();
        p.context = Some(format!("{ZH}\n\n{AR}"));
        p.definitions = Some(vec![Definition {
            term: "术语 / مصطلح".into(),
            definition: format!("{FA}\n\n{MIX}"),
        }]);
        p.rules = Some(vec![Rule {
            text: format!("规则：{AR}"),
            examples: vec![format!("例：{ZH}"), FA.into()],
        }]);
        p.body = vec![
            BodyBlock::Text(format!("{ZH}\n@不是标记\n\\反斜杠 {AR}")),
            BodyBlock::Flow(Flow {
                id: "flow1".into(),
                title: "流程 — جريان".into(),
                mermaid: "graph LR\n  A[开始] --> B[مرحبا]".into(),
                collapsed: true,
            }),
            BodyBlock::Text(MIX.into()),
        ];
        p.benchmarks = vec![Benchmark {
            expected: None, question: format!("{ZH} {AR}"),
            answers: vec![
                Answer { model: "模型 A / مدل".into(), text: FA.into() },
                Answer { model: "GPT-5".into(), text: MIX.into() },
            ],
        }];
        p
    }

    #[test]
    fn chinese_arabic_persian_roundtrip_through_the_file_format() {
        let p = rich();
        let text = p.to_file();
        let q = Prompt::from_file(&text);
        assert_eq!(q.to_file(), text);
        assert_eq!(q.context, p.context);
        assert_eq!(q.definitions.as_ref().unwrap()[0].term, "术语 / مصطلح");
        assert_eq!(q.rules.as_ref().unwrap()[0].examples[1], FA);
        assert_eq!(q.benchmarks[0].answers[0].model, "模型 A / مدل");
        assert_eq!(q.benchmarks[0].answers[1].text, MIX);
        match &q.body[0] {
            BodyBlock::Text(t) => assert!(t.contains("@不是标记") && t.contains("\\反斜杠"), "{t}"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn utf8_bytes_on_disk_are_valid_and_crlf_files_load() {
        let text = rich().to_file();
        assert!(std::str::from_utf8(text.as_bytes()).is_ok());
        let crlf = text.replace('\n', "\r\n");
        let q = Prompt::from_file(&crlf);
        assert_eq!(q.to_file(), text);
    }

    #[test]
    fn export_keeps_every_character_and_never_leaks_benchmarks() {
        let p = rich();
        let out = p.export_minified();
        for needle in [ZH, AR, FA, "术语 / مصطلح", "规则：", "流程 — جريان", "A[开始] --> B[مرحبا]"] {
            assert!(out.contains(needle), "missing {needle:?} in:\n{out}");
        }
        assert!(!out.contains("模型 A"), "benchmark data leaked into the export");
        // mermaid stays plain notation in the export
        assert!(out.contains("```mermaid\ngraph LR\n  A[开始] --> B[مرحبا]\n```"));
    }

    #[test]
    fn ideographic_space_does_not_break_header_demotion() {
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("\u{3000}# 标题\n# 一级标题\n正文".into())];
        let md = p.compile_markdown();
        assert!(md.contains("## 一级标题"), "{md}");
    }

    #[test]
    fn token_estimate_is_not_fooled_by_cjk() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd efgh"), 3);
        // 10 Chinese characters are ~10 tokens, not ~3
        assert_eq!(estimate_tokens("你好你好你好你好你好"), 10);
        assert!(estimate_tokens(AR) >= AR.chars().count() / 3);
    }

    #[test]
    fn expected_answer_round_trips_and_stays_private() {
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("Task text".into())];
        p.benchmarks = vec![
            Benchmark {
                question: "Q one".into(),
                expected: Some("must mention refunds\n@EXPECTED tricky".into()),
                answers: vec![Answer { model: "m".into(), text: "ans".into() }],
            },
            Benchmark { question: "Q two".into(), expected: None, answers: vec![] },
            Benchmark { question: "Q three".into(), expected: Some("  ".into()), answers: vec![] },
        ];
        let f = p.to_file();
        let q = Prompt::from_file(&f);
        assert_eq!(q.benchmarks[0].expected.as_deref(), Some("must mention refunds\n@EXPECTED tricky"));
        assert_eq!(q.benchmarks[0].answers[0].text, "ans");
        assert_eq!(q.benchmarks[1].expected, None);
        assert_eq!(q.benchmarks[2].expected, None, "blank expected answers are not written");
        assert!(!p.compile_markdown().contains("refunds") && !p.export_minified().contains("refunds"));
        // files from before this feature still load
        let old = Prompt::from_file("@@AIPROMPT 2\n@@BENCHMARKS\n@Q\nold q\n@A m\nold a\n");
        assert_eq!(old.benchmarks[0].expected, None);
        assert_eq!(old.benchmarks[0].answers[0].text, "old a");
    }

    #[test]
    fn code_block_round_trips_and_is_fenced_in_the_main_section() {
        let mut p = Prompt::new();
        p.body = vec![
            BodyBlock::Text("Review this:".into()),
            BodyBlock::Code(CodeBlock {
                id: "code1".into(),
                title: "main.py".into(),
                code: "def f():\n    # not a header\n    return 1\n\n@decorator\n\\x".into(),
                collapsed: false,
            }),
            BodyBlock::Text("Thanks.".into()),
        ];
        let q = Prompt::from_file(&p.to_file());
        let BodyBlock::Code(c) = &q.body[1] else { panic!("code block lost") };
        assert_eq!((c.id.as_str(), c.title.as_str()), ("code1", "main.py"));
        assert_eq!(c.code, "def f():\n    # not a header\n    return 1\n\n@decorator\n\\x");
        assert!(c.collapsed, "loaded code blocks start collapsed");
        let out = p.export_minified();
        assert_eq!(
            out,
            "Review this:\n\n**main.py**\n\n```\ndef f():\n    # not a header\n    return 1\n\n@decorator\n\\x\n```\n\nThanks."
        );
    }

    #[test]
    fn code_fence_grows_around_backticks_and_is_not_doubled() {
        assert_eq!(fenced("a\n```\nb"), "````\na\n```\nb\n````");
        assert_eq!(fenced("```rust\nlet x = 1;\n```"), "```\nlet x = 1;\n```");
        // a longer fence survives minify and demote untouched
        let md = format!("# T\n\n{}\n\n", fenced("x\n```\n\n\n# not a header  "));
        let min = minify_markdown(&md);
        assert!(min.contains("x\n```\n\n\n# not a header\n````"), "{min:?}");
    }

    #[test]
    fn scans_variable_tags() {
        let v = |t: &str| scan_variables(t).into_iter().map(|(_, n)| n).collect::<Vec<_>>();
        assert_eq!(v("a {{x}} b {{ dir_structure }} {{codechunk_1}}"), ["x", "dir_structure", "codechunk_1"]);
        assert!(v("{{}} {{ }} {{two words}} {{a\nb}} {x} {{open").is_empty());
        assert_eq!(v("{{{x}}}"), ["x"]);
        assert_eq!(v("{{مرحبا}} {{文件-1.2}}"), ["مرحبا", "文件-1.2"]);
    }

    #[test]
    fn variables_are_substituted_on_export_but_not_in_flows_or_code_or_benchmarks() {
        let mut p = Prompt::new();
        p.context = Some("Project {{name}}.".into());
        p.body = vec![
            BodyBlock::Text("See {{tree}} and {{name}}; {{missing}} stays.".into()),
            BodyBlock::Flow(Flow { id: "flow1".into(), title: "f".into(), mermaid: "graph TD\n A{{hex}}-->B".into(), collapsed: true }),
            BodyBlock::Code(CodeBlock { id: "code1".into(), title: String::new(), code: "{{tpl}}".into(), collapsed: true }),
        ];
        p.rules = Some(vec![Rule { text: "Use {{name}}".into(), examples: vec!["{{name}}!".into()] }]);
        p.benchmarks = vec![Benchmark { question: "{{private}}".into(), ..Default::default() }];
        *p.variable_value_mut("name") = "Atlas".into();
        *p.variable_value_mut("tree") = "src/\n  main.rs\n{{name}}".into(); // not re-expanded
        assert_eq!(p.used_variables(), ["name", "tree", "missing"]);
        assert_eq!(p.unaddressed_variables(), ["missing"]);
        let out = p.export_minified();
        assert!(out.contains("Project Atlas."), "{out}");
        assert!(out.contains("See src/\n  main.rs\n{{name}} and Atlas; {{missing}} stays."), "{out}");
        assert!(out.contains("A{{hex}}-->B") && out.contains("```\n{{tpl}}\n```"));
        assert!(out.contains("1. Use Atlas\n   - Example: Atlas!"));
        assert!(!p.used_variables().contains(&"private".to_string()));
        // the preview source keeps the tags
        assert!(p.compile_markdown().contains("Project {{name}}."));
    }

    #[test]
    fn variable_values_round_trip_and_blank_ones_are_not_stored() {
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("{{a}} {{b}} {{c}}".into())];
        *p.variable_value_mut("a") = "line1\n@line2\n\n  indented".into();
        *p.variable_value_mut("b") = "   ".into();
        let f = p.to_file();
        assert!(!f.contains("@VAR b"));
        let q = Prompt::from_file(&f);
        assert_eq!(q.variable_value("a"), Some("line1\n@line2\n\n  indented"));
        assert_eq!(q.unaddressed_variables(), ["b", "c"]);
    }

    #[test]
    fn tags_inside_code_are_literal() {
        let v = |t: &str| scan_prose_variables(t).into_iter().map(|(_, n)| n).collect::<Vec<_>>();
        assert_eq!(v("use `{{a}}` and {{b}} and ``x {{c}} y`` {{d}}"), ["b", "d"]);
        assert_eq!(v("```\n{{in_fence}}\n```\n{{out}}"), ["out"]);
        assert_eq!(v("lonely ` {{a}}"), ["a"]);
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("Docs: `{{a}}` but {{b}}".into())];
        *p.variable_value_mut("b") = "B".into();
        assert!(p.unaddressed_variables().is_empty());
        assert_eq!(p.export_minified(), "Docs: `{{a}}` but B");
    }
}
