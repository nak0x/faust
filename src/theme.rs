//! Colour palette, typography and the egui style derived from them.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke};

use crate::config::{Config, ThemeMode};

/// Family the user asked for. Each face is looked up as a separate egui family
/// because egui selects weight and slant by family, not by a style flag.
const FACES: [(&str, &str); 4] = [
    ("faust-regular", "Regular"),
    ("faust-bold", "Bold"),
    ("faust-italic", "Italic"),
    ("faust-bold-italic", "BoldItalic"),
];

/// Directories scanned when the fast path misses, in ascending priority.
const FONT_DIRS: [&str; 5] = [
    "/usr/share/fonts",
    "/usr/local/share/fonts",
    "/run/current-system/sw/share/X11/fonts",
    ".local/share/fonts",
    ".fonts",
];

/// The four egui font families Faust renders with.
#[derive(Clone)]
pub struct Faces {
    pub regular: FontFamily,
    pub bold: FontFamily,
    pub italic: FontFamily,
    pub bold_italic: FontFamily,
}

impl Faces {
    fn new() -> Self {
        Self {
            regular: FontFamily::Monospace,
            bold: FontFamily::Name(FACES[1].0.into()),
            italic: FontFamily::Name(FACES[2].0.into()),
            bold_italic: FontFamily::Name(FACES[3].0.into()),
        }
    }

    /// Picks the face matching a combination of emphasis marks.
    pub fn pick(&self, bold: bool, italic: bool) -> FontFamily {
        match (bold, italic) {
            (false, false) => self.regular.clone(),
            (true, false) => self.bold.clone(),
            (false, true) => self.italic.clone(),
            (true, true) => self.bold_italic.clone(),
        }
    }
}

/// Every colour Faust paints with.
#[derive(Clone)]
pub struct Theme {
    pub faces: Faces,
    pub font_size: f32,
    /// The translucent window surface.
    pub surface: Color32,
    /// Slightly shifted surface for the tree and tab strip.
    pub surface_alt: Color32,
    /// The line that separates UI areas.
    pub separator: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub selection: Color32,
    pub hover: Color32,
    pub code_bg: Color32,
    pub link: Color32,
    pub marker: Color32,
}

impl Theme {
    pub fn new(cfg: &Config) -> Self {
        let alpha = (cfg.opacity.clamp(0.05, 1.0) * 255.0).round() as u8;
        let faces = Faces::new();
        match cfg.theme {
            // The requested look: an #e1e1e1 sheet of frosted glass.
            ThemeMode::Light => Self {
                faces,
                font_size: cfg.font_size,
                surface: Color32::from_rgba_unmultiplied(0xe1, 0xe1, 0xe1, alpha),
                surface_alt: Color32::from_rgba_unmultiplied(0xd7, 0xd7, 0xd7, alpha),
                separator: Color32::from_rgb(0xb4, 0xb4, 0xb4),
                text: Color32::from_rgb(0x1f, 0x1f, 0x1f),
                text_dim: Color32::from_rgb(0x5c, 0x5c, 0x5c),
                text_faint: Color32::from_rgb(0x8c, 0x8c, 0x8c),
                accent: Color32::from_rgb(0x2c, 0x6a, 0x9e),
                selection: Color32::from_rgba_unmultiplied(0x2c, 0x6a, 0x9e, 0x33),
                hover: Color32::from_rgba_unmultiplied(0x00, 0x00, 0x00, 0x14),
                code_bg: Color32::from_rgba_unmultiplied(0x00, 0x00, 0x00, 0x12),
                link: Color32::from_rgb(0x1e, 0x5f, 0x8f),
                marker: Color32::from_rgb(0x9a, 0x9a, 0x9a),
            },
            ThemeMode::Dark => Self {
                faces,
                font_size: cfg.font_size,
                surface: Color32::from_rgba_unmultiplied(0x1a, 0x1a, 0x1a, alpha),
                surface_alt: Color32::from_rgba_unmultiplied(0x22, 0x22, 0x22, alpha),
                separator: Color32::from_rgb(0x4a, 0x4a, 0x4a),
                text: Color32::from_rgb(0xe1, 0xe1, 0xe1),
                text_dim: Color32::from_rgb(0xa0, 0xa0, 0xa0),
                text_faint: Color32::from_rgb(0x6b, 0x6b, 0x6b),
                accent: Color32::from_rgb(0x7f, 0xb0, 0xe6),
                selection: Color32::from_rgba_unmultiplied(0x7f, 0xb0, 0xe6, 0x33),
                hover: Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0x14),
                code_bg: Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0x0e),
                link: Color32::from_rgb(0x7f, 0xb0, 0xe6),
                marker: Color32::from_rgb(0x63, 0x63, 0x63),
            },
        }
    }

    pub fn separator_stroke(&self) -> Stroke {
        Stroke::new(1.0, self.separator)
    }

    pub fn body(&self) -> FontId {
        FontId::new(self.font_size, self.faces.regular.clone())
    }

    pub fn sized(&self, scale: f32, bold: bool, italic: bool) -> FontId {
        FontId::new(self.font_size * scale, self.faces.pick(bold, italic))
    }

    /// Pushes this palette into the egui style. Cheap enough to call whenever
    /// the theme changes; not called per frame.
    pub fn apply(&self, ctx: &Context) {
        ctx.all_styles_mut(|style| self.apply_to(style));
    }

    fn apply_to(&self, style: &mut egui::Style) {
        style.visuals.dark_mode = self.text.r() > 0x80;
        style.visuals.panel_fill = Color32::TRANSPARENT;
        style.visuals.window_fill = self.surface;
        style.visuals.window_stroke = self.separator_stroke();
        style.visuals.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
        style.visuals.window_shadow = egui::epaint::Shadow::NONE;
        style.visuals.popup_shadow = egui::epaint::Shadow::NONE;
        style.visuals.override_text_color = Some(self.text);
        style.visuals.hyperlink_color = self.link;
        style.visuals.code_bg_color = self.code_bg;
        style.visuals.extreme_bg_color = self.code_bg;
        style.visuals.faint_bg_color = self.hover;
        style.visuals.selection.bg_fill = self.selection;
        style.visuals.selection.stroke = Stroke::new(1.0, self.accent);
        style.visuals.button_frame = false;
        style.visuals.indent_has_left_vline = false;

        for widget in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
            &mut style.visuals.widgets.open,
        ] {
            widget.bg_fill = Color32::TRANSPARENT;
            widget.weak_bg_fill = Color32::TRANSPARENT;
            widget.bg_stroke = Stroke::NONE;
            widget.fg_stroke = Stroke::new(1.0, self.text);
            widget.corner_radius = CornerRadius::same(3);
            widget.expansion = 0.0;
        }
        // Panel separators and `Separator` both read this stroke.
        style.visuals.widgets.noninteractive.bg_stroke = self.separator_stroke();
        style.visuals.widgets.hovered.bg_fill = self.hover;
        style.visuals.widgets.hovered.weak_bg_fill = self.hover;
        style.visuals.widgets.active.bg_fill = self.selection;
        style.visuals.widgets.active.weak_bg_fill = self.selection;

        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(6.0, 2.0);
        style.spacing.indent = 14.0;
        style.spacing.scroll.bar_width = 6.0;
        style.spacing.scroll.floating = true;
        style.spacing.interact_size.y = self.font_size + 8.0;

        let f = &self.faces;
        style.text_styles = [
            (egui::TextStyle::Body, self.body()),
            (egui::TextStyle::Button, self.body()),
            (egui::TextStyle::Monospace, self.body()),
            (
                egui::TextStyle::Small,
                FontId::new(self.font_size - 2.0, f.regular.clone()),
            ),
            (
                egui::TextStyle::Heading,
                FontId::new(self.font_size * 1.4, f.bold.clone()),
            ),
        ]
        .into();
    }
}

/// Corner radius of the undecorated window, in points.
pub const WINDOW_RADIUS: u8 = 10;

/// Loads the four JetBrains Mono Nerd Font faces into `ctx`.
///
/// Returns the faces that could not be found; Faust still runs, falling back to
/// the regular weight, so a partial install degrades instead of failing.
pub fn install_fonts(ctx: &Context) -> Vec<&'static str> {
    let mut defs = FontDefinitions::empty();
    let mut missing = Vec::new();
    let mut regular_key = None;

    for (key, face) in FACES {
        match find_face(face) {
            Some(path) => match std::fs::read(&path) {
                Ok(bytes) => {
                    defs.font_data
                        .insert(key.to_owned(), Arc::new(FontData::from_owned(bytes)));
                    if regular_key.is_none() {
                        regular_key = Some(key);
                    }
                }
                Err(err) => {
                    eprintln!("faust: cannot read {}: {err}", path.display());
                    missing.push(face);
                }
            },
            None => missing.push(face),
        }
    }

    // Without at least one face egui cannot lay out any text at all.
    let Some(fallback) = regular_key else {
        return FACES.iter().map(|(_, face)| *face).collect();
    };

    let key_of = |key: &'static str| {
        if defs.font_data.contains_key(key) {
            key
        } else {
            fallback
        }
    };
    let family = |key: &'static str| vec![key_of(key).to_owned()];

    defs.families
        .insert(FontFamily::Monospace, family("faust-regular"));
    defs.families
        .insert(FontFamily::Proportional, family("faust-regular"));
    for (key, _) in &FACES[1..] {
        defs.families
            .insert(FontFamily::Name((*key).into()), family(key));
    }

    ctx.set_fonts(defs);
    missing
}

/// Finds one face of the family, preferring an exact hit over a directory walk.
fn find_face(face: &str) -> Option<PathBuf> {
    let names = [
        format!("JetBrainsMonoNerdFont-{face}.ttf"),
        format!("JetBrainsMonoNerdFontMono-{face}.ttf"),
        format!("JetBrainsMonoNLNerdFont-{face}.ttf"),
        format!("JetBrainsMono-{face}.ttf"),
    ];

    if let Some(dir) = std::env::var_os("FAUST_FONT_DIR") {
        let dir = PathBuf::from(dir);
        if let Some(hit) = names.iter().map(|n| dir.join(n)).find(|p| p.is_file()) {
            return Some(hit);
        }
    }

    // Fast path: the layout used by Arch, Fedora and Debian font packages.
    for dir in [
        "/usr/share/fonts/TTF",
        "/usr/share/fonts/truetype/jetbrains",
    ] {
        if let Some(hit) = names
            .iter()
            .map(|n| PathBuf::from(dir).join(n))
            .find(|p| p.is_file())
        {
            return Some(hit);
        }
    }

    let home = std::env::var_os("HOME").map(PathBuf::from);
    for dir in FONT_DIRS {
        let root = if dir.starts_with('/') {
            PathBuf::from(dir)
        } else {
            match &home {
                Some(home) => home.join(dir),
                None => continue,
            }
        };
        if let Some(hit) = scan_dir(&root, &names, 4) {
            return Some(hit);
        }
    }
    None
}

/// Depth-limited search for any of `names` beneath `dir`.
fn scan_dir(dir: &PathBuf, names: &[String], depth: u32) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            subdirs.push(entry.path());
        } else if names.iter().any(|n| entry.file_name() == n.as_str()) {
            return Some(entry.path());
        }
    }
    subdirs.iter().find_map(|d| scan_dir(d, names, depth - 1))
}
