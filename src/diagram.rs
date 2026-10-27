//! Draws ```mermaid blocks as real diagrams in the preview.
//!
//! mermaid text --(mermaid-rs-renderer)--> SVG --(usvg + resvg)--> pixels --> egui texture.
//! Text is laid out by resvg with the app's own font database, so handwriting,
//! Chinese and (shaped) Arabic labels all render. Results are cached per source.

use eframe::egui::{self, load::SizedTexture, Color32, RichText, TextureHandle, TextureOptions, Ui, Vec2};
use mermaid_rs_renderer::{RenderOptions, Theme};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use crate::{fonts, theme};

const MAX_PIXELS: f32 = 12_000_000.0;
const MAX_CACHED: usize = 48;

struct Rendered {
    tex: TextureHandle,
    /// Natural size in egui points.
    size: Vec2,
}

pub struct Diagrams {
    db: Arc<fontdb::Database>,
    cache: HashMap<u64, Result<Rendered, String>>,
}

/// Colors matching the app's violet / neon look.
fn dark_theme() -> Theme {
    let mut t = Theme::modern();
    t.font_family = format!("'{}', sans-serif", fonts::HANDWRITING_FAMILY);
    t.font_size = 16.8;
    t.primary_color = "#2a1f47".into();
    t.primary_text_color = "#f1ecfb".into();
    t.primary_border_color = "#b44cff".into();
    t.line_color = "#b99bff".into();
    t.secondary_color = "#33265a".into();
    t.tertiary_color = "#3b2c6b".into();
    t.edge_label_background = "rgba(18,14,27,0.92)".into();
    t.cluster_background = "rgba(180,76,255,0.08)".into();
    t.cluster_border = "#7a45b8".into();
    t.background = "#1a1527".into(); // same as the card behind it
    t.text_color = "#f1ecfb".into();
    t
}

/// mermaid text -> pixels. `scale` is shrunk automatically for huge diagrams.
fn rasterize(db: &Arc<fontdb::Database>, src: &str, scale: f32) -> Result<(resvg::tiny_skia::Pixmap, f32), String> {
    // The layout engine is third-party code fed with half-typed text: never let it take the UI down.
    let svg = catch_unwind(AssertUnwindSafe(|| {
        mermaid_rs_renderer::render_with_options(
            src,
            RenderOptions {
                theme: dark_theme(),
                layout: Default::default(),
            },
        )
    }))
    .map_err(|_| "the diagram engine could not handle this input".to_string())?
    .map_err(|e| e.to_string())?;

    let mut opt = usvg::Options::default();
    opt.font_family = fonts::HANDWRITING_FAMILY.to_string();
    opt.fontdb = db.clone();
    let tree = usvg::Tree::from_str(&svg, &opt).map_err(|e| e.to_string())?;

    let size = tree.size();
    let (w0, h0) = (size.width().max(1.0), size.height().max(1.0));
    let mut scale = scale;
    while w0 * h0 * scale * scale > MAX_PIXELS && scale > 0.2 {
        scale *= 0.8;
    }
    let (w, h) = ((w0 * scale).ceil() as u32, (h0 * scale).ceil() as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or("diagram is too large to draw")?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Ok((pixmap, scale))
}

fn render(ctx: &egui::Context, db: &Arc<fontdb::Database>, src: &str, key: u64) -> Result<Rendered, String> {
    let (pixmap, scale) = rasterize(db, src, ctx.pixels_per_point().clamp(1.0, 3.0))?;
    let (w, h) = (pixmap.width(), pixmap.height());
    let image = egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data());
    let tex = ctx.load_texture(format!("mermaid-{key:x}"), image, TextureOptions::LINEAR);
    Ok(Rendered {
        tex,
        size: Vec2::new(w as f32 / scale, h as f32 / scale),
    })
}

impl Diagrams {
    pub fn new(db: Arc<fontdb::Database>) -> Self {
        Self {
            db,
            cache: HashMap::new(),
        }
    }

    pub fn show(&mut self, ui: &mut Ui, src: &str) {
        let ppp = ui.ctx().pixels_per_point();
        let mut h = DefaultHasher::new();
        src.hash(&mut h);
        ((ppp * 100.0) as u32).hash(&mut h);
        let key = h.finish();

        if self.cache.len() >= MAX_CACHED && !self.cache.contains_key(&key) {
            self.cache.clear();
        }
        let db = &self.db;
        let entry = self
            .cache
            .entry(key)
            .or_insert_with(|| render(ui.ctx(), db, src, key));

        match entry {
            Ok(r) => {
                theme::card(ui, |ui| {
                    let avail = (ui.available_width() - 2.0).max(40.0);
                    let k = (avail / r.size.x).min(1.0);
                    ui.vertical_centered(|ui| {
                        ui.add(egui::Image::new(SizedTexture::new(r.tex.id(), r.size * k)));
                    });
                });
            }
            Err(msg) => {
                theme::card(ui, |ui| {
                    ui.label(RichText::new(format!("Can't draw this diagram: {msg}")).small().color(theme::WARN));
                    ui.add_space(4.0);
                    ui.label(RichText::new(src.trim_end()).monospace().color(Color32::from_gray(200)));
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Arc<fontdb::Database> {
        Arc::new(fonts::load_database())
    }

    fn has_ink(p: &resvg::tiny_skia::Pixmap) -> bool {
        p.pixels().iter().any(|px| px.alpha() > 0 && (px.red() > 150 || px.green() > 150 || px.blue() > 150))
    }

    #[test]
    fn flowchart_with_chinese_and_arabic_labels_renders() {
        let src = "graph TD\n A[New ticket 新工单] --> B{紧急?}\n B -->|نعم| C[قائمة الانتظار الطويلة]\n B -->|No| D[Done]";
        let (pm, scale) = rasterize(&db(), src, 2.0).expect("should render");
        assert!(pm.width() > 100 && pm.height() > 100 && scale > 0.0);
        assert!(has_ink(&pm), "diagram is blank");
    }

    #[test]
    fn invalid_diagrams_report_an_error_instead_of_crashing() {
        for bad in ["", "this is not mermaid at all", "graph TD\n A -->", "\u{0}\u{1}"] {
            let _ = rasterize(&db(), bad, 1.0); // Ok or Err, but never a panic
        }
    }

    #[test]
    fn handwriting_font_is_in_the_database() {
        let db = db();
        let q = fontdb::Query {
            families: &[fontdb::Family::Name(fonts::HANDWRITING_FAMILY)],
            ..Default::default()
        };
        assert!(db.query(&q).is_some());
    }
}
