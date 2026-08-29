//! Persisted preferences and session state.
//!
//! Faust deliberately avoids a serialization framework: the settings file is a
//! flat `key = value` list, which keeps the dependency tree (and therefore both
//! binary size and startup cost) small. Keys the running binary does not
//! recognise are kept and written back untouched, so an older Faust can never
//! silently discard a newer one's settings.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Where the file tree is docked.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TreeSide {
    #[default]
    Left,
    Right,
    Bottom,
}

impl TreeSide {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Bottom => "bottom",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }
}

/// Which of the two built-in palettes to paint with.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    /// Dark ink on a translucent `#e1e1e1` surface. This is the default.
    #[default]
    Light,
    /// Light ink on a translucent dark surface, for dark wallpapers.
    Dark,
}

impl ThemeMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub const fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }
}

/// Everything Faust remembers between runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Vault reopened on the next launch.
    pub vault: Option<PathBuf>,
    pub tree_side: TreeSide,
    /// Width when docked left/right, height when docked at the bottom.
    pub tree_size: f32,
    pub tree_visible: bool,
    pub theme: ThemeMode,
    /// Alpha of the window surface, `0.0..=1.0`.
    pub opacity: f32,
    /// Blur radius in pixels. Faust cannot blur what is behind its own window,
    /// so this is the value handed to the compositor by `install.sh`.
    pub blur: f32,
    pub font_size: f32,
    pub decorations: bool,
    pub window: [f32; 2],
    /// Command palette history, most recently used first.
    pub recent_commands: Vec<String>,
    /// Keys written by some other version of Faust, preserved verbatim.
    unknown: Vec<(String, String)>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            vault: None,
            tree_side: TreeSide::default(),
            tree_size: 260.0,
            tree_visible: true,
            theme: ThemeMode::default(),
            opacity: 0.82,
            blur: 8.0,
            font_size: 14.0,
            decorations: false,
            window: [1100.0, 720.0],
            recent_commands: Vec::new(),
            unknown: Vec::new(),
        }
    }
}

impl Config {
    /// Reads the settings file, falling back to defaults for anything missing.
    ///
    /// A corrupt or unreadable file is never fatal: Faust starts with defaults
    /// and reports the problem on stderr.
    pub fn load() -> Self {
        let path = Self::path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                if err.kind() != io::ErrorKind::NotFound {
                    eprintln!("faust: cannot read {}: {err}", path.display());
                }
                return Self::default();
            }
        };

        let mut cfg = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            cfg.set(key.trim(), value.trim());
        }
        cfg
    }

    fn set(&mut self, key: &str, value: &str) {
        match key {
            "vault" => self.vault = (!value.is_empty()).then(|| PathBuf::from(value)),
            "tree_side" => self.tree_side = TreeSide::parse(value).unwrap_or_default(),
            "tree_size" => self.tree_size = clamp_parse(value, 120.0, 900.0, self.tree_size),
            "tree_visible" => self.tree_visible = value == "true",
            "theme" => self.theme = ThemeMode::parse(value).unwrap_or_default(),
            "opacity" => self.opacity = clamp_parse(value, 0.05, 1.0, self.opacity),
            "blur" => self.blur = clamp_parse(value, 0.0, 64.0, self.blur),
            "font_size" => self.font_size = clamp_parse(value, 8.0, 32.0, self.font_size),
            "decorations" => self.decorations = value == "true",
            "window_width" => self.window[0] = clamp_parse(value, 320.0, 16384.0, self.window[0]),
            "window_height" => self.window[1] = clamp_parse(value, 240.0, 16384.0, self.window[1]),
            "recent_commands" => {
                self.recent_commands = value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            _ => self.unknown.push((key.to_owned(), value.to_owned())),
        }
    }

    /// Writes the settings file, creating the config directory if needed.
    pub fn save(&self) -> io::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut out = String::with_capacity(512);
        out.push_str(
            "# Faust settings. Written as you change them; edits here load at next launch.\n",
        );
        push(
            &mut out,
            "vault",
            self.vault.as_deref().map_or("", path_str),
        );
        push(&mut out, "tree_side", self.tree_side.label());
        push(&mut out, "tree_size", &format!("{:.0}", self.tree_size));
        push(&mut out, "tree_visible", bool_str(self.tree_visible));
        push(&mut out, "theme", self.theme.label());
        push(&mut out, "opacity", &format!("{:.2}", self.opacity));
        push(&mut out, "blur", &format!("{:.0}", self.blur));
        push(&mut out, "font_size", &format!("{:.0}", self.font_size));
        push(&mut out, "decorations", bool_str(self.decorations));
        push(&mut out, "window_width", &format!("{:.0}", self.window[0]));
        push(&mut out, "window_height", &format!("{:.0}", self.window[1]));
        push(&mut out, "recent_commands", &self.recent_commands.join(","));
        for (key, value) in &self.unknown {
            push(&mut out, key, value);
        }

        // Write-then-rename so a crash mid-save cannot truncate the settings.
        let tmp = path.with_extension("conf.tmp");
        fs::write(&tmp, out)?;
        fs::rename(&tmp, &path)
    }

    /// `$XDG_CONFIG_HOME/faust/faust.conf`, or `~/.config/faust/faust.conf`.
    pub fn path() -> PathBuf {
        Self::dir().join("faust.conf")
    }

    pub fn dir() -> PathBuf {
        if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
            PathBuf::from(dir).join("faust")
        } else if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(".config").join("faust")
        } else {
            PathBuf::from(".faust")
        }
    }

    /// Moves `id` to the front of the palette's most-recently-used list.
    pub fn touch_command(&mut self, id: &str) {
        self.recent_commands.retain(|c| c != id);
        self.recent_commands.insert(0, id.to_owned());
        self.recent_commands.truncate(16);
    }
}

fn push(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(" = ");
    out.push_str(value);
    out.push('\n');
}

const fn bool_str(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

/// Lossy but adequate: Faust only ever stores paths it was itself given.
fn path_str(path: &Path) -> &str {
    path.to_str().unwrap_or("")
}

fn clamp_parse(raw: &str, min: f32, max: f32, fallback: f32) -> f32 {
    raw.parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .map_or(fallback, |v| v.clamp(min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_keys_survive_a_round_trip() {
        let mut cfg = Config::default();
        cfg.set("future_option", "42");
        assert_eq!(cfg.unknown, vec![("future_option".into(), "42".into())]);
    }

    #[test]
    fn out_of_range_values_fall_back() {
        let mut cfg = Config::default();
        cfg.set("opacity", "9999");
        assert_eq!(cfg.opacity, 1.0);
        cfg.set("font_size", "not-a-number");
        assert_eq!(cfg.font_size, Config::default().font_size);
    }

    #[test]
    fn recent_commands_deduplicate_and_move_to_front() {
        let mut cfg = Config::default();
        cfg.touch_command("a");
        cfg.touch_command("b");
        cfg.touch_command("a");
        assert_eq!(cfg.recent_commands, vec!["a", "b"]);
    }
}
