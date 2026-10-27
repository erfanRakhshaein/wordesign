//! Text direction helpers (the editor widgets lay text out with real bidi support;
//! single-line fields only need to know which edge to hug).

use eframe::egui::Align;

fn is_rtl_char(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

/// Direction of the first strongly directional letter (like GTK/gedit "auto" direction).
pub fn is_rtl(text: &str) -> bool {
    text.chars()
        .find(|c| c.is_alphabetic() || is_rtl_char(*c))
        .is_some_and(is_rtl_char)
}

/// Where a single-line field should anchor its text.
pub fn align_for(text: &str) -> Align {
    if is_rtl(text) {
        Align::Max
    } else {
        Align::Min
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_strong_letter_decides() {
        assert!(is_rtl("مرحبا hello"));
        assert!(is_rtl("123 שלום"));
        assert!(!is_rtl("hello مرحبا"));
        assert!(!is_rtl("  “quoted” 你好"));
        assert!(!is_rtl(""));
        assert!(is_rtl("(שלום)"));
    }
}
