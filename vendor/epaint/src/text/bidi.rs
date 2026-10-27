//! Bidirectional text support (Unicode UAX #9) for text layout.
//!
//! Only texts that actually contain right-to-left characters pay for the analysis.

use core::ops::Range;

use unicode_bidi::BidiInfo;

/// Per-byte embedding levels and per-paragraph base levels of a piece of text.
pub(crate) struct Bidi {
    levels: Vec<u8>,
    paragraphs: Vec<(Range<usize>, u8)>,
}

/// Cheap pre-check: does the text contain any strongly right-to-left characters or controls?
fn has_rtl(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c as u32,
            0x0590..=0x08FF
            | 0x200F | 0x202B | 0x202E | 0x2067
            | 0xFB1D..=0xFDFF
            | 0xFE70..=0xFEFF
            | 0x10800..=0x10FFF
            | 0x1E800..=0x1EFFF)
    })
}

impl Bidi {
    /// `None` for text without right-to-left characters (everything is plain LTR).
    pub fn analyze(text: &str) -> Option<Self> {
        if !has_rtl(text) {
            return None;
        }
        // Each paragraph gets its own base direction from its first strong character (like gedit / GTK).
        let info = BidiInfo::new(text, None);
        Some(Self {
            levels: info.levels.iter().map(|l| l.number()).collect(),
            paragraphs: info
                .paragraphs
                .iter()
                .map(|p| (p.range.clone(), p.level.number()))
                .collect(),
        })
    }

    pub fn level_at(&self, byte: usize) -> u8 {
        self.levels.get(byte).copied().unwrap_or(0)
    }

    /// Base level (0 = LTR, 1 = RTL) of the paragraph containing `byte`.
    pub fn paragraph_level_at(&self, byte: usize) -> u8 {
        self.paragraphs
            .iter()
            .find(|(r, _)| r.contains(&byte))
            .or(self.paragraphs.last().filter(|(r, _)| byte >= r.end))
            .map_or(0, |(_, l)| *l)
    }
}

/// Rule L2: from the highest level down to the lowest odd level, reverse every
/// contiguous run of items at that level or higher. Returns indices in visual order.
pub(crate) fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(&highest) = levels.iter().max() else {
        return order;
    };
    let Some(lowest_odd) = levels.iter().copied().filter(|l| l & 1 == 1).min() else {
        return order;
    };
    let mut level = highest;
    while level >= lowest_odd {
        let mut i = 0;
        while i < order.len() {
            if levels[order[i]] >= level {
                let start = i;
                while i < order.len() && levels[order[i]] >= level {
                    i += 1;
                }
                order[start..i].reverse();
            } else {
                i += 1;
            }
        }
        if level == 0 {
            break;
        }
        level -= 1;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ltr_has_no_analysis() {
        assert!(Bidi::analyze("hello world 123").is_none());
    }

    #[test]
    fn paragraph_direction_follows_first_strong_char() {
        let b = Bidi::analyze("שלום world\nhello שלום").unwrap();
        assert_eq!(b.paragraph_level_at(0), 1);
        let second = "שלום world\n".len();
        assert_eq!(b.paragraph_level_at(second), 0);
    }

    #[test]
    fn reorders_an_rtl_run_inside_ltr() {
        // levels: a b [R R R] c  => visual: a b R R R(reversed) c
        assert_eq!(visual_order(&[0, 0, 1, 1, 1, 0]), vec![0, 1, 4, 3, 2, 5]);
    }

    #[test]
    fn numbers_inside_rtl_keep_their_order() {
        // RTL paragraph: [R R] 12 [R]: levels 1 1 2 1
        assert_eq!(visual_order(&[1, 1, 2, 1]), vec![3, 2, 1, 0]);
    }

    #[test]
    fn all_ltr_is_identity() {
        assert_eq!(visual_order(&[0, 0, 0]), vec![0, 1, 2]);
    }
}
