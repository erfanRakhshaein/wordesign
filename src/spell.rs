//! Spell checking (bundled en_US Hunspell dictionary via `spellbook`) and
//! recognition of the terms from the Definitions section.
//!
//! Rules of thumb, so the checker stays quiet about things that are not prose:
//! * only Latin-script words are checked; other scripts are never flagged
//! * words with digits, ALL-CAPS acronyms, very short words, URLs, `{{placeholders}}`,
//!   `<tags>`, inline code and fenced code are ignored
//! * hyphen / underscore / camelCase compounds pass if every part is a word
//! * a word the dictionary does not know that matches a defined term is *defined*, not misspelled

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;

use crate::model::{BodyBlock, Prompt};

const AFF: &str = include_str!("../assets/dict/en_US.index.aff");
const DIC: &str = include_str!("../assets/dict/en_US.index.dic");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Misspelled,
    /// Index into `Prompt::definitions`.
    Defined(usize),
    /// A `{{variable}}` placeholder.
    Variable,
}

struct Term {
    /// Index into the prompt's definitions list.
    index: usize,
    lower: Vec<char>,
    text: String,
    definition: String,
    /// Scripts written without spaces (CJK): match anywhere, no word boundaries.
    boundaryless: bool,
}

pub struct Checker {
    dict: spellbook::Dictionary,
    personal: HashSet<String>,
    personal_path: Option<PathBuf>,
    terms: Vec<Term>,
    accepted: RefCell<HashMap<String, bool>>,
    suggestions: RefCell<HashMap<String, Vec<String>>>,
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x20000..=0x3FFFF)
}

fn is_latin_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c as u32, 0x00C0..=0x024F | 0x1E00..=0x1EFF) && c != '×' && c != '÷'
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn lower_char(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

fn is_all_caps(s: &str) -> bool {
    let mut letters = s.chars().filter(|c| c.is_alphabetic()).peekable();
    letters.peek().is_some() && letters.all(|c| c.is_uppercase())
}

/// Byte ranges of the word-like tokens in `text`.
pub fn tokens(text: &str) -> Vec<Range<usize>> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !is_word_char(chars[i].1) {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + 1;
        while j < chars.len() {
            let c = chars[j].1;
            let joiner = matches!(c, '\'' | '’' | '-')
                && j + 1 < chars.len()
                && chars[j + 1].1.is_alphanumeric()
                && is_word_char(chars[j - 1].1);
            if is_word_char(c) || joiner {
                j += 1;
            } else {
                break;
            }
        }
        let byte_start = chars[start].0;
        let byte_end = chars.get(j).map_or(text.len(), |(b, _)| *b);
        out.push(byte_start..byte_end);
        i = j;
    }
    out
}

/// Tokens that are part of a URL, e-mail, placeholder or tag are not prose.
fn looks_technical(text: &str, range: &Range<usize>) -> bool {
    let before = &text[..range.start];
    let after = &text[range.end..];
    let prev = before.chars().next_back();
    if matches!(prev, Some('<' | '{' | '$' | '%' | '@' | '/' | '\\' | '#' | '=')) {
        return true;
    }
    if matches!(prev, Some('.')) && before.chars().rev().nth(1).is_some_and(|c| c.is_alphanumeric()) {
        return true; // file extension / domain part
    }
    let next = after.chars().next();
    if next == Some('@') || after.starts_with("://") {
        return true;
    }
    if matches!(next, Some('.')) && after.chars().nth(1).is_some_and(|c| c.is_alphanumeric()) {
        return true; // domain.tld, file.ext
    }
    false
}

/// Segments of one markdown source line that are ordinary prose
/// (everything except `inline code` and URLs).
fn prose_segments(line: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut seg_start = 0;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let ticks = bytes[i..].iter().take_while(|b| **b == b'`').count();
            let fence = "`".repeat(ticks);
            if let Some(close) = line[i + ticks..].find(&fence) {
                if seg_start < i {
                    out.push(seg_start..i);
                }
                i += ticks + close + ticks;
                seg_start = i;
                continue;
            }
            i += ticks;
            continue;
        }
        if line[i..].starts_with("](") {
            if let Some(close) = line[i..].find(')') {
                if seg_start < i + 1 {
                    out.push(seg_start..i + 1);
                }
                i += close + 1;
                seg_start = i;
                continue;
            }
        }
        if line[i..].starts_with("http://") || line[i..].starts_with("https://") {
            if seg_start < i {
                out.push(seg_start..i);
            }
            let end = line[i..].find(char::is_whitespace).map_or(line.len(), |e| i + e);
            i = end;
            seg_start = i;
            continue;
        }
        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }
    if seg_start < line.len() {
        out.push(seg_start..line.len());
    }
    out
}

impl Checker {
    pub fn new(personal_path: Option<PathBuf>) -> Self {
        let dict = spellbook::Dictionary::new(AFF, DIC).expect("bundled dictionary is valid");
        let personal = personal_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| s.lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect())
            .unwrap_or_default();
        Self {
            dict,
            personal,
            personal_path,
            terms: Vec::new(),
            accepted: RefCell::new(HashMap::new()),
            suggestions: RefCell::new(HashMap::new()),
        }
    }

    /// Does the dictionary (or the personal word list) know this word?
    pub fn accepts(&self, word: &str) -> bool {
        if let Some(v) = self.accepted.borrow().get(word) {
            return *v;
        }
        let norm = word.replace('’', "'");
        let ok = self.dict.check(&norm)
            || self.personal.contains(&norm.to_lowercase())
            || norm.strip_suffix("'s").is_some_and(|stem| self.dict.check(stem) || self.personal.contains(&stem.to_lowercase()));
        self.accepted.borrow_mut().insert(word.to_owned(), ok);
        ok
    }

    /// Add a word to the personal dictionary (saved next to the prompts).
    pub fn add_personal(&mut self, word: &str) {
        let w = word.trim().to_lowercase();
        if w.is_empty() || !self.personal.insert(w.clone()) {
            return;
        }
        self.accepted.borrow_mut().clear();
        if let Some(path) = &self.personal_path {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{w}");
            }
        }
    }

    /// Provide the Definitions section: `(index in the prompt, term, definition)`.
    ///
    /// Only terms the dictionary does not recognise get highlighted ("unrecognized words are
    /// looked up in the definitions").
    pub fn set_definitions(&mut self, defs: &[(usize, String, String)]) {
        let unchanged = self.terms.len()
            == defs
                .iter()
                .filter(|(_, t, _)| !t.trim().is_empty())
                .filter(|(_, t, _)| self.term_is_unrecognized(t.trim()))
                .count()
            && self.terms.iter().all(|t| defs.iter().any(|(i, term, d)| *i == t.index && term.trim() == t.text && d == &t.definition));
        if unchanged {
            return;
        }
        let mut terms: Vec<Term> = defs
            .iter()
            .filter(|(_, t, _)| !t.trim().is_empty())
            .filter(|(_, t, _)| self.term_is_unrecognized(t.trim()))
            .map(|(i, t, d)| Term {
                index: *i,
                lower: t.trim().chars().map(lower_char).collect(),
                text: t.trim().to_owned(),
                definition: d.clone(),
                boundaryless: t.chars().any(is_cjk),
            })
            .collect();
        terms.sort_by_key(|t| std::cmp::Reverse(t.lower.len()));
        self.terms = terms;
    }

    fn term_is_unrecognized(&self, term: &str) -> bool {
        let words: Vec<Range<usize>> = tokens(term);
        let all_known_latin = !words.is_empty()
            && words.iter().all(|r| {
                let w = &term[r.clone()];
                w.chars().all(|c| is_latin_letter(c) || !c.is_alphabetic()) && !is_all_caps(w) && self.accepts(w)
            });
        !all_known_latin
    }

    /// Definition text of the term at `index` in the prompt.
    pub fn definition_of(&self, index: usize) -> Option<(&str, &str)> {
        self.terms
            .iter()
            .find(|t| t.index == index)
            .map(|t| (t.text.as_str(), t.definition.as_str()))
    }

    /// Would this token be reported as misspelled (ignoring defined terms)?
    fn is_flagged_word(&self, token: &str) -> bool {
        let n = token.chars().count();
        if n < 3 {
            return false;
        }
        let mut letters = false;
        for c in token.chars() {
            if c.is_numeric() {
                return false;
            }
            if c.is_alphabetic() {
                letters = true;
                if !is_latin_letter(c) {
                    return false; // other scripts: no dictionary
                }
            }
        }
        if !letters || is_all_caps(token) {
            return false;
        }
        // "APIs", "URLs"
        if token.strip_suffix('s').is_some_and(|stem| stem.chars().count() >= 2 && is_all_caps(stem)) {
            return false;
        }
        if self.accepts(token) {
            return false;
        }
        // compounds: foo-bar, foo_bar, fooBar
        let parts = split_compound(token);
        if parts.len() > 1 {
            return parts.iter().any(|p| self.is_flagged_part(p));
        }
        true
    }

    fn is_flagged_part(&self, part: &str) -> bool {
        part.chars().count() >= 3
            && !is_all_caps(part)
            && !part.chars().any(char::is_numeric)
            && !self.accepts(part)
            && !self.accepts(&capitalise(part))
    }

    /// Everything noteworthy in a piece of prose: defined terms and misspelled words.
    pub fn annotate(&self, text: &str) -> Vec<(Range<usize>, Verdict)> {
        let mut out: Vec<(Range<usize>, Verdict)> = crate::model::scan_variables(text)
            .into_iter()
            .map(|(r, _)| (r, Verdict::Variable))
            .collect();
        for d in self.defined_ranges(text) {
            if !out.iter().any(|(r, _)| r.start < d.0.end && d.0.start < r.end) {
                out.push(d);
            }
        }
        for r in tokens(text) {
            if out.iter().any(|(d, _)| d.start < r.end && r.start < d.end) {
                continue;
            }
            if looks_technical(text, &r) {
                continue;
            }
            if self.is_flagged_word(&text[r.clone()]) {
                out.push((r, Verdict::Misspelled));
            }
        }
        out.sort_by_key(|(r, _)| r.start);
        out
    }

    fn defined_ranges(&self, text: &str) -> Vec<(Range<usize>, Verdict)> {
        if self.terms.is_empty() {
            return Vec::new();
        }
        let chars: Vec<(usize, char)> = text.char_indices().map(|(b, c)| (b, lower_char(c))).collect();
        let byte_at = |i: usize| chars.get(i).map_or(text.len(), |(b, _)| *b);
        let mut out: Vec<(Range<usize>, Verdict)> = Vec::new();
        for term in &self.terms {
            let n = term.lower.len();
            let mut i = 0;
            while i + n <= chars.len() {
                if chars[i..i + n].iter().map(|(_, c)| *c).eq(term.lower.iter().copied()) {
                    let mut end = i + n;
                    let ok_before = term.boundaryless || i == 0 || !is_word_char(chars[i - 1].1);
                    // plural / possessive: "RAGs", "RAG's"
                    if !term.boundaryless {
                        if chars.get(end).is_some_and(|(_, c)| *c == 's')
                            && !chars.get(end + 1).is_some_and(|(_, c)| is_word_char(*c))
                        {
                            end += 1;
                        } else if chars.get(end).is_some_and(|(_, c)| matches!(*c, '\'' | '’'))
                            && chars.get(end + 1).is_some_and(|(_, c)| *c == 's')
                            && !chars.get(end + 2).is_some_and(|(_, c)| is_word_char(*c))
                        {
                            end += 2;
                        }
                    }
                    let ok_after = term.boundaryless || !chars.get(end).is_some_and(|(_, c)| is_word_char(*c));
                    let range = byte_at(i)..byte_at(end);
                    if ok_before && ok_after && !out.iter().any(|(r, _)| r.start < range.end && range.start < r.end) {
                        out.push((range, Verdict::Defined(term.index)));
                        i = end;
                        continue;
                    }
                }
                i += 1;
            }
        }
        out
    }

    /// Spelling suggestions, best first.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        if let Some(s) = self.suggestions.borrow().get(word) {
            return s.clone();
        }
        let mut out = Vec::new();
        self.dict.suggest(&word.replace('’', "'"), &mut out);
        out.retain(|s| s != word);
        // Closest first (Damerau-Levenshtein), then the longest shared beginning; further ties keep the dictionary's order.
        let lw: Vec<char> = word.to_lowercase().chars().collect();
        out.sort_by_key(|s| {
            let sl: Vec<char> = s.to_lowercase().chars().collect();
            let prefix = lw.iter().zip(&sl).take_while(|(a, b)| a == b).count().min(lw.len().saturating_sub(1));
            (edit_distance(&lw, &sl), std::cmp::Reverse(prefix))
        });
        out.truncate(6);
        self.suggestions.borrow_mut().insert(word.to_owned(), out.clone());
        out
    }

    // -----------------------------------------------------------------
    // editing the source markdown
    // -----------------------------------------------------------------

    /// Rewrite the misspelled words of a markdown source text. `fix` gets the word and returns a
    /// replacement (or `None` to leave it). Code blocks, inline code and URLs are never touched.
    pub fn rewrite_text(&self, text: &str, fix: &mut dyn FnMut(&str) -> Option<String>) -> String {
        let mut out = String::with_capacity(text.len());
        let mut fence: Option<(char, usize)> = None;
        for (n, line) in text.split('\n').enumerate() {
            if n > 0 {
                out.push('\n');
            }
            let t = line.trim_start_matches(' ');
            let marker = t.chars().next().filter(|c| *c == '`' || *c == '~');
            let run = marker.map_or(0, |m| t.chars().take_while(|c| *c == m).count());
            if let (Some(m), true) = (marker, run >= 3 && line.len() - t.len() <= 3) {
                match fence {
                    None => fence = Some((m, run)),
                    Some((fm, fl)) if fm == m && run >= fl && t[run..].trim().is_empty() => fence = None,
                    _ => {}
                }
                out.push_str(line);
                continue;
            }
            if fence.is_some() {
                out.push_str(line);
                continue;
            }
            let mut last = 0;
            for seg in prose_segments(line) {
                let piece = &line[seg.clone()];
                out.push_str(&line[last..seg.start]);
                let mut at = 0;
                for (range, verdict) in self.annotate(piece) {
                    if verdict != Verdict::Misspelled {
                        continue;
                    }
                    if let Some(rep) = fix(&piece[range.clone()]) {
                        out.push_str(&piece[at..range.start]);
                        out.push_str(&rep);
                        at = range.end;
                    }
                }
                out.push_str(&piece[at..]);
                last = seg.end;
            }
            out.push_str(&line[last..]);
        }
        out
    }

    /// Apply `fix` to every prose field of the prompt (never to terms, flows or benchmarks).
    pub fn rewrite_prompt(&self, prompt: &mut Prompt, fix: &mut dyn FnMut(&str) -> Option<String>) {
        for block in &mut prompt.body {
            if let BodyBlock::Text(t) = block {
                *t = self.rewrite_text(t, fix);
            }
        }
        if let Some(c) = prompt.context.as_mut() {
            *c = self.rewrite_text(c, fix);
        }
        for d in prompt.definitions.iter_mut().flatten() {
            d.definition = self.rewrite_text(&d.definition, fix);
        }
        for r in prompt.rules.iter_mut().flatten() {
            r.text = self.rewrite_text(&r.text, fix);
            for e in &mut r.examples {
                *e = self.rewrite_text(e, fix);
            }
        }
    }

    /// Number of misspelled words in the prompt.
    pub fn count_misspelled(&self, prompt: &Prompt) -> usize {
        let mut n = 0;
        let mut copy = prompt.clone();
        self.rewrite_prompt(&mut copy, &mut |_| {
            n += 1;
            None
        });
        n
    }

    /// Replace each misspelled word by the dictionary's best suggestion. Returns how many were fixed.
    pub fn fix_all(&self, prompt: &mut Prompt) -> usize {
        let mut fixed = 0;
        self.rewrite_prompt(prompt, &mut |word| {
            let best = self.suggest(word).into_iter().next()?;
            fixed += 1;
            Some(best)
        });
        fixed
    }

    /// Replace every occurrence of one misspelled word.
    pub fn replace_word(&self, prompt: &mut Prompt, from: &str, to: &str) -> usize {
        let mut n = 0;
        self.rewrite_prompt(prompt, &mut |word| {
            (word == from).then(|| {
                n += 1;
                to.to_owned()
            })
        });
        n
    }
}

/// Optimal string alignment distance (insert / delete / substitute / swap neighbours).
fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in 0..=a.len() {
        d[i][0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// `foo-bar_baz`, `fooBar` → parts.
fn split_compound(token: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    for c in token.chars() {
        if c == '-' || c == '_' {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
        } else {
            if prev.is_some_and(char::is_lowercase) && c.is_uppercase() && !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        }
        prev = Some(c);
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker() -> Checker {
        Checker::new(None)
    }

    fn flagged(c: &Checker, text: &str) -> Vec<String> {
        c.annotate(text)
            .into_iter()
            .filter(|(_, v)| *v == Verdict::Misspelled)
            .map(|(r, _)| text[r].to_owned())
            .collect()
    }

    #[test]
    fn flags_real_typos_only() {
        let c = checker();
        assert_eq!(flagged(&c, "Answer the custmer politely and recieve feedback."), vec!["custmer", "recieve"]);
        assert!(flagged(&c, "The quick brown fox, don't panic. It's fine.").is_empty());
    }

    #[test]
    fn ignores_non_prose() {
        let c = checker();
        assert!(flagged(&c, "see https://exmple.com/pth and mail me@exmple.com").is_empty());
        assert!(flagged(&c, "use {{custmer_name}} and <thinkng> tags, API, JSON, GPT4, APIs").is_empty());
        assert!(flagged(&c, "你好世界 مرحبا שלום").is_empty());
    }

    #[test]
    fn compounds_and_camel_case() {
        let c = checker();
        assert!(flagged(&c, "state-of-the-art snake_case camelCase").is_empty());
        assert_eq!(flagged(&c, "well-knwon"), vec!["well-knwon"]);
    }

    #[test]
    fn defined_terms_are_not_misspellings() {
        let mut c = checker();
        c.set_definitions(&[
            (0, "wordesign".into(), "the app".into()),
            (1, "RAG".into(), "retrieval".into()),
            (2, "context".into(), "ordinary word, stays plain".into()),
            (3, "向量".into(), "vector".into()),
            (4, "مصطلح".into(), "term".into()),
        ]);
        let text = "Use Wordesign and RAG's output with 向量数据库 and مصطلح here, plus wordesigns and the context.";
        let ann = c.annotate(text);
        let defined: Vec<(&str, usize)> = ann
            .iter()
            .filter_map(|(r, v)| match v {
                Verdict::Defined(i) => Some((&text[r.clone()], *i)),
                _ => None,
            })
            .collect();
        assert_eq!(defined, vec![("Wordesign", 0), ("RAG's", 1), ("向量", 3), ("مصطلح", 4), ("wordesigns", 0)]);
        assert!(flagged(&c, text).is_empty());
        assert_eq!(c.definition_of(1), Some(("RAG", "retrieval")));
        assert!(c.definition_of(2).is_none(), "recognised words are not highlighted");
    }

    #[test]
    fn suggestions_work() {
        let c = checker();
        assert_eq!(c.suggest("custmer").first().map(String::as_str), Some("customer"));
        assert!(c.suggest("recieve").contains(&"receive".to_owned()));
        assert_eq!(c.suggest("bulet").first().map(String::as_str), Some("bullet"));
    }

    #[test]
    fn fix_all_edits_prose_but_not_code() {
        let c = checker();
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text(
            "Be polite to the custmer.\n```\nprint(custmer)\n```\nUse `custmer` literally; recieve it.".into(),
        )];
        p.context = Some("A helpfull bot".into());
        assert_eq!(c.count_misspelled(&p), 3);
        let fixed = c.fix_all(&mut p);
        assert_eq!(fixed, 3);
        let BodyBlock::Text(t) = &p.body[0] else { panic!() };
        assert_eq!(t, "Be polite to the customer.\n```\nprint(custmer)\n```\nUse `custmer` literally; receive it.");
        assert_eq!(p.context.as_deref(), Some("A helpful bot"));
        assert_eq!(c.count_misspelled(&p), 0);
    }

    #[test]
    fn replace_single_word_everywhere() {
        let c = checker();
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("custmer and custmer. recieve".into())];
        assert_eq!(c.replace_word(&mut p, "custmer", "customer"), 2);
        let BodyBlock::Text(t) = &p.body[0] else { panic!() };
        assert_eq!(t, "customer and customer. recieve");
    }

    #[test]
    fn personal_dictionary_is_remembered() {
        let dir = std::env::temp_dir().join(format!("wd-spell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".personal_dictionary.txt");
        let _ = std::fs::remove_file(&path);
        let mut c = Checker::new(Some(path.clone()));
        assert!(!c.accepts("Zorblax"));
        c.add_personal("Zorblax");
        assert!(c.accepts("zorblax") && c.accepts("Zorblax"));
        let again = Checker::new(Some(path));
        assert!(again.accepts("Zorblax"));
    }

    #[test]
    fn variables_are_annotated_and_never_misspelled() {
        let mut c = checker();
        c.set_definitions(&[(0, "RAG".into(), "retrieval".into())]);
        let text = "Use {{codechunk_1}} and {{xyzzyq}} with RAG.";
        let a = c.annotate(text);
        let vars: Vec<&str> = a.iter().filter(|(_, v)| *v == Verdict::Variable).map(|(r, _)| &text[r.clone()]).collect();
        assert_eq!(vars, ["{{codechunk_1}}", "{{xyzzyq}}"]);
        assert!(a.iter().all(|(_, v)| *v != Verdict::Misspelled));
        assert!(a.iter().any(|(_, v)| matches!(v, Verdict::Defined(0))));
    }
}
