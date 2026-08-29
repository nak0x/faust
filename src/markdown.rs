//! Markdown parsing into a small display tree.
//!
//! Faust parses a note once per change and renders the resulting tree every
//! frame. Re-parsing at 60 Hz would be wasteful; the tree below is deliberately
//! shallow and owns its text so rendering never touches the source buffer.

use pulldown_cmark::{
    CodeBlockKind, Event, HeadingLevel, LinkType, MetadataBlockKind, Options, Parser, Tag, TagEnd,
};

/// A parsed note.
#[derive(Debug, Default, Clone)]
pub struct Document {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone)]
pub enum Block {
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph(Vec<Span>),
    Code {
        lang: Option<String>,
        text: String,
    },
    Quote(Vec<Block>),
    List(List),
    Table(Table),
    /// A YAML front-matter block, kept verbatim.
    Meta(String),
    Rule,
}

#[derive(Debug, Clone)]
pub struct List {
    /// `Some(n)` for an ordered list starting at `n`.
    pub start: Option<u64>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub struct Item {
    /// `Some(done)` when the item carries a `- [ ]` marker.
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Default, Clone)]
pub struct Table {
    pub header: Vec<Vec<Span>>,
    pub rows: Vec<Vec<Vec<Span>>>,
}

/// A run of text sharing one set of inline marks.
#[derive(Debug, Clone)]
pub struct Span {
    pub text: String,
    pub style: Style,
    pub link: Option<Link>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    /// An Obsidian `#tag`.
    pub tag: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// An ordinary URL, opened with the desktop handler.
    Url(String),
    /// An Obsidian `[[wikilink]]`, resolved inside the vault.
    Note(String),
}

/// Parses Markdown into a display tree.
pub fn parse(text: &str) -> Document {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;

    let mut builder = Builder::default();
    for event in Parser::new_ext(text, options) {
        builder.event(event);
    }
    builder.finish()
}

/// Wraps arbitrary text as a single code block, for files Faust will not render
/// as Markdown (source files, plain text, and anything it failed to decode).
pub fn as_code(lang: Option<String>, text: String) -> Document {
    Document {
        blocks: vec![Block::Code { lang, text }],
    }
}

/// A block container currently being filled.
enum Frame {
    Blocks(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Item {
        task: Option<bool>,
        blocks: Vec<Block>,
    },
}

/// Where completed inline spans should be delivered.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Inline {
    #[default]
    None,
    Paragraph,
    Heading(u8),
    TableCell,
}

#[derive(Default)]
struct Builder {
    stack: Vec<Frame>,
    spans: Vec<Span>,
    inline: InlineState,
    table: Option<TableState>,
    code: Option<CodeState>,
    meta: Option<String>,
}

#[derive(Default)]
struct InlineState {
    target: Option<Inline>,
    style: Style,
    link: Option<Link>,
    /// Nesting depth of each mark, so `**a *b* c**` closes correctly.
    bold: u32,
    italic: u32,
    strike: u32,
}

struct TableState {
    table: Table,
    row: Vec<Vec<Span>>,
    in_header: bool,
}

struct CodeState {
    lang: Option<String>,
    text: String,
}

impl Builder {
    fn finish(mut self) -> Document {
        self.flush_inline();
        // Unclosed tags at EOF: unwind whatever is still open.
        while self.stack.len() > 1 {
            self.close_frame();
        }
        match self.stack.pop() {
            Some(Frame::Blocks(blocks)) => Document { blocks },
            _ => Document::default(),
        }
    }

    /// The innermost frame that can hold blocks, creating a root if needed.
    fn blocks_mut(&mut self) -> &mut Vec<Block> {
        let found = self
            .stack
            .iter()
            .rposition(|frame| matches!(frame, Frame::Blocks(_) | Frame::Item { .. }));
        let index = match found {
            Some(index) => index,
            None => {
                self.stack.push(Frame::Blocks(Vec::new()));
                self.stack.len() - 1
            }
        };
        match &mut self.stack[index] {
            Frame::Blocks(blocks) | Frame::Item { blocks, .. } => blocks,
            // `rposition` only ever selects one of the two arms above.
            Frame::List { .. } => unreachable!("selected frame holds blocks"),
        }
    }

    fn push_block(&mut self, block: Block) {
        self.blocks_mut().push(block);
    }

    /// Emits inline text that no explicit paragraph will claim.
    ///
    /// A *tight* list — `- one` with no blank lines — carries its text
    /// directly inside the item, with no `Paragraph` tag to close it. Without
    /// this, that text would sit in the buffer and surface inside whichever
    /// later block happened to flush first.
    fn flush_inline(&mut self) {
        if self.spans.is_empty() || self.inline.target == Some(Inline::TableCell) {
            return;
        }
        let spans = self.take_spans();
        if spans.is_empty() {
            return;
        }
        match self.inline.target {
            Some(Inline::Heading(level)) => self.push_block(Block::Heading { level, spans }),
            _ => self.push_block(Block::Paragraph(spans)),
        }
        self.inline.target = None;
    }

    /// Pops one frame and folds it into its parent.
    fn close_frame(&mut self) {
        match self.stack.pop() {
            Some(Frame::Blocks(blocks)) => self.push_block(Block::Quote(blocks)),
            Some(Frame::List { start, items }) => {
                self.push_block(Block::List(List { start, items }))
            }
            Some(Frame::Item { task, blocks }) => {
                if let Some(Frame::List { items, .. }) = self.stack.last_mut() {
                    items.push(Item { task, blocks });
                }
            }
            None => {}
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(text) => {
                let saved = self.inline.style;
                self.inline.style.code = true;
                self.raw(&text);
                self.inline.style = saved;
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                let saved = self.inline.style;
                self.inline.style.code = true;
                self.raw(&text);
                self.inline.style = saved;
            }
            Event::SoftBreak => self.raw(" "),
            Event::HardBreak => self.raw("\n"),
            Event::Rule => {
                self.flush_inline();
                self.push_block(Block::Rule);
            }
            Event::TaskListMarker(done) => {
                if let Some(Frame::Item { task, .. }) = self.stack.last_mut() {
                    *task = Some(done);
                }
            }
            Event::FootnoteReference(name) => self.raw(&format!("[^{name}]")),
            // Raw HTML is shown as-is rather than silently dropped, so a note
            // that leans on HTML is still legible.
            Event::Html(html) => {
                self.flush_inline();
                self.push_block(Block::Code {
                    lang: Some("html".into()),
                    text: html.trim_end().to_owned(),
                });
            }
            Event::InlineHtml(html) => {
                let saved = self.inline.style;
                self.inline.style.code = true;
                self.raw(&html);
                self.inline.style = saved;
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        if opens_block(&tag) {
            self.flush_inline();
        }
        match tag {
            Tag::Paragraph => self.inline.target = Some(Inline::Paragraph),
            Tag::Heading { level, .. } => {
                self.inline.target = Some(Inline::Heading(heading_level(level)));
            }
            Tag::BlockQuote(_) => self.stack.push(Frame::Blocks(Vec::new())),
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        let lang = info.split_whitespace().next().unwrap_or("");
                        (!lang.is_empty()).then(|| lang.to_owned())
                    }
                    CodeBlockKind::Indented => None,
                };
                self.code = Some(CodeState {
                    lang,
                    text: String::new(),
                });
            }
            Tag::List(start) => self.stack.push(Frame::List {
                start,
                items: Vec::new(),
            }),
            Tag::Item => self.stack.push(Frame::Item {
                task: None,
                blocks: Vec::new(),
            }),
            Tag::Table(_) => {
                self.table = Some(TableState {
                    table: Table::default(),
                    row: Vec::new(),
                    in_header: false,
                });
            }
            Tag::TableHead => {
                if let Some(state) = &mut self.table {
                    state.in_header = true;
                }
            }
            Tag::TableRow => {}
            Tag::TableCell => {
                self.inline.target = Some(Inline::TableCell);
                self.spans.clear();
            }
            Tag::Emphasis => self.inline.italic += 1,
            Tag::Strong => self.inline.bold += 1,
            Tag::Strikethrough => self.inline.strike += 1,
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => {
                self.inline.link = Some(match link_type {
                    LinkType::WikiLink { .. } => Link::Note(dest_url.into_string()),
                    _ => Link::Url(dest_url.into_string()),
                });
            }
            Tag::Image { dest_url, .. } => {
                // Faust ships without an image decoder on purpose; a labelled
                // placeholder keeps the note readable and memory flat.
                self.raw(&format!("[image: {dest_url}]"));
            }
            Tag::MetadataBlock(MetadataBlockKind::YamlStyle | MetadataBlockKind::PlusesStyle) => {
                self.meta = Some(String::new());
            }
            Tag::FootnoteDefinition(name) => {
                self.stack.push(Frame::Blocks(Vec::new()));
                self.raw(&format!("[^{name}]: "));
            }
            Tag::HtmlBlock
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript => {}
        }
        self.sync_style();
    }

    fn end(&mut self, tag: TagEnd) {
        if matches!(
            tag,
            TagEnd::Item
                | TagEnd::List(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::FootnoteDefinition
                | TagEnd::DefinitionListTitle
                | TagEnd::DefinitionListDefinition
        ) {
            self.flush_inline();
        }
        match tag {
            TagEnd::Paragraph => {
                let spans = self.take_spans();
                if !spans.is_empty() {
                    self.push_block(Block::Paragraph(spans));
                }
                self.inline.target = None;
            }
            TagEnd::Heading(level) => {
                let spans = self.take_spans();
                if !spans.is_empty() {
                    self.push_block(Block::Heading {
                        level: heading_level(level),
                        spans,
                    });
                }
                self.inline.target = None;
            }
            TagEnd::BlockQuote(_) | TagEnd::FootnoteDefinition => self.close_frame(),
            TagEnd::CodeBlock => {
                if let Some(code) = self.code.take() {
                    self.push_block(Block::Code {
                        lang: code.lang,
                        text: code.text.trim_end().to_owned(),
                    });
                }
            }
            TagEnd::List(_) | TagEnd::Item => self.close_frame(),
            TagEnd::Table => {
                if let Some(state) = self.table.take() {
                    self.push_block(Block::Table(state.table));
                }
            }
            TagEnd::TableHead => {
                if let Some(state) = &mut self.table {
                    state.in_header = false;
                    state.table.header = std::mem::take(&mut state.row);
                }
            }
            TagEnd::TableRow => {
                if let Some(state) = &mut self.table {
                    let row = std::mem::take(&mut state.row);
                    state.table.rows.push(row);
                }
            }
            TagEnd::TableCell => {
                let spans = std::mem::take(&mut self.spans);
                if let Some(state) = &mut self.table {
                    state.row.push(spans);
                }
            }
            TagEnd::Emphasis => self.inline.italic = self.inline.italic.saturating_sub(1),
            TagEnd::Strong => self.inline.bold = self.inline.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.inline.strike = self.inline.strike.saturating_sub(1),
            TagEnd::Link | TagEnd::Image => self.inline.link = None,
            TagEnd::MetadataBlock(_) => {
                if let Some(meta) = self.meta.take() {
                    let meta = meta.trim_end().to_owned();
                    if !meta.is_empty() {
                        self.push_block(Block::Meta(meta));
                    }
                }
            }
            TagEnd::HtmlBlock
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Superscript
            | TagEnd::Subscript => {}
        }
        self.sync_style();
    }

    fn sync_style(&mut self) {
        self.inline.style.bold = self.inline.bold > 0;
        self.inline.style.italic = self.inline.italic > 0;
        self.inline.style.strike = self.inline.strike > 0;
    }

    fn take_spans(&mut self) -> Vec<Span> {
        let mut spans = std::mem::take(&mut self.spans);
        if let Some(last) = spans.last_mut() {
            let trimmed = last.text.trim_end();
            if trimmed.len() != last.text.len() {
                last.text.truncate(trimmed.len());
            }
        }
        spans.retain(|span| !span.text.is_empty());
        spans
    }

    /// Text from the source, scanned for Obsidian tags.
    fn text(&mut self, text: &str) {
        if let Some(code) = &mut self.code {
            code.text.push_str(text);
            return;
        }
        if let Some(meta) = &mut self.meta {
            meta.push_str(text);
            return;
        }
        if self.inline.style.code || self.inline.link.is_some() {
            self.raw(text);
            return;
        }

        let mut rest = text;
        let mut at_boundary = self
            .spans
            .last()
            .is_none_or(|s| s.text.ends_with(char::is_whitespace));
        while let Some(hash) = rest.find('#') {
            let before = &rest[..hash];
            let boundary = if hash == 0 {
                at_boundary
            } else {
                before.ends_with(char::is_whitespace)
            };
            let tag_body = &rest[hash + 1..];
            let len = tag_len(tag_body);
            if !boundary || len == 0 {
                self.raw(&rest[..hash + 1]);
                rest = tag_body;
                at_boundary = false;
                continue;
            }
            self.raw(before);
            let saved = self.inline.style;
            self.inline.style.tag = true;
            self.raw(&rest[hash..hash + 1 + len]);
            self.inline.style = saved;
            rest = &tag_body[len..];
            at_boundary = false;
        }
        self.raw(rest);
    }

    /// Text taken literally, with no tag scanning.
    fn raw(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(code) = &mut self.code {
            code.text.push_str(text);
            return;
        }
        if let Some(meta) = &mut self.meta {
            meta.push_str(text);
            return;
        }
        if self.inline.target.is_none() {
            // Text outside any block (rare, but valid) starts a paragraph.
            self.inline.target = Some(Inline::Paragraph);
        }

        // Merge into the previous run when the marks match, which keeps the
        // span count close to the number of visually distinct runs.
        if let Some(last) = self.spans.last_mut() {
            if last.style == self.inline.style && last.link == self.inline.link {
                last.text.push_str(text);
                return;
            }
        }
        self.spans.push(Span {
            text: text.to_owned(),
            style: self.inline.style,
            link: self.inline.link.clone(),
        });
    }
}

/// Whether a tag begins a new block, and therefore ends any pending inline run.
const fn opens_block(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::List(_)
            | Tag::Item
            | Tag::Table(_)
            | Tag::MetadataBlock(_)
            | Tag::FootnoteDefinition(_)
            | Tag::HtmlBlock
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
    )
}

/// Length of a valid `#tag` body, or 0 if this `#` does not start a tag.
fn tag_len(rest: &str) -> usize {
    let mut len = 0;
    let mut has_alpha = false;
    for ch in rest.chars() {
        if ch.is_alphanumeric() {
            has_alpha |= ch.is_alphabetic();
            len += ch.len_utf8();
        } else if matches!(ch, '_' | '-' | '/') && len > 0 {
            len += ch.len_utf8();
        } else {
            break;
        }
    }
    // `#1` is a number, not a tag; Obsidian requires at least one letter.
    if has_alpha {
        len
    } else {
        0
    }
}

const fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn headings_and_paragraphs() {
        let doc = parse("# Title\n\nHello *world*.\n");
        assert_eq!(doc.blocks.len(), 2);
        let Block::Heading { level, spans } = &doc.blocks[0] else {
            panic!("expected a heading, got {:?}", doc.blocks[0]);
        };
        assert_eq!(*level, 1);
        assert_eq!(text_of(spans), "Title");

        let Block::Paragraph(spans) = &doc.blocks[1] else {
            panic!("expected a paragraph");
        };
        assert_eq!(text_of(spans), "Hello world.");
        assert!(spans.iter().any(|s| s.style.italic && s.text == "world"));
    }

    #[test]
    fn wikilinks_become_note_links() {
        let doc = parse("See [[Some Note]] and <https://example.com>.");
        let Block::Paragraph(spans) = &doc.blocks[0] else {
            panic!("expected a paragraph");
        };
        assert!(spans
            .iter()
            .any(|s| s.link == Some(Link::Note("Some Note".into()))));
        assert!(spans
            .iter()
            .any(|s| matches!(&s.link, Some(Link::Url(u)) if u.contains("example.com"))));
    }

    #[test]
    fn tags_are_marked_but_numbers_are_not() {
        let doc = parse("A #project/alpha and issue #42 here.");
        let Block::Paragraph(spans) = &doc.blocks[0] else {
            panic!("expected a paragraph");
        };
        assert!(spans
            .iter()
            .any(|s| s.style.tag && s.text == "#project/alpha"));
        assert!(!spans.iter().any(|s| s.style.tag && s.text.contains("42")));
        assert_eq!(text_of(spans), "A #project/alpha and issue #42 here.");
    }

    #[test]
    fn nested_lists_and_tasks() {
        let doc = parse("- [ ] todo\n- [x] done\n  - nested\n");
        let Block::List(list) = &doc.blocks[0] else {
            panic!("expected a list, got {:?}", doc.blocks[0]);
        };
        assert_eq!(list.items.len(), 2);
        assert_eq!(list.items[0].task, Some(false));
        assert_eq!(list.items[1].task, Some(true));
        assert!(list.items[1]
            .blocks
            .iter()
            .any(|b| matches!(b, Block::List(_))));
    }

    fn flatten(blocks: &[Block]) -> String {
        blocks
            .iter()
            .map(|block| match block {
                Block::Paragraph(spans) | Block::Heading { spans, .. } => text_of(spans),
                Block::List(list) => list
                    .items
                    .iter()
                    .map(|item| flatten(&item.blocks))
                    .collect::<Vec<_>>()
                    .join("|"),
                Block::Quote(blocks) => flatten(blocks),
                Block::Code { text, .. } => text.clone(),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn tight_list_items_keep_their_text() {
        let doc = parse("- alpha\n- beta\n  - nested\n\n## After\n");
        let Block::List(list) = &doc.blocks[0] else {
            panic!("expected a list, got {:?}", doc.blocks[0]);
        };
        assert_eq!(flatten(&list.items[0].blocks), "alpha");
        assert_eq!(flatten(&list.items[1].blocks), "beta\nnested");
        // The heading after the list must not have absorbed the item text.
        assert!(
            matches!(&doc.blocks[1], Block::Heading { spans, .. } if text_of(spans) == "After")
        );
    }

    #[test]
    fn tight_task_items_keep_their_text() {
        let doc = parse("- [x] done thing\n- [ ] todo thing\n");
        let Block::List(list) = &doc.blocks[0] else {
            panic!("expected a list");
        };
        assert_eq!(list.items[0].task, Some(true));
        assert_eq!(flatten(&list.items[0].blocks), "done thing");
        assert_eq!(flatten(&list.items[1].blocks), "todo thing");
    }

    #[test]
    fn a_rule_does_not_swallow_the_text_before_it() {
        let doc = parse("- item\n\n---\n\ntail\n");
        let rule = doc
            .blocks
            .iter()
            .position(|b| matches!(b, Block::Rule))
            .expect("a rule");
        assert!(matches!(&doc.blocks[0], Block::List(_)));
        assert!(matches!(&doc.blocks[rule + 1], Block::Paragraph(s) if text_of(s) == "tail"));
    }

    #[test]
    fn front_matter_is_kept_separate() {
        let doc = parse("---\ntags: [a, b]\n---\n\nBody\n");
        assert!(matches!(&doc.blocks[0], Block::Meta(m) if m.contains("tags")));
        assert!(matches!(&doc.blocks[1], Block::Paragraph(_)));
    }

    #[test]
    fn tables_keep_header_and_rows() {
        let doc = parse("| a | b |\n|---|---|\n| 1 | 2 |\n");
        let Block::Table(table) = &doc.blocks[0] else {
            panic!("expected a table, got {:?}", doc.blocks[0]);
        };
        assert_eq!(table.header.len(), 2);
        assert_eq!(table.rows.len(), 1);
        assert_eq!(text_of(&table.rows[0][1]), "2");
    }

    #[test]
    fn fenced_code_keeps_language_and_text() {
        let doc = parse("```rust\nfn main() {}\n```\n");
        let Block::Code { lang, text } = &doc.blocks[0] else {
            panic!("expected code");
        };
        assert_eq!(lang.as_deref(), Some("rust"));
        assert_eq!(text, "fn main() {}");
    }

    #[test]
    fn unterminated_input_does_not_panic() {
        let _ = parse("> quote\n\n- item\n  - deeper");
        let _ = parse("```\nunclosed");
        let _ = parse("| a |\n|---|");
    }
}
