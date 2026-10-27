//! Alt+Up / Alt+Down: move the current line (or the selected lines) like VS Code.

/// Move the lines touched by the selection `(anchor, head)` (char indices) one line up or down.
/// Returns the new text and selection, or `None` when already at the edge.
pub fn move_lines(text: &str, anchor: usize, head: usize, up: bool) -> Option<(String, usize, usize)> {
    let lines: Vec<&str> = text.split('\n').collect();
    // char index at which each line starts
    let mut starts = Vec::with_capacity(lines.len());
    let mut at = 0usize;
    for l in &lines {
        starts.push(at);
        at += l.chars().count() + 1;
    }
    let line_of = |pos: usize| starts.iter().rposition(|&s| s <= pos).unwrap_or(0);
    let (lo, hi) = (anchor.min(head), anchor.max(head));
    let first = line_of(lo);
    let mut last = line_of(hi);
    // a selection ending exactly at the start of a line does not include that line
    if hi > lo && last > first && starts[last] == hi {
        last -= 1;
    }
    let len = |i: usize| lines[i].chars().count() + 1;
    let (new_lines, delta): (Vec<&str>, isize) = if up {
        if first == 0 {
            return None;
        }
        let mut v: Vec<&str> = lines[..first - 1].to_vec();
        v.extend_from_slice(&lines[first..=last]);
        v.push(lines[first - 1]);
        v.extend_from_slice(&lines[last + 1..]);
        (v, -(len(first - 1) as isize))
    } else {
        if last + 1 >= lines.len() {
            return None;
        }
        let mut v: Vec<&str> = lines[..first].to_vec();
        v.push(lines[last + 1]);
        v.extend_from_slice(&lines[first..=last]);
        v.extend_from_slice(&lines[last + 2..]);
        (v, len(last + 1) as isize)
    };
    let shift = |p: usize| (p as isize + delta).max(0) as usize;
    Some((new_lines.join("\n"), shift(anchor), shift(head)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_single_line_down_and_up_with_cursor() {
        let (t, a, h) = move_lines("one\ntwo\nthree", 5, 5, false).unwrap();
        assert_eq!((t.as_str(), a, h), ("one\nthree\ntwo", 11, 11));
        let (t, a, _) = move_lines(&t, a, h, true).unwrap();
        assert_eq!((t.as_str(), a), ("one\ntwo\nthree", 5));
    }

    #[test]
    fn edges_do_nothing() {
        assert!(move_lines("a\nb", 0, 0, true).is_none());
        assert!(move_lines("a\nb", 3, 3, false).is_none());
        assert!(move_lines("only", 2, 2, false).is_none());
    }

    #[test]
    fn moves_a_multi_line_selection_as_a_block() {
        // select "b\nc" (chars 2..5)
        let (t, a, h) = move_lines("a\nb\nc\nd", 2, 5, true).unwrap();
        assert_eq!((t.as_str(), a, h), ("b\nc\na\nd", 0, 3));
        // selection ending at the start of the next line excludes that line
        let (t, ..) = move_lines("a\nb\nc\nd", 2, 4, false).unwrap();
        assert_eq!(t, "a\nc\nb\nd");
    }

    #[test]
    fn unicode_and_empty_lines() {
        let (t, a, _) = move_lines("مرحبا\n\n你好", 0, 0, false).unwrap();
        assert_eq!(t, "\nمرحبا\n你好");
        assert_eq!(a, 1);
    }
}
