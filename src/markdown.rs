//! Markdown rendering with native GTK widgets. Text is parsed into a small
//! block tree whose inline content is Pango markup, then turned into labels,
//! frames and grids.

use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(String),
    Heading(u8, String),
    List { start: Option<u64>, items: Vec<Vec<Block>> },
    Code(String),
    Quote(Vec<Block>),
    Rule,
    Table { alignments: Vec<Alignment>, header: Vec<String>, rows: Vec<Vec<String>> },
}

pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            c => escaped.push(c),
        }
    }
    escaped
}

enum Frame {
    Blocks(Vec<Block>),
    Quote(Vec<Block>),
    List { start: Option<u64>, items: Vec<Vec<Block>> },
    Item(Vec<Block>),
    Table { alignments: Vec<Alignment>, header: Vec<String>, rows: Vec<Vec<String>>, row: Vec<String>, in_head: bool },
}

impl Frame {
    fn blocks(&mut self) -> Option<&mut Vec<Block>> {
        match self {
            Frame::Blocks(blocks) | Frame::Quote(blocks) | Frame::Item(blocks) => Some(blocks),
            _ => None,
        }
    }
}

struct Builder {
    stack: Vec<Frame>,
    inline: String,
    code: Option<String>,
}

impl Builder {
    fn push_block(&mut self, block: Block) {
        if let Some(blocks) = self.stack.last_mut().and_then(Frame::blocks) {
            blocks.push(block);
        }
    }

    /// Text of a tight list item arrives without a paragraph around it; turn
    /// it into one before a nested block starts or the item ends.
    fn flush_loose_inline(&mut self) {
        if !self.inline.trim().is_empty() {
            let text = std::mem::take(&mut self.inline);
            self.push_block(Block::Paragraph(text.trim().to_owned()));
        }
        self.inline.clear();
    }

    fn take_inline(&mut self) -> String {
        std::mem::take(&mut self.inline).trim().to_owned()
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph | Tag::Heading { .. } | Tag::TableCell => self.flush_loose_inline(),
            Tag::BlockQuote(_) => {
                self.flush_loose_inline();
                self.stack.push(Frame::Quote(Vec::new()));
            }
            Tag::CodeBlock(_) => {
                self.flush_loose_inline();
                self.code = Some(String::new());
            }
            Tag::List(start) => {
                self.flush_loose_inline();
                self.stack.push(Frame::List { start, items: Vec::new() });
            }
            Tag::Item => self.stack.push(Frame::Item(Vec::new())),
            Tag::Table(alignments) => {
                self.flush_loose_inline();
                self.stack.push(Frame::Table {
                    alignments,
                    header: Vec::new(),
                    rows: Vec::new(),
                    row: Vec::new(),
                    in_head: false,
                });
            }
            Tag::TableHead => {
                if let Some(Frame::Table { in_head, .. }) = self.stack.last_mut() {
                    *in_head = true;
                }
            }
            Tag::Emphasis => self.inline.push_str("<i>"),
            Tag::Strong => self.inline.push_str("<b>"),
            Tag::Strikethrough => self.inline.push_str("<s>"),
            Tag::Link { dest_url, .. } => self.inline.push_str(&format!("<a href=\"{}\">", escape(&dest_url))),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                let text = self.take_inline();
                self.push_block(Block::Paragraph(text));
            }
            TagEnd::Heading(level) => {
                let text = self.take_inline();
                let level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                self.push_block(Block::Heading(level, text));
            }
            TagEnd::CodeBlock => {
                let code = self.code.take().unwrap_or_default();
                self.push_block(Block::Code(code.trim_end_matches('\n').to_owned()));
            }
            TagEnd::BlockQuote(_) => {
                if let Some(Frame::Quote(blocks)) = self.stack.pop() {
                    self.push_block(Block::Quote(blocks));
                }
            }
            TagEnd::Item => {
                self.flush_loose_inline();
                if let Some(Frame::Item(blocks)) = self.stack.pop()
                    && let Some(Frame::List { items, .. }) = self.stack.last_mut()
                {
                    items.push(blocks);
                }
            }
            TagEnd::List(_) => {
                if let Some(Frame::List { start, items }) = self.stack.pop() {
                    self.push_block(Block::List { start, items });
                }
            }
            TagEnd::TableCell => {
                let text = self.take_inline();
                if let Some(Frame::Table { row, .. }) = self.stack.last_mut() {
                    row.push(text);
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(Frame::Table { header, rows, row, in_head, .. }) = self.stack.last_mut() {
                    let cells = std::mem::take(row);
                    if *in_head {
                        *header = cells;
                        *in_head = false;
                    } else {
                        rows.push(cells);
                    }
                }
            }
            TagEnd::Table => {
                if let Some(Frame::Table { alignments, header, rows, .. }) = self.stack.pop() {
                    self.push_block(Block::Table { alignments, header, rows });
                }
            }
            TagEnd::Emphasis => self.inline.push_str("</i>"),
            TagEnd::Strong => self.inline.push_str("</b>"),
            TagEnd::Strikethrough => self.inline.push_str("</s>"),
            TagEnd::Link => self.inline.push_str("</a>"),
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        match &mut self.code {
            Some(code) => code.push_str(text),
            None => self.inline.push_str(&escape(text)),
        }
    }
}

pub fn parse(markdown: &str) -> Vec<Block> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let mut builder = Builder { stack: vec![Frame::Blocks(Vec::new())], inline: String::new(), code: None };
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => builder.start(tag),
            Event::End(tag) => builder.end(tag),
            Event::Text(text) => builder.text(&text),
            Event::Code(code) => builder.inline.push_str(&format!("<tt>{}</tt>", escape(&code))),
            Event::Html(html) | Event::InlineHtml(html) => builder.text(&html),
            Event::SoftBreak => builder.text(if builder.code.is_some() { "\n" } else { " " }),
            Event::HardBreak => builder.inline.push('\n'),
            Event::Rule => {
                builder.flush_loose_inline();
                builder.push_block(Block::Rule);
            }
            Event::TaskListMarker(done) => builder.inline.push_str(if done { "☑ " } else { "☐ " }),
            _ => {}
        }
    }
    builder.flush_loose_inline();
    match builder.stack.into_iter().next() {
        Some(Frame::Blocks(blocks)) => blocks,
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Widgets

use gtk::prelude::*;

fn label(markup: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_markup(markup);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label.set_halign(gtk::Align::Fill);
    label.set_selectable(true);
    // Selectable labels would otherwise take focus and select all their text.
    label.set_focus_on_click(false);
    label.set_can_focus(false);
    label
}

fn render_into(container: &gtk::Box, blocks: &[Block]) {
    for block in blocks {
        let widget: gtk::Widget = match block {
            Block::Paragraph(markup) => label(markup).upcast(),
            Block::Heading(level, markup) => {
                let size = match level {
                    1 => "x-large",
                    2 => "large",
                    _ => "medium",
                };
                let label = label(&format!("<span size=\"{size}\" weight=\"bold\">{markup}</span>"));
                label.add_css_class("md-heading");
                label.upcast()
            }
            Block::Code(code) => {
                let label = label(&escape(code));
                label.set_wrap(false);
                label.add_css_class("monospace");
                let scroller = gtk::ScrolledWindow::builder()
                    .child(&label)
                    .vscrollbar_policy(gtk::PolicyType::Never)
                    .propagate_natural_height(true)
                    .build();
                scroller.add_css_class("md-code");
                scroller.upcast()
            }
            Block::Quote(blocks) => {
                let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);
                inner.add_css_class("md-quote");
                render_into(&inner, blocks);
                inner.upcast()
            }
            Block::Rule => gtk::Separator::new(gtk::Orientation::Horizontal).upcast(),
            Block::List { start, items } => {
                let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
                for (index, item) in items.iter().enumerate() {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    let marker = match start {
                        Some(start) => format!("{}.", start + index as u64),
                        None => "•".to_owned(),
                    };
                    let marker = gtk::Label::new(Some(&marker));
                    marker.set_valign(gtk::Align::Start);
                    marker.set_yalign(0.0);
                    let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
                    body.set_hexpand(true);
                    render_into(&body, item);
                    row.append(&marker);
                    row.append(&body);
                    list.append(&row);
                }
                list.set_margin_start(8);
                list.upcast()
            }
            Block::Table { alignments, header, rows } => {
                let grid = gtk::Grid::new();
                grid.add_css_class("md-table");
                let xalign = |column: usize| match alignments.get(column) {
                    Some(Alignment::Center) => 0.5,
                    Some(Alignment::Right) => 1.0,
                    _ => 0.0,
                };
                let all_rows = std::iter::once((header, true)).chain(rows.iter().map(|r| (r, false)));
                for (row_index, (cells, is_header)) in all_rows.enumerate() {
                    for (column, cell) in cells.iter().enumerate() {
                        let markup = if is_header { format!("<b>{cell}</b>") } else { cell.clone() };
                        let label = label(&markup);
                        label.set_xalign(xalign(column));
                        label.set_hexpand(true);
                        label.add_css_class(if is_header { "md-th" } else { "md-td" });
                        grid.attach(&label, column as i32, row_index as i32, 1, 1);
                    }
                }
                // Cells wrap, so the table fits the width without a scroller
                // (which would measure every cell at its narrowest).
                grid.upcast()
            }
        };
        container.append(&widget);
    }
}

/// A vertical box showing `markdown`.
pub fn render(markdown: &str) -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 10);
    container.add_css_class("markdown");
    render_into(&container, &parse(markdown));
    container
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_markup_is_escaped() {
        assert_eq!(
            parse("Hello **<b>world</b>** and *it* ~~x~~ `a<b` [l](https://e.com/?a=1&b=2)"),
            vec![Block::Paragraph(
                "Hello <b>&lt;b&gt;world&lt;/b&gt;</b> and <i>it</i> <s>x</s> <tt>a&lt;b</tt> <a href=\"https://e.com/?a=1&amp;b=2\">l</a>".into()
            )]
        );
    }

    #[test]
    fn headings_lists_and_code() {
        let blocks = parse("### Title\n\n- one\n- two\n  1. nested\n\n```\nfn a() {}\n```\n\n> quote\n\n---");
        assert_eq!(
            blocks,
            vec![
                Block::Heading(3, "Title".into()),
                Block::List {
                    start: None,
                    items: vec![
                        vec![Block::Paragraph("one".into())],
                        vec![
                            Block::Paragraph("two".into()),
                            Block::List { start: Some(1), items: vec![vec![Block::Paragraph("nested".into())]] },
                        ],
                    ],
                },
                Block::Code("fn a() {}".into()),
                Block::Quote(vec![Block::Paragraph("quote".into())]),
                Block::Rule,
            ]
        );
    }

    #[test]
    fn tables() {
        let blocks = parse("| A | **B** |\n|:--|--:|\n| 1 | 2 |\n| 3 | x&y |");
        assert_eq!(
            blocks,
            vec![Block::Table {
                alignments: vec![Alignment::Left, Alignment::Right],
                header: vec!["A".into(), "<b>B</b>".into()],
                rows: vec![vec!["1".into(), "2".into()], vec!["3".into(), "x&amp;y".into()]],
            }]
        );
    }

    #[test]
    fn plain_text_and_breaks() {
        assert_eq!(parse("a\nb"), vec![Block::Paragraph("a b".into())]);
        assert_eq!(parse("a  \nb"), vec![Block::Paragraph("a\nb".into())]);
        assert_eq!(parse(""), vec![]);
        assert_eq!(parse("<div>x</div>"), vec![Block::Paragraph("&lt;div&gt;x&lt;/div&gt;".into())]);
    }
}
