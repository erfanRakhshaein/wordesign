//! Find & replace engine (plain text or regex). Pure functions, no UI.

use regex::{Regex, RegexBuilder};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub regex: bool,
    pub case_sensitive: bool,
}

impl Default for Query {
    fn default() -> Self {
        Self { text: String::new(), regex: false, case_sensitive: false }
    }
}

/// Compile the query. `Ok(None)` for an empty query, `Err` with a short message for an invalid regex.
pub fn compile(q: &Query) -> Result<Option<Regex>, String> {
    if q.text.is_empty() {
        return Ok(None);
    }
    let pattern = if q.regex { q.text.clone() } else { regex::escape(&q.text) };
    RegexBuilder::new(&pattern)
        .case_insensitive(!q.case_sensitive)
        .multi_line(true) // ^ and $ match at every line
        .build()
        .map(Some)
        .map_err(|e| {
            let msg = e.to_string();
            // regex errors are multi-line; the last line holds the actual reason
            msg.lines().last().unwrap_or("invalid regex").trim_start_matches("error: ").to_owned()
        })
}

/// Non-empty matches as char ranges (start, end).
pub fn find_all(re: &Regex, text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut chars_before = 0usize;
    let mut last_byte = 0usize;
    for m in re.find_iter(text) {
        if m.start() == m.end() {
            continue;
        }
        chars_before += text[last_byte..m.start()].chars().count();
        let len = text[m.start()..m.end()].chars().count();
        out.push((chars_before, chars_before + len));
        chars_before += len;
        last_byte = m.end();
    }
    out
}

/// Turn `\n`, `\t`, `\r` and `\\` in a replacement into the real characters (regex mode only).
/// Everything else, including `$1`, is kept for the regex engine.
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// What replacing one match produces (capture groups expanded in regex mode).
fn expand(q: &Query, re: &Regex, matched: &str, replacement: &str) -> String {
    if q.regex {
        re.replace(matched, unescape(replacement).as_str()).into_owned()
    } else {
        replacement.to_owned()
    }
}

/// Replace every match; returns how many were replaced.
pub fn replace_all(q: &Query, re: &Regex, text: &mut String, replacement: &str) -> usize {
    let n = find_all(re, text).len();
    if n == 0 {
        return 0;
    }
    let out = if q.regex {
        let rep = unescape(replacement);
        re.replace_all(text, rep.as_str()).into_owned()
    } else {
        re.replace_all(text, regex::NoExpand(replacement)).into_owned()
    };
    *text = out;
    n
}

/// Replace the single match at char range `(start, end)`. Returns the char length of the inserted text.
pub fn replace_at(q: &Query, re: &Regex, text: &mut String, range: (usize, usize), replacement: &str) -> usize {
    let byte = |ci: usize| text.char_indices().nth(ci).map_or(text.len(), |(b, _)| b);
    let (b0, b1) = (byte(range.0), byte(range.1));
    // Re-run the regex anchored on the slice so `$1` refers to this match's groups
    // (lookaround-free patterns only, which is all the regex crate supports).
    let new = expand(q, re, &text[b0..b1], replacement);
    let n = new.chars().count();
    text.replace_range(b0..b1, &new);
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(t: &str, regex: bool, cs: bool) -> Query {
        Query { text: t.into(), regex, case_sensitive: cs }
    }

    #[test]
    fn plain_search_is_literal_and_case_insensitive_by_default() {
        let query = q("a.b", false, false);
        let re = compile(&query).unwrap().unwrap();
        assert_eq!(find_all(&re, "A.B axb a.b"), vec![(0, 3), (8, 11)]);
        let re = compile(&q("a.b", false, true)).unwrap().unwrap();
        assert_eq!(find_all(&re, "A.B a.b"), vec![(4, 7)]);
    }

    #[test]
    fn match_positions_are_char_indices_for_unicode() {
        let re = compile(&q("好", false, false)).unwrap().unwrap();
        assert_eq!(find_all(&re, "你好, مرحبا 好!"), vec![(1, 2), (10, 11)]);
    }

    #[test]
    fn comma_to_newline_replacement_works_in_regex_mode() {
        let query = q(",\\s*", true, false);
        let re = compile(&query).unwrap().unwrap();
        let mut t = "red, green,blue".to_owned();
        assert_eq!(replace_all(&query, &re, &mut t, "\\n"), 2);
        assert_eq!(t, "red\ngreen\nblue");
    }

    #[test]
    fn capture_groups_and_anchors() {
        let query = q("^(\\w+): (\\w+)$", true, false);
        let re = compile(&query).unwrap().unwrap();
        let mut t = "a: b\nc: d\nnot this".to_owned();
        assert_eq!(replace_all(&query, &re, &mut t, "$2=$1"), 2);
        assert_eq!(t, "b=a\nd=c\nnot this");
    }

    #[test]
    fn plain_replacement_is_literal() {
        let query = q("x", false, false);
        let re = compile(&query).unwrap().unwrap();
        let mut t = "x-X".to_owned();
        assert_eq!(replace_all(&query, &re, &mut t, "$1\\n"), 2);
        assert_eq!(t, "$1\\n-$1\\n");
    }

    #[test]
    fn replace_one_match_and_report_length() {
        let query = q("(\\d+)", true, false);
        let re = compile(&query).unwrap().unwrap();
        let mut t = "a 12 b 345".to_owned();
        let m = find_all(&re, &t);
        let added = replace_at(&query, &re, &mut t, m[1], "<$1>");
        assert_eq!((t.as_str(), added), ("a 12 b <345>", 5));
    }

    #[test]
    fn invalid_and_empty_queries() {
        assert!(compile(&q("", true, false)).unwrap().is_none());
        assert!(compile(&q("(", true, false)).is_err());
        let re = compile(&q("x*", true, false)).unwrap().unwrap();
        assert!(find_all(&re, "abc").is_empty(), "empty matches are ignored");
    }

    #[test]
    fn unescape_only_known_escapes() {
        assert_eq!(unescape("a\\nb\\tc\\\\d\\q$1"), "a\nb\tc\\d\\q$1");
    }
}
