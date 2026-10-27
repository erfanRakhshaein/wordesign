//! Splits compiled markdown into plain-markdown pieces and ```mermaid blocks, so
//! the preview can draw the latter as real flowcharts. Only the preview does
//! this; Export always keeps the mermaid notation untouched.

#[derive(Debug, PartialEq, Eq)]
pub enum Segment {
    Markdown(String),
    Mermaid(String),
}

/// `(fence char, fence length, info string)` if `line` opens/closes a code fence.
fn fence(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let t = &line[indent..];
    let ch = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = t.chars().take_while(|c| *c == ch).count();
    (len >= 3).then(|| (ch, len, t[len..].trim()))
}

enum State {
    Text,
    Code(char, usize),
    Mermaid(char, usize),
}

pub fn split(md: &str) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut text = String::new();
    let mut diagram = String::new();
    let mut state = State::Text;

    let flush_text = |text: &mut String, out: &mut Vec<Segment>| {
        if !text.trim().is_empty() {
            out.push(Segment::Markdown(std::mem::take(text)));
        } else {
            text.clear();
        }
    };

    for line in md.lines() {
        match state {
            State::Text => match fence(line) {
                Some((ch, len, info)) if info.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("mermaid")) => {
                    flush_text(&mut text, &mut out);
                    diagram.clear();
                    state = State::Mermaid(ch, len);
                }
                Some((ch, len, _)) => {
                    text.push_str(line);
                    text.push('\n');
                    state = State::Code(ch, len);
                }
                None => {
                    text.push_str(line);
                    text.push('\n');
                }
            },
            State::Code(ch, len) => {
                text.push_str(line);
                text.push('\n');
                if matches!(fence(line), Some((c, l, info)) if c == ch && l >= len && info.is_empty()) {
                    state = State::Text;
                }
            }
            State::Mermaid(ch, len) => {
                if matches!(fence(line), Some((c, l, info)) if c == ch && l >= len && info.is_empty()) {
                    out.push(Segment::Mermaid(std::mem::take(&mut diagram)));
                    state = State::Text;
                } else {
                    diagram.push_str(line);
                    diagram.push('\n');
                }
            }
        }
    }
    match state {
        State::Mermaid(..) => out.push(Segment::Mermaid(diagram)), // unterminated fence: still a diagram
        _ => flush_text(&mut text, &mut out),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(s: &str) -> Segment {
        Segment::Markdown(s.to_string())
    }
    fn mm(s: &str) -> Segment {
        Segment::Mermaid(s.to_string())
    }

    #[test]
    fn plain_markdown_is_one_segment() {
        assert_eq!(split("# Hi\n\ntext"), vec![md("# Hi\n\ntext\n")]);
        assert!(split("").is_empty());
        assert!(split("  \n\n").is_empty());
    }

    #[test]
    fn mermaid_block_is_split_out_between_text() {
        let s = "intro\n\n## Flow\n\n```mermaid\ngraph TD\n  A-->B\n```\n\noutro";
        assert_eq!(
            split(s),
            vec![md("intro\n\n## Flow\n\n"), mm("graph TD\n  A-->B\n"), md("\noutro\n")]
        );
    }

    #[test]
    fn several_blocks_and_tilde_fences() {
        let s = "```mermaid\nA-->B\n```\n~~~mermaid\nC-->D\n~~~\n";
        assert_eq!(split(s), vec![mm("A-->B\n"), mm("C-->D\n")]);
    }

    #[test]
    fn mermaid_inside_another_code_block_is_left_alone() {
        let s = "````text\n```mermaid\nA-->B\n```\n````\nafter";
        assert_eq!(split(s), vec![md(&format!("{s}\n"))]);
    }

    #[test]
    fn other_languages_stay_markdown() {
        let s = "```rust\nfn main() {}\n```";
        assert_eq!(split(s), vec![md("```rust\nfn main() {}\n```\n")]);
    }

    #[test]
    fn unterminated_mermaid_still_renders_as_diagram() {
        assert_eq!(split("```mermaid\nA-->B"), vec![mm("A-->B\n")]);
    }

    #[test]
    fn unicode_labels_survive() {
        let s = "```mermaid\ngraph LR\n  A[开始] --> B[مرحبا]\n```";
        assert_eq!(split(s), vec![mm("graph LR\n  A[开始] --> B[مرحبا]\n")]);
    }
}
