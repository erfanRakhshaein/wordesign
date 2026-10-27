//! Fonts.
//!
//! * A bundled handwriting-style font (Patrick Hand, SIL OFL) is the main UI font.
//! * egui's default fonts stay behind it (Latin/Cyrillic/Greek, emoji, symbols).
//! * System fonts are scanned for the scripts egui's defaults lack (Chinese,
//!   Japanese, Korean, Arabic/Persian, Hebrew, Thai, Devanagari). A face is
//!   chosen by *which glyphs it really contains*, not by guessing file names,
//!   so it works on Linux, Windows and macOS alike.
//!
//! The same font database is handed to the flowchart renderer, so diagrams use
//! the same fonts as the rest of the app.

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const HANDWRITING: &[u8] = include_bytes!("../assets/fonts/PatrickHand-Regular.ttf");
pub const HANDWRITING_FAMILY: &str = "Patrick Hand";

struct Script {
    name: &'static str,
    /// A font is only chosen if it contains every one of these characters.
    samples: &'static [char],
    /// Preferred family names, best first. Anything else that covers the samples is a last resort.
    prefer: &'static [&'static str],
}

const SCRIPTS: &[Script] = &[
    Script {
        name: "CJK",
        samples: &['中', '文', '国', '的'],
        prefer: &[
            "Noto Sans CJK SC", "Noto Sans SC", "Source Han Sans SC", "Microsoft YaHei", "PingFang SC",
            "WenQuanYi Micro Hei", "Droid Sans Fallback", "Noto Sans CJK JP", "Noto Sans CJK TC",
            "Microsoft JhengHei", "SimSun", "Yu Gothic", "Hiragino Sans",
        ],
    },
    Script {
        name: "Kana",
        samples: &['あ', 'ア', 'ん'],
        prefer: &["Noto Sans CJK JP", "Noto Sans JP", "Yu Gothic", "Meiryo", "Hiragino Sans"],
    },
    Script {
        name: "Hangul",
        samples: &['한', '글'],
        prefer: &["Noto Sans CJK KR", "Noto Sans KR", "Malgun Gothic", "Apple SD Gothic Neo"],
    },
    Script {
        // Includes the Persian-specific letters so Persian text works too.
        name: "Arabic",
        samples: &['ا', 'ب', 'م', 'ی', 'پ', 'گ', 'ک'],
        prefer: &[
            "Noto Sans Arabic", "Noto Naskh Arabic", "Segoe UI", "Tahoma", "Arial", "Geeza Pro",
            "DejaVu Sans", "Noto Sans",
        ],
    },
    Script {
        name: "Hebrew",
        samples: &['א', 'ב', 'ת'],
        prefer: &["Noto Sans Hebrew", "Segoe UI", "Arial", "Arial Hebrew", "DejaVu Sans", "Noto Sans"],
    },
    Script {
        name: "Thai",
        samples: &['ก', 'ข', 'ะ'],
        prefer: &["Noto Sans Thai", "Leelawadee UI", "Tahoma", "Thonburi", "Noto Sans"],
    },
    Script {
        name: "Devanagari",
        samples: &['क', 'ह', 'ा'],
        prefer: &["Noto Sans Devanagari", "Nirmala UI", "Kohinoor Devanagari", "Noto Sans"],
    },
];

/// System fonts + the bundled handwriting font (sans-serif maps to it for diagrams).
pub fn load_database() -> fontdb::Database {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    db.load_font_data(HANDWRITING.to_vec());
    db.set_sans_serif_family(HANDWRITING_FAMILY);
    db
}

fn covers(db: &fontdb::Database, id: fontdb::ID, samples: &[char]) -> bool {
    db.with_face_data(id, |data, index| {
        ttf_parser::Face::parse(data, index)
            .map(|f| samples.iter().all(|c| f.glyph_index(*c).is_some()))
            .unwrap_or(false)
    })
    .unwrap_or(false)
}

fn rank(face: &fontdb::FaceInfo, script: &Script) -> usize {
    let listed = face
        .families
        .iter()
        .filter_map(|(n, _)| script.prefer.iter().position(|p| p.eq_ignore_ascii_case(n)))
        .min();
    listed.unwrap_or_else(|| {
        let noto = face.families.iter().any(|(n, _)| n.starts_with("Noto"));
        script.prefer.len() + usize::from(!noto)
    })
}

/// Best system face that contains every sample character of `script`.
fn pick(db: &fontdb::Database, script: &Script) -> Option<fontdb::ID> {
    let mut best: Option<((usize, u16), fontdb::ID)> = None;
    for face in db.faces() {
        if face.style != fontdb::Style::Normal || face.monospaced {
            continue;
        }
        if face.families.iter().any(|(n, _)| {
            let n = n.to_lowercase();
            n.contains("unifont") || n.contains("last resort") || n.contains("symbol") || n.contains("emoji")
        }) {
            continue;
        }
        let key = (rank(face, script), face.weight.0.abs_diff(400));
        if best.as_ref().is_some_and(|(b, _)| *b <= key) {
            continue; // cheap checks first: only parse the font if it could win
        }
        if covers(db, face.id, script.samples) {
            best = Some((key, face.id));
        }
    }
    best.map(|(_, id)| id)
}

fn face_data(
    db: &fontdb::Database,
    id: fontdb::ID,
    files: &mut HashMap<PathBuf, &'static [u8]>,
) -> Option<FontData> {
    let face = db.face(id)?;
    let path = match &face.source {
        fontdb::Source::File(p) => p.clone(),
        fontdb::Source::SharedFile(p, _) => p.clone(),
        fontdb::Source::Binary(_) => return None,
    };
    // Read each file once and keep it for the life of the process: several
    // faces of one collection (e.g. a CJK .ttc) then share the same bytes.
    let bytes: &'static [u8] = match files.get(&path) {
        Some(b) => b,
        None => {
            let data = std::fs::read(&path).ok()?;
            let leaked: &'static [u8] = Box::leak(data.into_boxed_slice());
            files.insert(path, leaked);
            leaked
        }
    };
    Some(FontData {
        font: Cow::Borrowed(bytes),
        index: face.index,
        tweak: Default::default(),
    })
}

/// Install the handwriting font and fallbacks into egui.
/// Returns the names of the scripts that are covered (for diagnostics).
pub fn install(ctx: &egui::Context, db: &fontdb::Database) -> Vec<&'static str> {
    let mut defs = FontDefinitions::default();
    defs.font_data.insert(
        HANDWRITING_FAMILY.to_owned(),
        Arc::new(FontData::from_static(HANDWRITING)),
    );
    defs.families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, HANDWRITING_FAMILY.to_owned());

    let mut chosen: Vec<fontdb::ID> = Vec::new();
    let mut files: HashMap<PathBuf, &'static [u8]> = HashMap::new();
    let mut covered = Vec::new();

    for script in SCRIPTS {
        // A font we already picked (e.g. a pan-CJK face) may cover this script too.
        if chosen.iter().any(|id| covers(db, *id, script.samples)) {
            covered.push(script.name);
            continue;
        }
        let Some(id) = pick(db, script) else { continue };
        let Some(data) = face_data(db, id, &mut files) else { continue };
        let key = format!("system-{}", script.name.to_lowercase());
        defs.font_data.insert(key.clone(), Arc::new(data));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            defs.families.entry(family).or_default().push(key.clone());
        }
        chosen.push(id);
        covered.push(script.name);
    }

    ctx.set_fonts(defs);
    covered
}
