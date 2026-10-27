//! Look & feel: deep blue surfaces (default), neon borders with a soft glow,
//! slightly rounded corners. Also home to the small widget helpers that keep
//! every button the same height so toolbars line up.

use eframe::egui::{
    self, epaint::Shadow, Color32, Context, CornerRadius, FontFamily, FontId, Frame, Margin,
    Painter, Rect, RichText, Stroke, StrokeKind, TextStyle, Theme, Ui, Vec2, Visuals,
};

/// Accent used until the user picks another one.
pub const VIOLET_ACCENT: Color32 = Color32::from_rgb(0xb4, 0x4c, 0xff);
pub const DEFAULT_ACCENT: Color32 = Color32::from_rgb(0x4c, 0x8d, 0xff);
/// The neutral grey theme.
pub const GRAY_ACCENT: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xaa);

/// Every colour that follows the theme colour. Surfaces are dark tints of the accent's hue.
#[derive(Clone, Copy)]
struct Palette {
    bg_root: Color32,
    bg_panel: Color32,
    bg_card: Color32,
    bg_input: Color32,
    bg_button: Color32,
    bg_button_hot: Color32,
    bg_active: Color32,
    neon: Color32,
    neon_hot: Color32,
    separator: Color32,
}

impl Palette {
    /// The original violet look, exactly.
    const VIOLET: Palette = Palette {
        bg_root: Color32::from_rgb(0x10, 0x0d, 0x18),
        bg_panel: Color32::from_rgb(0x15, 0x11, 0x20),
        bg_card: Color32::from_rgb(0x1a, 0x15, 0x27),
        bg_input: Color32::from_rgb(0x12, 0x0e, 0x1b),
        bg_button: Color32::from_rgb(0x21, 0x1a, 0x33),
        bg_button_hot: Color32::from_rgb(0x2c, 0x20, 0x47),
        bg_active: Color32::from_rgb(0x4a, 0x24, 0x80),
        neon: VIOLET_ACCENT,
        neon_hot: Color32::from_rgb(0xd2, 0x7b, 0xff),
        separator: Color32::from_rgb(0x2d, 0x24, 0x42),
    };

    /// Neutral surfaces (no hue) with a soft grey accent.
    const GRAY: Palette = Palette {
        bg_root: Color32::from_rgb(0x12, 0x12, 0x14),
        bg_panel: Color32::from_rgb(0x18, 0x18, 0x1b),
        bg_card: Color32::from_rgb(0x1e, 0x1e, 0x22),
        bg_input: Color32::from_rgb(0x14, 0x14, 0x17),
        bg_button: Color32::from_rgb(0x28, 0x28, 0x2d),
        bg_button_hot: Color32::from_rgb(0x36, 0x36, 0x3d),
        bg_active: Color32::from_rgb(0x4a, 0x4d, 0x58),
        neon: GRAY_ACCENT,
        neon_hot: Color32::from_rgb(0xd0, 0xd4, 0xdc),
        separator: Color32::from_rgb(0x30, 0x30, 0x36),
    };

    fn from_accent(accent: Color32) -> Palette {
        if accent == VIOLET_ACCENT {
            return Self::VIOLET;
        }
        if accent == GRAY_ACCENT {
            return Self::GRAY;
        }
        // HSV in plain sRGB (gamma) space, which is how the original palette was designed.
        let (h0, sat, val) = egui::ecolor::hsv_from_rgb([
            f32::from(accent.r()) / 255.0,
            f32::from(accent.g()) / 255.0,
            f32::from(accent.b()) / 255.0,
        ]);
        let to_color = |hsv: (f32, f32, f32)| -> Color32 {
            let [r, g, b] = egui::ecolor::rgb_from_hsv(hsv);
            Color32::from_rgb((r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8)
        };
        // The original surfaces sit ~14 degrees "below" the accent on the colour wheel.
        let h = (h0 - 14.0 / 360.0).rem_euclid(1.0);
        let tint = |s: f32, v: f32| to_color((h, s, v));
        // Lighter, softer twin of the accent for highlights; stays readable for dark or grey accents.
        let hot = to_color((h0, sat * 0.74, (val + (1.0 - val) * 0.6).min(1.0)));
        Palette {
            bg_root: tint(0.46, 0.094),
            bg_panel: tint(0.45, 0.125),
            bg_card: tint(0.46, 0.153),
            bg_input: tint(0.48, 0.106),
            bg_button: tint(0.49, 0.20),
            bg_button_hot: tint(0.55, 0.28),
            bg_active: tint(0.72, 0.50),
            neon: accent,
            neon_hot: hot,
            separator: tint(0.45, 0.26),
        }
    }
}

static PALETTE: std::sync::RwLock<Palette> = std::sync::RwLock::new(Palette::VIOLET);

fn pal() -> Palette {
    *PALETTE.read().unwrap_or_else(|e| e.into_inner())
}

/// Switch the whole app to a new theme colour (call `apply` afterwards to refresh egui's visuals).
pub fn set_accent(accent: Color32) {
    *PALETTE.write().unwrap_or_else(|e| e.into_inner()) = Palette::from_accent(accent);
}

pub fn bg_root() -> Color32 { pal().bg_root }
pub fn bg_panel() -> Color32 { pal().bg_panel }
pub fn bg_card() -> Color32 { pal().bg_card }
pub fn bg_input() -> Color32 { pal().bg_input }
pub fn bg_button() -> Color32 { pal().bg_button }
pub fn bg_button_hot() -> Color32 { pal().bg_button_hot }
pub fn bg_active() -> Color32 { pal().bg_active }
pub fn neon() -> Color32 { pal().neon }
pub fn neon_hot() -> Color32 { pal().neon_hot }
pub fn separator() -> Color32 { pal().separator }

pub const TEXT: Color32 = Color32::from_rgb(0xec, 0xe8, 0xf6);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x8f, 0x86, 0xa8);
pub const GOOD: Color32 = Color32::from_rgb(0x7b, 0xe0, 0xa4);
/// Errors (unaddressed variables, blocked export).
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0x4d, 0x5e);
pub const WARN: Color32 = Color32::from_rgb(0xf0, 0xb8, 0x6a);

/// Corner radius used everywhere ("slightly round").
pub const RADIUS: u8 = 7;
/// Height shared by every toolbar control.
pub const CONTROL_HEIGHT: f32 = 30.0;

pub fn apply(ctx: &Context) {
    ctx.set_theme(egui::ThemePreference::Dark);

    let mut v = Visuals::dark();
    let r = CornerRadius::same(RADIUS);

    v.override_text_color = Some(TEXT);
    v.panel_fill = bg_panel();
    v.window_fill = bg_card();
    v.extreme_bg_color = bg_input();
    v.faint_bg_color = bg_card();
    v.code_bg_color = bg_input();
    v.hyperlink_color = neon_hot();

    v.window_corner_radius = CornerRadius::same(RADIUS + 3);
    v.menu_corner_radius = r;
    v.window_stroke = Stroke::new(1.5, neon());
    let glow = Shadow {
        offset: [0, 0],
        blur: 24,
        spread: 2,
        color: neon().gamma_multiply(70.0 / 255.0),
    };
    v.window_shadow = glow;
    v.popup_shadow = glow;

    v.selection.bg_fill = neon().gamma_multiply(85.0 / 255.0);
    v.selection.stroke = Stroke::new(1.5, neon());
    v.text_cursor.stroke = Stroke::new(2.0, neon_hot());

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = bg_panel();
    w.noninteractive.weak_bg_fill = bg_panel();
    w.noninteractive.bg_stroke = Stroke::new(1.0, separator());
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = bg_button();
    w.inactive.weak_bg_fill = bg_button();
    w.inactive.bg_stroke = Stroke::new(1.0, neon().gamma_multiply(0.45));
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = bg_button_hot();
    w.hovered.weak_bg_fill = bg_button_hot();
    w.hovered.bg_stroke = Stroke::new(1.5, neon());
    w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = bg_active();
    w.active.weak_bg_fill = bg_active();
    w.active.bg_stroke = Stroke::new(1.5, neon_hot());
    w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = bg_button_hot();
    w.open.weak_bg_fill = bg_button_hot();
    w.open.bg_stroke = Stroke::new(1.5, neon());
    w.open.corner_radius = r;

    ctx.set_visuals_of(Theme::Dark, v);

    ctx.style_mut_of(Theme::Dark, |s| {
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.button_padding = Vec2::new(12.0, 6.0);
        s.spacing.interact_size = Vec2::new(36.0, CONTROL_HEIGHT);
        s.spacing.scroll.bar_width = 8.0;
        s.text_styles.insert(TextStyle::Heading, FontId::new(24.2, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Body, FontId::new(17.85, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Button, FontId::new(16.8, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Small, FontId::new(14.2, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Monospace, FontId::new(14.7, FontFamily::Monospace));
    });
}

// ---------------------------------------------------------------------
// Neon painting
// ---------------------------------------------------------------------

/// Paint a neon outline (with glow when `hot`) around `rect`.
pub fn paint_neon(painter: &Painter, rect: Rect, hot: bool) {
    let r = CornerRadius::same(RADIUS);
    if hot {
        painter.rect_stroke(
            rect.expand(3.0),
            CornerRadius::same(RADIUS + 3),
            Stroke::new(6.0, neon().gamma_multiply(0.07)),
            StrokeKind::Outside,
        );
        painter.rect_stroke(
            rect.expand(1.0),
            CornerRadius::same(RADIUS + 1),
            Stroke::new(2.5, neon().gamma_multiply(0.20)),
            StrokeKind::Outside,
        );
        painter.rect_stroke(rect, r, Stroke::new(1.5, neon()), StrokeKind::Inside);
    } else {
        painter.rect_stroke(
            rect.expand(1.0),
            CornerRadius::same(RADIUS + 1),
            Stroke::new(2.0, neon().gamma_multiply(0.06)),
            StrokeKind::Outside,
        );
        painter.rect_stroke(rect, r, Stroke::new(1.0, neon().gamma_multiply(0.55)), StrokeKind::Inside);
    }
}

fn focus_within(ctx: &Context, rect: Rect) -> bool {
    ctx.memory(|m| m.focused())
        .and_then(|id| ctx.read_response(id))
        .is_some_and(|r| rect.contains(r.rect.center()))
}

/// A rounded card with a neon outline that lights up on hover / keyboard focus.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let out = Frame::new()
        .fill(bg_card())
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::same(12))
        .show(ui, add);
    let rect = out.response.rect;
    let hot = ui.rect_contains_pointer(rect) || focus_within(ui.ctx(), rect);
    paint_neon(ui.painter(), rect, hot);
    out.inner
}

/// A compact variant for collapsed items (flow chips).
pub fn chip<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let out = Frame::new()
        .fill(bg_button())
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, add);
    let rect = out.response.rect;
    let hot = ui.rect_contains_pointer(rect) || focus_within(ui.ctx(), rect);
    paint_neon(ui.painter(), rect, hot);
    out.inner
}

// ---------------------------------------------------------------------
// Widgets
// ---------------------------------------------------------------------

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(TEXT_MUTED)
}

pub fn small_caps_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).small().strong().color(neon_hot()));
}

fn base_button(text: RichText) -> egui::Button<'static> {
    egui::Button::new(text)
        .min_size(Vec2::new(0.0, CONTROL_HEIGHT))
        .corner_radius(CornerRadius::same(RADIUS))
}

/// Standard toolbar button.
pub fn tool_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(base_button(RichText::new(text)))
}

/// A button spanning the given width (sidebar actions).
pub fn wide_button(ui: &mut Ui, width: f32, text: &str) -> egui::Response {
    ui.add_sized(Vec2::new(width, CONTROL_HEIGHT), base_button(RichText::new(text)))
}

/// Toolbar button that can be disabled (e.g. a section that already exists).
pub fn tool_button_enabled(ui: &mut Ui, enabled: bool, text: &str) -> egui::Response {
    ui.add_enabled(enabled, base_button(RichText::new(text)))
}

/// The one accented action (Export).
pub fn primary_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(
        base_button(RichText::new(text).strong().color(Color32::WHITE))
            .fill(bg_active())
            .stroke(Stroke::new(1.5, neon())),
    )
}

/// Tiny borderless button used for per-item actions inside cards (remove, add item).
pub fn ghost_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(TEXT_MUTED))
            .frame(false)
            .min_size(Vec2::new(24.0, 24.0)),
    )
}
