//! Painting: window chrome, tab strip, tree, preview and the palette overlay.

use std::path::{Path, PathBuf};
use std::time::Instant;

use egui::{vec2, Align, CornerRadius, Layout, Sense, Stroke, Ui};

use super::{name_of, open_externally, App, RankKey, RESIZE_GRIP};
use crate::config::TreeSide;
use crate::document::Status;
use crate::palette::{Command, Mode, Outcome};
use crate::render::{Action, Renderer};
use crate::theme::{self, Theme};

/// Height of one file-tree row, relative to the font size.
const ROW_SCALE: f32 = 1.6;
/// Widest comfortable measure for prose, in points.
const READING_WIDTH: f32 = 900.0;

impl eframe::App for App {
    /// Fully transparent: the window surface is painted by `ui`, so the
    /// compositor sees the alpha Faust actually wants.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);
        self.drain_filesystem(&ctx);
        self.drain_query(&ctx);
        self.drain_search();

        let theme = self.theme.clone();
        self.chrome(ui, &theme);

        let screen = ctx.viewport_rect();
        self.cfg.window = [screen.width(), screen.height()];

        self.tab_strip(ui, &theme);
        self.status_bar(ui, &theme);
        self.tree_panel(ui, &theme);
        self.preview(ui, &theme);
        self.palette_overlay(&ctx, &theme);

        self.autosave(&ctx);
        if self.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.cfg != self.saved {
            if let Err(err) = self.cfg.save() {
                eprintln!("faust: cannot save settings: {err}");
            }
        }
    }
}

impl App {
    /// Paints the window surface and, when undecorated, the resize handles.
    fn chrome(&mut self, ui: &mut Ui, theme: &Theme) {
        let screen = ui.ctx().viewport_rect();
        let radius = if self.cfg.decorations {
            CornerRadius::ZERO
        } else {
            CornerRadius::same(theme::WINDOW_RADIUS)
        };
        let painter = ui.painter();
        painter.rect_filled(screen, radius, theme.surface);
        painter.rect_stroke(
            screen,
            radius,
            Stroke::new(1.0, theme.separator),
            egui::StrokeKind::Inside,
        );

        if !self.cfg.decorations {
            self.resize_handles(ui, screen);
        }
    }

    fn resize_handles(&self, ui: &mut Ui, screen: egui::Rect) {
        use egui::ResizeDirection as Dir;
        let g = RESIZE_GRIP;
        let (x0, x1, y0, y1) = (screen.left(), screen.right(), screen.top(), screen.bottom());
        let zones = [
            (
                Dir::NorthWest,
                egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x0 + g, y0 + g)),
            ),
            (
                Dir::NorthEast,
                egui::Rect::from_min_max(egui::pos2(x1 - g, y0), egui::pos2(x1, y0 + g)),
            ),
            (
                Dir::SouthWest,
                egui::Rect::from_min_max(egui::pos2(x0, y1 - g), egui::pos2(x0 + g, y1)),
            ),
            (
                Dir::SouthEast,
                egui::Rect::from_min_max(egui::pos2(x1 - g, y1 - g), egui::pos2(x1, y1)),
            ),
            (
                Dir::North,
                egui::Rect::from_min_max(egui::pos2(x0 + g, y0), egui::pos2(x1 - g, y0 + g)),
            ),
            (
                Dir::South,
                egui::Rect::from_min_max(egui::pos2(x0 + g, y1 - g), egui::pos2(x1 - g, y1)),
            ),
            (
                Dir::West,
                egui::Rect::from_min_max(egui::pos2(x0, y0 + g), egui::pos2(x0 + g, y1 - g)),
            ),
            (
                Dir::East,
                egui::Rect::from_min_max(egui::pos2(x1 - g, y0 + g), egui::pos2(x1, y1 - g)),
            ),
        ];

        for (index, (direction, rect)) in zones.into_iter().enumerate() {
            let response = ui.interact(rect, egui::Id::new(("faust-resize", index)), Sense::drag());
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(cursor_for(direction));
            }
            if response.drag_started() {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
            }
        }
    }

    fn tab_strip(&mut self, ui: &mut Ui, theme: &Theme) {
        let height = theme.font_size * 2.1;
        egui::containers::Panel::top("faust-tabs")
            .exact_size(height)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 8,
                right: 8,
                top: 0,
                bottom: 0,
            }))
            .show_separator_line(true)
            .show(ui, |ui| {
                // Claim the strip for window dragging first; the tabs drawn
                // afterwards sit on top and keep their own clicks.
                let bar = ui.interact(
                    ui.max_rect(),
                    egui::Id::new("faust-titlebar"),
                    Sense::click_and_drag(),
                );
                if !self.cfg.decorations {
                    if bar.drag_started() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if bar.double_clicked() {
                        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                }

                if self.tabs.is_empty() {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(self.window_title())
                                .color(theme.text_faint)
                                .font(theme.body()),
                        );
                    });
                    return;
                }

                let mut activate = None;
                let mut close = None;
                egui::ScrollArea::horizontal()
                    .id_salt("faust-tabstrip")
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            for index in 0..self.tabs.len() {
                                match self.tab(ui, theme, index) {
                                    TabHit::None => {}
                                    TabHit::Activate => activate = Some(index),
                                    TabHit::Close => close = Some(index),
                                }
                            }
                        });
                    });

                if let Some(index) = activate {
                    self.active = index;
                }
                if let Some(index) = close {
                    self.close_tab(index);
                }
            });
    }

    fn tab(&self, ui: &mut Ui, theme: &Theme, index: usize) -> TabHit {
        let Some(tab) = self.tabs.get(index) else {
            return TabHit::None;
        };
        let active = index == self.active;
        let font = theme.body();
        let text_color = if active { theme.text } else { theme.text_dim };

        let label = ui
            .painter()
            .layout_no_wrap(tab.title.clone(), font.clone(), text_color);
        let width = label.size().x + theme.font_size * 2.6;
        let (rect, response) =
            ui.allocate_exact_size(vec2(width, ui.available_height()), Sense::click());

        let painter = ui.painter();
        if active {
            painter.rect_filled(rect, CornerRadius::ZERO, theme.surface_alt);
            painter.hline(
                rect.x_range(),
                rect.bottom() - 1.0,
                Stroke::new(2.0, theme.accent),
            );
        } else if response.hovered() {
            painter.rect_filled(rect, CornerRadius::ZERO, theme.hover);
        }
        painter.vline(
            rect.right(),
            rect.y_range().shrink(6.0),
            Stroke::new(1.0, theme.separator),
        );
        painter.galley(
            egui::pos2(rect.left() + 10.0, rect.center().y - label.size().y / 2.0),
            label,
            text_color,
        );

        // The close affordance only appears where it can be hit.
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - theme.font_size * 0.85, rect.center().y),
            vec2(theme.font_size, theme.font_size),
        );
        let close = ui.interact(
            close_rect,
            egui::Id::new(("faust-tab-close", index)),
            Sense::click(),
        );
        if active || response.hovered() || close.hovered() {
            ui.painter().text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                "\u{00d7}",
                theme.body(),
                if close.hovered() {
                    theme.text
                } else {
                    theme.text_faint
                },
            );
        }

        if close.clicked() || response.middle_clicked() {
            TabHit::Close
        } else if response.clicked() {
            TabHit::Activate
        } else {
            TabHit::None
        }
    }

    fn status_bar(&mut self, ui: &mut Ui, theme: &Theme) {
        let height = theme.font_size * 1.7;
        egui::containers::Panel::bottom("faust-status")
            .exact_size(height)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 10,
                right: 10,
                top: 0,
                bottom: 0,
            }))
            .show_separator_line(true)
            .show(ui, |ui| {
                let small = theme.sized(0.85, false, false);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if let Some(vault) = &self.vault {
                        ui.label(
                            egui::RichText::new(name_of(&vault.root))
                                .color(theme.accent)
                                .font(small.clone()),
                        );
                    }
                    if let Some(notice) = self.notice.clone() {
                        ui.label(
                            egui::RichText::new(notice)
                                .color(theme.text_dim)
                                .font(small.clone()),
                        );
                    } else if let Some(path) = self.active_path() {
                        let shown = self
                            .vault
                            .as_ref()
                            .and_then(|v| path.strip_prefix(&v.root).ok())
                            .unwrap_or(path);
                        ui.label(
                            egui::RichText::new(shown.display().to_string())
                                .color(theme.text_dim)
                                .font(small.clone()),
                        );
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("alt+space")
                                .color(theme.text_faint)
                                .font(small.clone()),
                        );
                        let count = if self.indexing {
                            "indexing…".to_owned()
                        } else {
                            let files = self.files.iter().filter(|e| !e.is_dir).count();
                            format!("{files} file{}", if files == 1 { "" } else { "s" })
                        };
                        ui.label(
                            egui::RichText::new(count)
                                .color(theme.text_faint)
                                .font(small),
                        );
                    });
                });
            });
    }

    fn tree_panel(&mut self, ui: &mut Ui, theme: &Theme) {
        if !self.cfg.tree_visible || self.vault.is_none() {
            return;
        }
        let frame = egui::Frame::new().inner_margin(egui::Margin {
            left: 4,
            right: 4,
            top: 4,
            bottom: 4,
        });
        let active = self.active_path().map(Path::to_path_buf);
        let mut hit = None;

        let size = self.cfg.tree_size;
        let rect = match self.cfg.tree_side {
            TreeSide::Left => {
                egui::containers::Panel::left("faust-tree")
                    .frame(frame)
                    .resizable(true)
                    .default_size(size)
                    .size_range(140.0..=640.0)
                    .show(ui, |ui| hit = self.tree_rows(ui, theme, active.as_deref()))
                    .response
                    .rect
            }
            TreeSide::Right => {
                egui::containers::Panel::right("faust-tree")
                    .frame(frame)
                    .resizable(true)
                    .default_size(size)
                    .size_range(140.0..=640.0)
                    .show(ui, |ui| hit = self.tree_rows(ui, theme, active.as_deref()))
                    .response
                    .rect
            }
            TreeSide::Bottom => {
                egui::containers::Panel::bottom("faust-tree")
                    .frame(frame)
                    .resizable(true)
                    .default_size(size)
                    .size_range(100.0..=600.0)
                    .show(ui, |ui| hit = self.tree_rows(ui, theme, active.as_deref()))
                    .response
                    .rect
            }
        };
        // Remember whatever the user dragged the splitter to.
        self.cfg.tree_size = if self.cfg.tree_side == TreeSide::Bottom {
            rect.height()
        } else {
            rect.width()
        };

        match hit {
            Some(TreeHit::Open(path)) => self.open_file(path),
            Some(TreeHit::Toggle(path)) => {
                if let Some(vault) = &mut self.vault {
                    vault.tree.toggle(&path);
                }
            }
            None => {}
        }
    }

    fn tree_rows(&mut self, ui: &mut Ui, theme: &Theme, active: Option<&Path>) -> Option<TreeHit> {
        let Some(vault) = &mut self.vault else {
            return None;
        };
        let row_height = theme.font_size * ROW_SCALE;
        let font = theme.body();
        let mut hit = None;

        egui::ScrollArea::both()
            .id_salt("faust-tree-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = ui.available_width();
                for row in vault.tree.rows() {
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(width, row_height), Sense::click());
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let is_active = active == Some(row.path.as_path());
                    let painter = ui.painter();
                    if is_active {
                        painter.rect_filled(rect, CornerRadius::same(3), theme.selection);
                    } else if response.hovered() {
                        painter.rect_filled(rect, CornerRadius::same(3), theme.hover);
                    }

                    let indent = 6.0 + row.depth as f32 * 12.0;
                    let marker = if row.is_dir {
                        if row.expanded {
                            "\u{25be}"
                        } else {
                            "\u{25b8}"
                        }
                    } else {
                        "\u{00b7}"
                    };
                    painter.text(
                        egui::pos2(rect.left() + indent, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        marker,
                        font.clone(),
                        theme.marker,
                    );
                    let color = if is_active {
                        theme.text
                    } else if row.is_dir {
                        theme.text_dim
                    } else {
                        theme.text
                    };
                    painter.text(
                        egui::pos2(rect.left() + indent + theme.font_size, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &row.name,
                        font.clone(),
                        color,
                    );

                    if response.clicked() {
                        hit = Some(if row.is_dir {
                            TreeHit::Toggle(row.path.clone())
                        } else {
                            TreeHit::Open(row.path.clone())
                        });
                    }
                }
            });
        hit
    }

    fn preview(&mut self, ui: &mut Ui, theme: &Theme) {
        let frame = egui::Frame::new().inner_margin(egui::Margin {
            left: 16,
            right: 16,
            top: 10,
            bottom: 10,
        });
        let mut actions = Vec::new();

        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            let Some(tab) = self.tabs.get(self.active) else {
                self.empty_state(ui, theme);
                return;
            };

            egui::ScrollArea::vertical()
                .id_salt(("faust-preview", &tab.path))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let width = ui.available_width().min(READING_WIDTH);
                    let margin = ((ui.available_width() - width) / 2.0).max(0.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(margin);
                        ui.vertical(|ui| {
                            ui.set_width(width);
                            match &tab.status {
                                Status::Ready => {
                                    actions = Renderer::new(theme).show(ui, &tab.body);
                                }
                                status => {
                                    ui.add_space(20.0);
                                    ui.label(
                                        egui::RichText::new(describe(status))
                                            .color(theme.text_faint)
                                            .font(theme.body()),
                                    );
                                }
                            }
                            ui.add_space(40.0);
                        });
                    });
                });
        });

        for action in actions {
            match action {
                Action::OpenNote(name) => self.open_note(&name),
                Action::OpenUrl(url) => open_url(&url),
            }
        }
    }

    fn empty_state(&self, ui: &mut Ui, theme: &Theme) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.32);
            let (title, hint) = if self.vault.is_some() {
                ("no file open", "click a note in the tree, or press ctrl+p")
            } else {
                ("no vault open", "press alt+space, then “Open Vault Folder”")
            };
            ui.label(
                egui::RichText::new(title)
                    .color(theme.text_dim)
                    .font(theme.sized(1.3, false, false)),
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(hint)
                    .color(theme.text_faint)
                    .font(theme.body()),
            );
        });
    }

    fn palette_overlay(&mut self, ctx: &egui::Context, theme: &Theme) {
        if !self.palette.open {
            return;
        }
        self.sync_palette();
        let outcome = self.palette.ui(ctx, theme);

        // The text field may have changed the query while drawing; make sure
        // the rows catch up on the next frame rather than at the next input.
        if self
            .ranked
            .as_ref()
            .is_none_or(|key| key.query != self.palette.query)
        {
            ctx.request_repaint();
        }

        if let Some(outcome) = outcome {
            match outcome {
                Outcome::Run(command) => {
                    // `OpenVault` and the search modes re-open the palette
                    // themselves; everything else closes it.
                    let reopens = matches!(
                        command,
                        Command::OpenVault | Command::FindFile | Command::SearchContent
                    );
                    if !reopens {
                        self.palette.close();
                    }
                    self.run(command, ctx);
                }
                Outcome::Open(path) => {
                    self.palette.close();
                    self.open_file(path);
                }
                Outcome::OpenVault(path) => {
                    self.palette.close();
                    self.open_vault(path, ctx);
                }
            }
        }
    }

    /// Rebuilds the palette's rows, but only when one of their inputs moved.
    ///
    /// Ranking is not free — the file finder scores every path in the vault and
    /// the folder picker reads a directory — and the palette is redrawn on
    /// every frame, so the result is cached against the inputs it came from.
    fn sync_palette(&mut self) {
        let key = RankKey {
            epoch: self.palette.epoch,
            mode: self.palette.mode,
            query: self.palette.query.clone(),
            files: self.files.len(),
            hits: self.hits.len(),
            searching: self.searching,
            indexing: self.indexing,
            browse: self.palette.browse.clone(),
        };
        if self.ranked.as_ref() == Some(&key) {
            return;
        }
        self.ranked = Some(key);

        match self.palette.mode {
            Mode::Command => {
                let recent = std::mem::take(&mut self.cfg.recent_commands);
                self.palette.rank_commands(&recent);
                self.cfg.recent_commands = recent;
            }
            Mode::File => {
                let indexing = self.indexing;
                let files = std::mem::take(&mut self.files);
                self.palette.rank_files(&files, indexing);
                self.files = files;
            }
            Mode::Content => {
                if self.palette.query != self.content_query {
                    self.content_query.clone_from(&self.palette.query);
                    self.pending_query = Some((self.palette.query.clone(), Instant::now()));
                }
                let hits = std::mem::take(&mut self.hits);
                self.palette.set_hits(&hits, self.searching);
                self.hits = hits;
            }
            Mode::Vault => self.palette.rank_folders(),
        }
    }
}

enum TabHit {
    None,
    Activate,
    Close,
}

enum TreeHit {
    Open(PathBuf),
    Toggle(PathBuf),
}

fn describe(status: &Status) -> String {
    match status {
        Status::Ready => String::new(),
        Status::Missing => "file no longer exists".to_owned(),
        Status::TooLarge(bytes) => format!("file is too large to preview ({} MiB)", bytes >> 20),
        Status::Binary => "binary file".to_owned(),
        Status::Error(err) => format!("cannot read file: {err}"),
    }
}

const fn cursor_for(direction: egui::ResizeDirection) -> egui::CursorIcon {
    use egui::{CursorIcon, ResizeDirection as Dir};
    match direction {
        Dir::North | Dir::South => CursorIcon::ResizeVertical,
        Dir::East | Dir::West => CursorIcon::ResizeHorizontal,
        Dir::NorthWest | Dir::SouthEast => CursorIcon::ResizeNwSe,
        Dir::NorthEast | Dir::SouthWest => CursorIcon::ResizeNeSw,
    }
}

/// Opens a URL, refusing schemes that would hand a local path to a browser.
fn open_url(url: &str) {
    if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
        open_externally(Path::new(url));
    }
}
