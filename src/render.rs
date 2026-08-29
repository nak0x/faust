//! Painting a parsed note with egui.

use egui::text::LayoutJob;
use egui::{
    vec2, Color32, CornerRadius, Frame, Label, Margin, Rect, Sense, Stroke, TextFormat, Ui,
};

use crate::markdown::{Block, Document, Link, List, Span, Table};
use crate::theme::Theme;

/// Relative font size of each heading level.
const HEADING_SCALE: [f32; 6] = [1.7, 1.42, 1.24, 1.12, 1.02, 0.96];
/// Bullet used at each nesting depth, cycling.
/// Every glyph here is present in JetBrains Mono Nerd Font.
const BULLETS: [&str; 3] = ["\u{2022}", "\u{25e6}", "\u{00b7}"];

/// Something the reader clicked.
pub enum Action {
    /// An Obsidian `[[wikilink]]`.
    OpenNote(String),
    /// An external URL.
    OpenUrl(String),
}

pub struct Renderer<'a> {
    theme: &'a Theme,
    actions: Vec<Action>,
    /// Distinguishes the egui ids of nested scroll areas and grids.
    next_id: usize,
}

impl<'a> Renderer<'a> {
    pub fn new(theme: &'a Theme) -> Self {
        Self {
            theme,
            actions: Vec::new(),
            next_id: 0,
        }
    }

    /// Paints `doc` and returns whatever the reader clicked.
    pub fn show(mut self, ui: &mut Ui, doc: &Document) -> Vec<Action> {
        self.blocks(ui, &doc.blocks, 0);
        self.actions
    }

    fn id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    fn blocks(&mut self, ui: &mut Ui, blocks: &[Block], depth: usize) {
        for (index, block) in blocks.iter().enumerate() {
            if index > 0 {
                ui.add_space(4.0);
            }
            self.block(ui, block, depth);
        }
    }

    fn block(&mut self, ui: &mut Ui, block: &Block, depth: usize) {
        let theme = self.theme;
        match block {
            Block::Heading { level, spans } => {
                let scale = HEADING_SCALE[(*level as usize - 1).min(5)];
                ui.add_space(if *level <= 2 { 10.0 } else { 6.0 });
                let marker = "#".repeat(*level as usize);
                self.spans(
                    ui,
                    Some((format!("{marker} "), theme.marker)),
                    spans,
                    scale,
                    theme.text,
                    true,
                );
                if *level <= 2 {
                    ui.add_space(4.0);
                    hline(ui, theme.separator);
                }
            }
            Block::Paragraph(spans) => {
                self.spans(ui, None, spans, 1.0, theme.text, false);
            }
            Block::Code { lang, text } => self.code(ui, lang.as_deref(), text),
            Block::Quote(blocks) => self.quote(ui, blocks, depth),
            Block::List(list) => self.list(ui, list, depth),
            Block::Table(table) => self.table(ui, table),
            Block::Meta(text) => self.meta(ui, text),
            Block::Rule => {
                ui.add_space(6.0);
                hline(ui, theme.separator);
                ui.add_space(6.0);
            }
        }
    }

    fn code(&mut self, ui: &mut Ui, lang: Option<&str>, text: &str) {
        let theme = self.theme;
        let id = self.id();
        Frame::new()
            .fill(theme.code_bg)
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin {
                left: 10,
                right: 10,
                top: 6,
                bottom: 6,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if let Some(lang) = lang {
                    ui.label(
                        egui::RichText::new(lang)
                            .color(theme.text_faint)
                            .font(theme.sized(0.85, false, false)),
                    );
                }
                egui::ScrollArea::horizontal()
                    .id_salt(("code", id))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            Label::new(
                                egui::RichText::new(text)
                                    .font(theme.body())
                                    .color(theme.text),
                            )
                            .wrap_mode(egui::TextWrapMode::Extend)
                            .selectable(true),
                        );
                    });
            });
    }

    fn quote(&mut self, ui: &mut Ui, blocks: &[Block], depth: usize) {
        let theme = self.theme;
        let response = Frame::new()
            .inner_margin(Margin {
                left: 12,
                right: 0,
                top: 2,
                bottom: 2,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.blocks(ui, blocks, depth + 1);
            });

        let rect = response.response.rect;
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top(), vec2(2.0, rect.height())),
            CornerRadius::same(1),
            theme.marker,
        );
    }

    fn list(&mut self, ui: &mut Ui, list: &List, depth: usize) {
        let theme = self.theme;
        for (index, item) in list.items.iter().enumerate() {
            let marker = match (item.task, list.start) {
                (Some(true), _) => "[x]".to_owned(),
                (Some(false), _) => "[ ]".to_owned(),
                (None, Some(start)) => format!("{}.", start + index as u64),
                (None, None) => BULLETS[depth % BULLETS.len()].to_owned(),
            };
            let marker_color = if item.task == Some(true) {
                theme.accent
            } else {
                theme.marker
            };

            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add_space(if depth == 0 { 2.0 } else { 10.0 });
                ui.label(
                    egui::RichText::new(marker)
                        .color(marker_color)
                        .font(theme.body()),
                );
                ui.vertical(|ui| {
                    self.blocks(ui, &item.blocks, depth + 1);
                });
            });
        }
    }

    fn table(&mut self, ui: &mut Ui, table: &Table) {
        let theme = self.theme;
        let columns = table
            .header
            .len()
            .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
        if columns == 0 {
            return;
        }
        let id = self.id();

        egui::ScrollArea::horizontal()
            .id_salt(("table", id))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new(("grid", id))
                    .num_columns(columns)
                    .spacing(vec2(16.0, 4.0))
                    .striped(true)
                    .show(ui, |ui| {
                        for cell in &table.header {
                            let job = self.job(None, cell, 1.0, theme.text, true);
                            ui.add(Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
                        }
                        ui.end_row();
                        for row in &table.rows {
                            for cell in row {
                                let job = self.job(None, cell, 1.0, theme.text, false);
                                ui.add(Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
                            }
                            ui.end_row();
                        }
                    });
            });
    }

    fn meta(&mut self, ui: &mut Ui, text: &str) {
        let theme = self.theme;
        Frame::new()
            .fill(theme.code_bg)
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin {
                left: 10,
                right: 10,
                top: 6,
                bottom: 6,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new(text)
                        .color(theme.text_dim)
                        .font(theme.sized(0.9, false, false)),
                );
            });
    }

    /// Lays out a run of inline spans, keeping links clickable.
    fn spans(
        &mut self,
        ui: &mut Ui,
        prefix: Option<(String, Color32)>,
        spans: &[Span],
        scale: f32,
        color: Color32,
        bold: bool,
    ) {
        // The common case is prose with no links, which lays out as one galley
        // and therefore wraps and justifies properly.
        if spans.iter().all(|span| span.link.is_none()) {
            let job = self.job(prefix, spans, scale, color, bold);
            ui.add(Label::new(job).selectable(true));
            return;
        }

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let mut prefix = prefix;
            let mut run: Vec<Span> = Vec::new();

            for span in spans {
                match &span.link {
                    None => run.push(span.clone()),
                    Some(link) => {
                        if !run.is_empty() || prefix.is_some() {
                            let job = self.job(prefix.take(), &run, scale, color, bold);
                            ui.add(Label::new(job).selectable(true));
                            run.clear();
                        }
                        let job = self.job(None, std::slice::from_ref(span), scale, color, bold);
                        let response = ui
                            .add(Label::new(job).sense(Sense::click()))
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if response.clicked() {
                            self.actions.push(match link {
                                Link::Note(name) => Action::OpenNote(name.clone()),
                                Link::Url(url) => Action::OpenUrl(url.clone()),
                            });
                        }
                    }
                }
            }
            if !run.is_empty() || prefix.is_some() {
                let job = self.job(prefix, &run, scale, color, bold);
                ui.add(Label::new(job).selectable(true));
            }
        });
    }

    fn job(
        &self,
        prefix: Option<(String, Color32)>,
        spans: &[Span],
        scale: f32,
        color: Color32,
        bold: bool,
    ) -> LayoutJob {
        let theme = self.theme;
        let mut job = LayoutJob::default();
        if let Some((text, prefix_color)) = prefix {
            job.append(
                &text,
                0.0,
                TextFormat {
                    font_id: theme.sized(scale, bold, false),
                    color: prefix_color,
                    ..Default::default()
                },
            );
        }
        for span in spans {
            let style = span.style;
            let mut format = TextFormat {
                font_id: theme.sized(scale, bold || style.bold, style.italic),
                color,
                ..Default::default()
            };
            if style.code {
                format.background = theme.code_bg;
                format.color = theme.text;
            }
            if style.tag {
                format.color = theme.accent;
            }
            if style.strike {
                format.strikethrough = Stroke::new(1.0, format.color);
            }
            if span.link.is_some() {
                format.color = theme.link;
                format.underline = Stroke::new(1.0, theme.link.gamma_multiply(0.6));
            }
            job.append(&span.text, 0.0, format);
        }
        job
    }
}

/// A one-pixel rule spanning the available width.
pub fn hline(ui: &mut Ui, color: Color32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, color));
}
