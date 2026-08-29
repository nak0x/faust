//! The command palette: the only place Faust exposes its commands.
//!
//! Four modes share one text field — commands, file names, file contents, and
//! a folder picker — so every action is reachable from a single keystroke and
//! nothing needs permanent chrome.

use std::path::{Path, PathBuf};

use egui::{Align2, Color32, CornerRadius, Frame, Key, Margin, Modifiers, Stroke, Ui};

use crate::fuzzy::Fuzzy;
use crate::index::{FileEntry, Hit};
use crate::render::hline;
use crate::theme::Theme;

/// Rows shown under the search field in command mode, as requested.
const COMMAND_ROWS: usize = 5;
/// Search modes list more, because a five-row result list is not a search.
const SEARCH_ROWS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    OpenVault,
    FindFile,
    SearchContent,
    ReloadFile,
    RevealInTree,
    CloseTab,
    CloseOthers,
    CloseAll,
    NextTab,
    PrevTab,
    ToggleTree,
    TreeLeft,
    TreeRight,
    TreeBottom,
    ToggleTheme,
    IncreaseFont,
    DecreaseFont,
    ToggleDecorations,
    CopyPath,
    OpenExternally,
    Quit,
}

impl Command {
    pub const ALL: [Self; 21] = [
        Self::FindFile,
        Self::SearchContent,
        Self::OpenVault,
        Self::ReloadFile,
        Self::RevealInTree,
        Self::CloseTab,
        Self::CloseOthers,
        Self::CloseAll,
        Self::NextTab,
        Self::PrevTab,
        Self::ToggleTree,
        Self::TreeLeft,
        Self::TreeRight,
        Self::TreeBottom,
        Self::ToggleTheme,
        Self::IncreaseFont,
        Self::DecreaseFont,
        Self::ToggleDecorations,
        Self::CopyPath,
        Self::OpenExternally,
        Self::Quit,
    ];

    /// Stable identifier, used for the persisted most-recently-used list.
    pub const fn id(self) -> &'static str {
        match self {
            Self::OpenVault => "open-vault",
            Self::FindFile => "find-file",
            Self::SearchContent => "search-content",
            Self::ReloadFile => "reload-file",
            Self::RevealInTree => "reveal-in-tree",
            Self::CloseTab => "close-tab",
            Self::CloseOthers => "close-others",
            Self::CloseAll => "close-all",
            Self::NextTab => "next-tab",
            Self::PrevTab => "prev-tab",
            Self::ToggleTree => "toggle-tree",
            Self::TreeLeft => "tree-left",
            Self::TreeRight => "tree-right",
            Self::TreeBottom => "tree-bottom",
            Self::ToggleTheme => "toggle-theme",
            Self::IncreaseFont => "increase-font",
            Self::DecreaseFont => "decrease-font",
            Self::ToggleDecorations => "toggle-decorations",
            Self::CopyPath => "copy-path",
            Self::OpenExternally => "open-externally",
            Self::Quit => "quit",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::OpenVault => "Open Vault Folder",
            Self::FindFile => "Find File or Folder by Name",
            Self::SearchContent => "Search File Contents",
            Self::ReloadFile => "Reload Current File",
            Self::RevealInTree => "Reveal Current File in Tree",
            Self::CloseTab => "Close Tab",
            Self::CloseOthers => "Close Other Tabs",
            Self::CloseAll => "Close All Tabs",
            Self::NextTab => "Next Tab",
            Self::PrevTab => "Previous Tab",
            Self::ToggleTree => "Toggle File Tree",
            Self::TreeLeft => "File Tree: Dock Left",
            Self::TreeRight => "File Tree: Dock Right",
            Self::TreeBottom => "File Tree: Dock Bottom",
            Self::ToggleTheme => "Toggle Light / Dark Theme",
            Self::IncreaseFont => "Increase Font Size",
            Self::DecreaseFont => "Decrease Font Size",
            Self::ToggleDecorations => "Toggle Window Decorations",
            Self::CopyPath => "Copy Current File Path",
            Self::OpenExternally => "Open Current File in Default App",
            Self::Quit => "Quit Faust",
        }
    }

    /// Shortcut shown on the right of the row, if there is one.
    pub const fn hint(self) -> &'static str {
        match self {
            Self::FindFile => "Ctrl P",
            Self::SearchContent => "Ctrl Shift F",
            Self::ReloadFile => "Ctrl R",
            Self::CloseTab => "Ctrl W",
            Self::NextTab => "Ctrl Tab",
            Self::PrevTab => "Ctrl Shift Tab",
            Self::ToggleTree => "Ctrl B",
            Self::IncreaseFont => "Ctrl +",
            Self::DecreaseFont => "Ctrl -",
            Self::Quit => "Ctrl Q",
            _ => "",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Command,
    File,
    Content,
    Vault,
}

impl Mode {
    const fn prompt(self) -> &'static str {
        match self {
            Self::Command => "",
            Self::File => "file ",
            Self::Content => "grep ",
            Self::Vault => "vault ",
        }
    }

    const fn hint(self) -> &'static str {
        match self {
            Self::Command => "Type a command…",
            Self::File => "Find a file or folder…",
            Self::Content => "Search inside notes…",
            Self::Vault => "Filter folders, Enter to descend…",
        }
    }
}

/// One row of the result list.
enum Row {
    Command(Command),
    Path {
        path: PathBuf,
        display: String,
        is_dir: bool,
    },
    Hit {
        path: PathBuf,
        display: String,
        line: u32,
        text: String,
    },
    /// "Open this folder as the vault" — always first in `Mode::Vault`.
    UseFolder(PathBuf),
    Folder {
        path: PathBuf,
        label: String,
    },
    /// Not selectable; explains an empty list.
    Note(String),
}

/// What the app should do about a chosen row.
pub enum Outcome {
    Run(Command),
    Open(PathBuf),
    OpenVault(PathBuf),
}

pub struct Palette {
    pub open: bool,
    pub mode: Mode,
    pub query: String,
    /// Folder shown in `Mode::Vault`.
    pub browse: PathBuf,
    selected: usize,
    rows: Vec<Row>,
    fuzzy: Fuzzy,
    /// Set when the palette opens, so the text field takes focus once.
    grab_focus: bool,
    /// True while a content search is still running.
    pub searching: bool,
    /// Bumped every time the palette opens. Callers that cache the row list
    /// key off this, because opening the palette clears the rows.
    pub epoch: u64,
}

impl Palette {
    pub fn new() -> Self {
        Self {
            open: false,
            mode: Mode::Command,
            query: String::new(),
            browse: PathBuf::new(),
            selected: 0,
            rows: Vec::new(),
            fuzzy: Fuzzy::new(),
            grab_focus: false,
            searching: false,
            epoch: 0,
        }
    }

    pub fn show(&mut self, mode: Mode) {
        self.epoch = self.epoch.wrapping_add(1);
        self.open = true;
        self.mode = mode;
        self.query.clear();
        self.selected = 0;
        self.rows.clear();
        self.grab_focus = true;
        self.searching = false;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.rows.clear();
        self.searching = false;
    }

    /// Ranks the built-in commands, most recently used first when idle.
    pub fn rank_commands(&mut self, recent: &[String]) {
        let pattern = Fuzzy::pattern(&self.query);
        if self.query.is_empty() {
            let mut ordered: Vec<Command> = recent
                .iter()
                .filter_map(|id| Command::from_id(id))
                .collect();
            for command in Command::ALL {
                if !ordered.contains(&command) {
                    ordered.push(command);
                }
            }
            self.rows = ordered.into_iter().map(Row::Command).collect();
        } else {
            let mut scored: Vec<(u32, Command)> = Command::ALL
                .into_iter()
                .filter_map(|command| {
                    let score = self.fuzzy.score(&pattern, command.title())?;
                    // A recently used command wins ties with an unused one.
                    let bonus = recent
                        .iter()
                        .position(|id| id == command.id())
                        .map_or(0, |rank| (recent.len() - rank) as u32);
                    Some((score + bonus, command))
                })
                .collect();
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            self.rows = scored.into_iter().map(|(_, c)| Row::Command(c)).collect();
        }
        self.clamp();
    }

    /// Ranks vault entries by name.
    pub fn rank_files(&mut self, entries: &[FileEntry], indexing: bool) {
        let pattern = Fuzzy::pattern(&self.query);
        let mut scored: Vec<(u32, &FileEntry)> = entries
            .iter()
            .filter_map(|entry| Some((self.fuzzy.score(&pattern, &entry.display)?, entry)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.display.cmp(&b.1.display)));
        scored.truncate(SEARCH_ROWS * 4);

        self.rows = scored
            .into_iter()
            .map(|(_, entry)| Row::Path {
                path: entry.path.clone(),
                display: entry.display.clone(),
                is_dir: entry.is_dir,
            })
            .collect();
        if self.rows.is_empty() {
            self.rows.push(Row::Note(if indexing {
                "indexing…".to_owned()
            } else {
                "no match".to_owned()
            }));
        }
        self.clamp();
    }

    /// Shows content-search results as they stream in.
    pub fn set_hits(&mut self, hits: &[Hit], searching: bool) {
        self.searching = searching;
        self.rows = hits
            .iter()
            .map(|hit| Row::Hit {
                path: hit.path.clone(),
                display: hit.display.clone(),
                line: hit.line,
                text: hit.text.clone(),
            })
            .collect();
        if self.rows.is_empty() {
            self.rows.push(Row::Note(
                if self.query.trim().is_empty() {
                    "type to search note contents"
                } else if searching {
                    "searching…"
                } else {
                    "no match"
                }
                .to_owned(),
            ));
        }
        self.clamp();
    }

    /// Lists the folders inside `browse`, filtered by the query.
    pub fn rank_folders(&mut self) {
        let pattern = Fuzzy::pattern(&self.query);
        let mut folders: Vec<(u32, PathBuf)> = std::fs::read_dir(&self.browse)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                // Hidden folders are noise in a folder picker, but `.config`
                // style paths still need to be reachable by typing the dot.
                if name.starts_with('.') && !self.query.starts_with('.') {
                    return None;
                }
                Some((self.fuzzy.score(&pattern, &name)?, entry.path()))
            })
            .collect();
        folders.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

        self.rows = vec![Row::UseFolder(self.browse.clone())];
        if let Some(parent) = self.browse.parent().map(Path::to_path_buf) {
            self.rows.push(Row::Folder {
                path: parent,
                label: "..".to_owned(),
            });
        }
        self.rows
            .extend(folders.into_iter().map(|(_, path)| Row::Folder {
                label: name_of(&path),
                path,
            }));
        self.clamp();
    }

    fn clamp(&mut self) {
        let limit = self.visible_rows();
        if self.selected >= limit {
            self.selected = limit.saturating_sub(1);
        }
    }

    fn visible_rows(&self) -> usize {
        let cap = if self.mode == Mode::Command {
            COMMAND_ROWS
        } else {
            SEARCH_ROWS
        };
        self.rows.len().min(cap)
    }

    /// Draws the palette and returns the chosen action, if any.
    pub fn ui(&mut self, ctx: &egui::Context, theme: &Theme) -> Option<Outcome> {
        if !self.open {
            return None;
        }

        // Consume navigation keys before the text field sees them, so Up/Down
        // move the selection instead of the caret.
        let (accept, up, down, cancel) = ctx.input_mut(|input| {
            (
                input.consume_key(Modifiers::NONE, Key::Enter),
                input.consume_key(Modifiers::NONE, Key::ArrowUp)
                    || input.consume_key(Modifiers::CTRL, Key::P),
                input.consume_key(Modifiers::NONE, Key::ArrowDown)
                    || input.consume_key(Modifiers::CTRL, Key::N)
                    || input.consume_key(Modifiers::NONE, Key::Tab),
                input.consume_key(Modifiers::NONE, Key::Escape),
            )
        });

        if cancel {
            self.close();
            return None;
        }
        let visible = self.visible_rows();
        if visible > 0 {
            if up {
                self.selected = (self.selected + visible - 1) % visible;
            }
            if down {
                self.selected = (self.selected + 1) % visible;
            }
        }

        let mut outcome = None;
        let width = (ctx.viewport_rect().width() * 0.62).clamp(360.0, 640.0);

        egui::Area::new(egui::Id::new("faust-palette"))
            .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(opaque(theme.surface))
                    .stroke(Stroke::new(1.0, theme.separator))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin {
                        left: 12,
                        right: 12,
                        top: 10,
                        bottom: 8,
                    })
                    .show(ui, |ui| {
                        ui.set_width(width);
                        self.field(ui, theme);
                        ui.add_space(6.0);
                        hline(ui, theme.separator);
                        ui.add_space(6.0);
                        outcome = self.list(ui, theme, accept);
                    });
            });

        outcome
    }

    fn field(&mut self, ui: &mut Ui, theme: &Theme) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let prompt = if self.mode == Mode::Vault {
                elide_left(&display_path(&self.browse), 44)
            } else {
                self.mode.prompt().to_owned()
            };
            ui.label(
                egui::RichText::new(if prompt.is_empty() {
                    "›".to_owned()
                } else {
                    prompt
                })
                .color(theme.accent)
                .font(theme.body()),
            );

            let response = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .desired_width(f32::INFINITY)
                    .frame(Frame::NONE)
                    .hint_text(egui::RichText::new(self.mode.hint()).color(theme.text_faint))
                    .font(egui::FontSelection::FontId(theme.body()))
                    .text_color(theme.text),
            );
            if self.grab_focus {
                response.request_focus();
                self.grab_focus = false;
            }
            if response.changed() {
                self.selected = 0;
            }
        });
    }

    fn list(&mut self, ui: &mut Ui, theme: &Theme, accept: bool) -> Option<Outcome> {
        let visible = self.visible_rows();
        let mut chosen = accept.then_some(self.selected);

        for index in 0..visible {
            let selected = index == self.selected;
            let response = self.row(ui, theme, index, selected);
            if response.clicked() {
                chosen = Some(index);
            }
        }
        if visible == 0 {
            ui.label(
                egui::RichText::new("no match")
                    .color(theme.text_faint)
                    .font(theme.body()),
            );
        }
        if self.searching {
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("searching…")
                    .color(theme.text_faint)
                    .font(theme.sized(0.85, false, false)),
            );
        }

        let index = chosen?;
        self.activate(index)
    }

    fn row(&self, ui: &mut Ui, theme: &Theme, index: usize, selected: bool) -> egui::Response {
        let height = theme.font_size * 1.75;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::click(),
        );

        let Some(row) = self.rows.get(index) else {
            return response;
        };
        let (icon, primary, secondary) = match row {
            Row::Command(command) => (
                "\u{203a}",
                command.title().to_owned(),
                command.hint().to_owned(),
            ),
            Row::Path {
                display, is_dir, ..
            } => (
                if *is_dir { "\u{25b8}" } else { "\u{2022}" },
                display.clone(),
                String::new(),
            ),
            Row::Hit {
                display,
                line,
                text,
                ..
            } => ("\u{2022}", text.clone(), format!("{display}:{line}")),
            Row::UseFolder(path) => (
                "\u{2713}",
                format!("Open \u{201c}{}\u{201d} as vault", name_of(path)),
                String::new(),
            ),
            Row::Folder { label, .. } => ("\u{25b8}", label.clone(), String::new()),
            Row::Note(text) => (" ", text.clone(), String::new()),
        };
        let selectable = !matches!(row, Row::Note(_));

        if !ui.is_rect_visible(rect) {
            return response;
        }

        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, CornerRadius::same(4), theme.selection);
        } else if response.hovered() && selectable {
            painter.rect_filled(rect, CornerRadius::same(4), theme.hover);
        }

        let pad = 8.0;
        let mut right = rect.right() - pad;
        if !secondary.is_empty() {
            let galley = painter.layout_no_wrap(
                secondary,
                theme.sized(0.85, false, false),
                theme.text_faint,
            );
            let width = galley.size().x.min(rect.width() * 0.45);
            let pos = egui::pos2(right - width, rect.center().y - galley.size().y / 2.0);
            painter.galley(pos, galley, theme.text_faint);
            right -= width + 12.0;
        }

        painter.text(
            egui::pos2(rect.left() + pad, rect.center().y),
            egui::Align2::LEFT_CENTER,
            icon,
            theme.body(),
            if selected { theme.accent } else { theme.marker },
        );

        let text_left = rect.left() + pad + theme.font_size * 1.4;
        let galley = truncated(
            ui.painter(),
            primary,
            theme.body(),
            if selectable {
                theme.text
            } else {
                theme.text_faint
            },
            (right - text_left).max(16.0),
        );
        ui.painter().galley(
            egui::pos2(text_left, rect.center().y - galley.size().y / 2.0),
            galley,
            theme.text,
        );

        if selectable {
            response.on_hover_cursor(egui::CursorIcon::PointingHand)
        } else {
            response
        }
    }

    fn activate(&mut self, index: usize) -> Option<Outcome> {
        match self.rows.get(index)? {
            Row::Command(command) => Some(Outcome::Run(*command)),
            Row::Path { path, .. } => Some(Outcome::Open(path.clone())),
            Row::Hit { path, .. } => Some(Outcome::Open(path.clone())),
            Row::UseFolder(path) => Some(Outcome::OpenVault(path.clone())),
            Row::Folder { path, .. } => {
                // Descending is handled here rather than by the app: it only
                // changes what the palette is showing.
                self.browse = path.clone();
                self.query.clear();
                self.selected = 0;
                self.rank_folders();
                None
            }
            Row::Note(_) => None,
        }
    }
}

/// Lays out one line, eliding with `…` instead of wrapping or overflowing.
fn truncated(
    painter: &egui::Painter,
    text: String,
    font: egui::FontId,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text,
        egui::TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap.max_width = max_width;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('\u{2026}');
    painter.layout_job(job)
}

/// Palette rows must stay legible over whatever is behind the window.
fn opaque(color: Color32) -> Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, a.saturating_add(48))
}

/// Keeps the tail of an over-long path, which is the part that identifies it.
fn elide_left(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    let skip = count - max_chars + 1;
    format!("\u{2026}{}", text.chars().skip(skip).collect::<String>())
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Shortens `$HOME` to `~` so the picker stays readable.
fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    let Some(home) = std::env::var_os("HOME") else {
        return text;
    };
    let home = home.to_string_lossy();
    match text.strip_prefix(home.as_ref()) {
        Some("") => "~".to_owned(),
        Some(rest) => format!("~{rest}"),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(palette: &Palette) -> Vec<&str> {
        palette
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::Command(command) => Some(command.title()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_command_has_a_unique_id() {
        let mut ids: Vec<&str> = Command::ALL.iter().map(|c| c.id()).collect();
        ids.sort_unstable();
        let unique = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), unique, "duplicate command id");
    }

    #[test]
    fn an_idle_palette_lists_recent_commands_first() {
        let mut palette = Palette::new();
        palette.show(Mode::Command);
        palette.rank_commands(&["quit".to_owned(), "toggle-theme".to_owned()]);
        assert_eq!(
            &titles(&palette)[..2],
            &[Command::Quit.title(), Command::ToggleTheme.title()]
        );
        // Every command stays reachable by scrolling, not just the recent ones.
        assert_eq!(palette.rows.len(), Command::ALL.len());
    }

    #[test]
    fn only_five_command_rows_are_offered() {
        let mut palette = Palette::new();
        palette.show(Mode::Command);
        palette.rank_commands(&[]);
        assert_eq!(palette.visible_rows(), COMMAND_ROWS);
    }

    #[test]
    fn typing_ranks_the_best_command_first() {
        let mut palette = Palette::new();
        palette.show(Mode::Command);
        palette.query = "clostab".to_owned();
        palette.rank_commands(&[]);
        assert_eq!(titles(&palette).first(), Some(&Command::CloseTab.title()));
    }

    #[test]
    fn opening_the_palette_moves_the_epoch() {
        let mut palette = Palette::new();
        let before = palette.epoch;
        palette.show(Mode::File);
        assert_ne!(palette.epoch, before, "callers cache their rows on this");
    }

    #[test]
    fn file_ranking_prefers_the_closer_name() {
        let entry = |display: &str| FileEntry {
            path: PathBuf::from(display),
            display: display.to_owned(),
            is_dir: false,
        };
        let entries = vec![
            entry("a/b/c/unrelated.md"),
            entry("notes/design.md"),
            entry("d/e/s/i/g/n.md"),
        ];
        let mut palette = Palette::new();
        palette.show(Mode::File);
        palette.query = "design".to_owned();
        palette.rank_files(&entries, false);

        let first = match &palette.rows[0] {
            Row::Path { display, .. } => display.as_str(),
            _ => panic!("expected a path row"),
        };
        assert_eq!(first, "notes/design.md");
        assert!(!palette.rows.iter().any(|row| matches!(
            row,
            Row::Path { display, .. } if display.contains("unrelated")
        )));
    }

    #[test]
    fn an_empty_result_explains_itself_and_is_not_selectable() {
        let mut palette = Palette::new();
        palette.show(Mode::File);
        palette.query = "zzzz".to_owned();
        palette.rank_files(&[], true);
        assert!(matches!(&palette.rows[0], Row::Note(text) if text.contains("indexing")));
        assert!(palette.activate(0).is_none());
    }

    #[test]
    fn long_paths_are_elided_from_the_left() {
        assert_eq!(elide_left("/short", 44), "/short");
        let long = "/a/very/long/path/that/keeps/going/and/going/and/going/further";
        let out = elide_left(long, 20);
        assert_eq!(out.chars().count(), 20);
        assert!(out.starts_with('\u{2026}'));
        assert!(long.ends_with(&out[out.char_indices().nth(1).unwrap().0..]));
    }
}
