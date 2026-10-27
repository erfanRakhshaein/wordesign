//! Small persistent settings, stored next to the prompts in `~/.wordesign/.settings`
//! as plain `key=value` lines.

use std::path::{Path, PathBuf};

/// The default theme: deep blue.
pub const DEFAULT_ACCENT: [u8; 3] = [0x4c, 0x8d, 0xff];
/// The earlier default, kept as a preset.
pub const VIOLET: [u8; 3] = [0xb4, 0x4c, 0xff];
/// The neutral grey theme.
pub const GRAY: [u8; 3] = [0x9a, 0xa0, 0xaa];

/// Choices offered in the Settings menu (days); 0 = never.
pub const TEMP_CHOICES: [u32; 8] = [1, 2, 3, 5, 7, 14, 30, 0];

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Prompts in the temp folder expire this many days after their last change (0 = never).
    pub temp_days: u32,
    /// Theme colour (RGB).
    pub accent: [u8; 3],
    /// The cheatsheet has been shown once.
    pub cheatsheet_seen: bool,
    /// UI scale chosen by zooming (None = the built-in default).
    pub zoom: Option<f32>,
    /// Model last used for the cost estimate in the preview.
    pub cost_model: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self { temp_days: 3, accent: DEFAULT_ACCENT, cheatsheet_seen: false, zoom: None, cost_model: String::new() }
    }
}

fn file(dir: &Path) -> PathBuf {
    dir.join(".settings")
}

impl Settings {
    pub fn load(dir: &Path) -> Self {
        let mut s = Self::default();
        if let Ok(text) = std::fs::read_to_string(file(dir)) {
            let chose_theme = text.lines().any(|l| l.trim() == "theme=2");
            for line in text.lines() {
                match line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) {
                    Some(("temp_days", v)) => {
                        if let Ok(n) = v.parse::<u32>() {
                            s.temp_days = n.min(3650);
                        }
                    }
                    Some(("accent", v)) => {
                        if let Some(c) = parse_hex(v) {
                            s.accent = c;
                        }
                    }
                    Some(("cheatsheet", v)) => s.cheatsheet_seen = v == "1",
                    Some(("zoom", v)) => {
                        if let Ok(z) = v.parse::<f32>() {
                            if (0.5..=4.0).contains(&z) {
                                s.zoom = Some(z);
                            }
                        }
                    }
                    Some(("cost_model", v)) => s.cost_model = v.to_owned(),
                    _ => {}
                }
            }
            // Files from before the blue default: violet was only the default then, not a choice.
            if !chose_theme && s.accent == VIOLET {
                s.accent = DEFAULT_ACCENT;
            }
        }
        s
    }

    pub fn save(&self, dir: &Path) {
        let _ = std::fs::write(file(dir), format!(
                "theme=2\ntemp_days={}\naccent=#{:02x}{:02x}{:02x}\ncheatsheet={}\nzoom={}\ncost_model={}\n",
                self.temp_days, self.accent[0], self.accent[1], self.accent[2], u8::from(self.cheatsheet_seen),
                self.zoom.map_or(String::new(), |z| format!("{z:.3}")),
                self.cost_model.replace('\n', " ")
            ));
    }

    pub fn temp_max_age(&self) -> Option<std::time::Duration> {
        (self.temp_days > 0).then(|| std::time::Duration::from_secs(u64::from(self.temp_days) * 86_400))
    }
}

fn parse_hex(v: &str) -> Option<[u8; 3]> {
    let h = v.strip_prefix('#')?;
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let n = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([n(0)?, n(2)?, n(4)?])
}

pub fn describe_days(days: u32) -> String {
    match days {
        0 => "Never".to_owned(),
        1 => "After 1 day".to_owned(),
        n => format!("After {n} days"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let dir = std::env::temp_dir().join(format!("wd-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(file(&dir));
        assert_eq!(Settings::load(&dir).temp_days, 3);
        Settings { temp_days: 7, accent: [1, 2, 255], cheatsheet_seen: true, zoom: Some(1.2), cost_model: "gpt-5".into() }.save(&dir);
        assert_eq!(Settings::load(&dir).temp_days, 7);
        assert_eq!(Settings::load(&dir).accent, [1, 2, 255]);
        assert!(Settings::load(&dir).cheatsheet_seen);
        assert_eq!(Settings::load(&dir).zoom, Some(1.2));
        assert_eq!(Settings::load(&dir).cost_model, "gpt-5");
        std::fs::write(file(&dir), "zoom=99\n").unwrap();
        assert_eq!(Settings::load(&dir).zoom, None, "absurd zoom values are ignored");
        std::fs::write(file(&dir), "temp_days=3\nzoom=\n").unwrap();
        assert_eq!(Settings::load(&dir).zoom, None);
        Settings { temp_days: 0, ..Settings::default() }.save(&dir);
        assert_eq!(Settings::load(&dir).temp_max_age(), None);
        std::fs::write(file(&dir), "temp_days=banana\naccent=#12\n").unwrap();
        assert_eq!(Settings::load(&dir).temp_days, 3);
        assert_eq!(Settings::load(&dir).accent, DEFAULT_ACCENT);
        // An old file with the old default colour moves to the new default; a deliberate pick stays.
        std::fs::write(file(&dir), "accent=#b44cff\n").unwrap();
        assert_eq!(Settings::load(&dir).accent, DEFAULT_ACCENT);
        Settings { accent: VIOLET, ..Settings::default() }.save(&dir);
        assert_eq!(Settings::load(&dir).accent, VIOLET);
    }
}
