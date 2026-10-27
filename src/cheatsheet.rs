//! The built-in cheatsheet, shipped as a normal prompt.

use crate::model::{BodyBlock, Prompt};

pub const NAME: &str = "Cheatsheet";

pub fn prompt() -> Prompt {
    let mut p = Prompt::new();
    p.body = vec![BodyBlock::Text(include_str!("cheatsheet.md").trim_end().to_owned())];
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cheatsheet_is_clean_in_the_preview() {
        let p = prompt();
        assert!(p.unaddressed_variables().is_empty(), "tags in backticks must not count as variables");
        assert!(p.export_minified().contains("{{codechunk_1}}"), "and they are exported literally");
        let spell = crate::spell::Checker::new(None);
        assert_eq!(spell.count_misspelled(&p), 0, "no red words in the cheatsheet");
        assert!(Prompt::from_file(&p.to_file()).export_minified() == p.export_minified());
    }
}
