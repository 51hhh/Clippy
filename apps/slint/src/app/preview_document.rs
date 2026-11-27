use crate::{RichAlignment, RichBlock, RichTableCell, RichTableRow};
use html5ever::tokenizer::{
    states::RawKind, BufferQueue, Tag as HtmlTag, TagKind, Token, TokenSink, TokenSinkResult,
    Tokenizer,
};
use pulldown_cmark::{Alignment, Event, Options, Parser, Tag, TagEnd};
use slint::{ModelRc, StyledText, VecModel};
use std::cell::RefCell;

const MAX_MARKUP_BYTES: usize = 128 * 1024;
const MAX_TABLE_CELLS: usize = 1024;
const MAX_TABLE_COLUMNS: usize = 32;

pub(super) struct Document(Vec<Block>);

enum Block {
    Text(StyledText),
    Table {
        columns: usize,
        rows: Vec<Vec<RichTableCell>>,
    },
}

impl Document {
    pub(super) fn into_model(self) -> ModelRc<RichBlock> {
        let blocks = self
            .0
            .into_iter()
            .map(|block| match block {
                Block::Text(body) => RichBlock {
                    body,
                    ..Default::default()
                },
                Block::Table { columns, rows } => {
                    let rows = rows
                        .into_iter()
                        .map(|cells| RichTableRow {
                            cells: ModelRc::new(VecModel::from(cells)),
                        })
                        .collect::<Vec<_>>();
                    RichBlock {
                        columns: columns as i32,
                        rows: ModelRc::new(VecModel::from(rows)),
                        ..Default::default()
                    }
                }
            })
            .collect::<Vec<_>>();
        ModelRc::new(VecModel::from(blocks))
    }
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    markup: String,
    table: Option<Table>,
    cell_count: usize,
    rejected: bool,
}

#[derive(Default)]
struct Table {
    alignments: Vec<Alignment>,
    columns: usize,
    rows: Vec<Vec<RichTableCell>>,
    row: Vec<RichTableCell>,
    cell: Option<RichTableCell>,
    heading: bool,
}

impl Builder {
    fn flush_text(&mut self) {
        if let Some(body) = styled(&self.markup) {
            self.blocks.push(Block::Text(body));
        }
        self.markup.clear();
    }

    fn start_table(&mut self, alignments: Vec<Alignment>) {
        if self.table.is_some() || alignments.len() > MAX_TABLE_COLUMNS {
            self.rejected = true;
            return;
        }
        self.flush_text();
        self.table = Some(Table {
            alignments,
            ..Default::default()
        });
    }

    fn start_cell(&mut self, cell: RichTableCell) {
        self.end_cell();
        if let Some(table) = &mut self.table {
            table.cell = Some(cell);
        }
    }

    fn end_cell(&mut self) {
        if let Some(table) = &mut self.table {
            if let Some(mut cell) = table.cell.take() {
                if self.cell_count == MAX_TABLE_CELLS {
                    self.rejected = true;
                } else {
                    self.cell_count += 1;
                    let body = styled(&self.markup);
                    self.rejected |= body.is_none() && !self.markup.trim().is_empty();
                    cell.body = body.unwrap_or_default();
                    table.row.push(cell);
                }
            }
            self.markup.clear();
        }
    }

    fn end_row(&mut self) {
        self.end_cell();
        if let Some(table) = &mut self.table {
            if !table.row.is_empty() {
                let columns = table.row.iter().map(|cell| cell.span as usize).sum();
                self.rejected |= columns > MAX_TABLE_COLUMNS;
                table.columns = table.columns.max(columns);
                table.rows.push(std::mem::take(&mut table.row));
            }
        }
    }

    fn end_table(&mut self) {
        self.end_row();
        if let Some(table) = self.table.take() {
            if table.columns > 0 {
                self.blocks.push(Block::Table {
                    columns: table.columns,
                    rows: table.rows,
                });
            }
        }
    }

    fn finish(mut self) -> Option<Document> {
        self.end_table();
        self.flush_text();
        (!self.rejected && !self.blocks.is_empty()).then_some(Document(self.blocks))
    }
}

fn escape(text: &str, output: &mut String) {
    for character in text.chars() {
        if character.is_ascii_punctuation() {
            output.push('\\');
        }
        output.push(character);
    }
}

fn styled(markup: &str) -> Option<StyledText> {
    let markup = markup.trim();
    if markup.is_empty() {
        return None;
    }
    StyledText::from_markdown(markup).ok()
}

#[derive(Default)]
struct HtmlText {
    document: Builder,
    hidden: Option<html5ever::LocalName>,
}

#[derive(Default)]
struct HtmlTokens(RefCell<HtmlText>);

impl TokenSink for HtmlTokens {
    type Handle = ();

    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<Self::Handle> {
        let mut state = self.0.borrow_mut();
        if let Some(hidden) = &state.hidden {
            if matches!(token, Token::TagToken(tag) if tag.kind == TagKind::EndTag && tag.name == *hidden)
            {
                state.hidden = None;
            }
            return TokenSinkResult::Continue;
        }
        match token {
            Token::CharacterTokens(text) => {
                let output = &mut state.document.markup;
                for character in text.chars() {
                    // HTML 正文的空白折叠为一个空格，段落由标签控制。
                    if character.is_ascii_whitespace() {
                        if !output.is_empty() && !output.ends_with([' ', '\n']) {
                            output.push(' ');
                        }
                    } else {
                        if character.is_ascii_punctuation() {
                            output.push('\\');
                        }
                        output.push(character);
                    }
                }
            }
            Token::TagToken(tag) => match (tag.name.as_ref(), tag.kind) {
                ("script" | "style" | "iframe" | "title", TagKind::StartTag) => {
                    let mode = match tag.name.as_ref() {
                        "script" => RawKind::ScriptData,
                        "title" => RawKind::Rcdata,
                        _ => RawKind::Rawtext,
                    };
                    state.hidden = Some(tag.name);
                    return TokenSinkResult::RawData(mode);
                }
                ("table", TagKind::StartTag) => state.document.start_table(Vec::new()),
                ("table", TagKind::EndTag) => state.document.end_table(),
                ("tr", _) => state.document.end_row(),
                ("caption", TagKind::EndTag) if state.document.table.is_some() => {
                    state.document.flush_text()
                }
                ("td" | "th", TagKind::StartTag) if state.document.table.is_some() => {
                    state.document.start_cell(html_cell(&tag));
                    state.document.rejected |= attribute(&tag, "rowspan")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .is_some_and(|value| value != 1);
                }
                ("td" | "th", TagKind::EndTag) => state.document.end_cell(),
                ("b" | "strong", _) => state.document.markup.push_str("**"),
                ("i" | "em", _) => state.document.markup.push('*'),
                ("u", TagKind::StartTag) => state.document.markup.push_str("<u>"),
                ("u", TagKind::EndTag) => state.document.markup.push_str("</u>"),
                ("br", TagKind::StartTag) => state.document.markup.push_str("  \n"),
                ("div" | "p" | "li" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6", _) => {
                    let output = &mut state.document.markup;
                    if !output.is_empty() && !output.ends_with("\n\n") {
                        output.push_str("\n\n");
                    }
                    if tag.name.as_ref() == "li" && tag.kind == TagKind::StartTag {
                        output.push_str("• ");
                    }
                }
                _ => {}
            },
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn attribute<'a>(tag: &'a HtmlTag, name: &str) -> Option<&'a str> {
    tag.attrs
        .iter()
        .find(|attr| attr.name.local.as_ref() == name)
        .map(|attr| attr.value.as_ref())
}

fn html_cell(tag: &HtmlTag) -> RichTableCell {
    let span = attribute(tag, "colspan")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, MAX_TABLE_COLUMNS + 1);
    let alignment = match attribute(tag, "align")
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("left") => RichAlignment::Left,
        Some("center") => RichAlignment::Center,
        Some("right") => RichAlignment::Right,
        _ => RichAlignment::Start,
    };
    RichTableCell {
        span: span as i32,
        heading: tag.name.as_ref() == "th",
        alignment,
        ..Default::default()
    }
}

pub(super) fn html(source: &str) -> Option<Document> {
    if source.len() > MAX_MARKUP_BYTES {
        return None;
    }
    let input = BufferQueue::default();
    input.push_back(source.into());
    let tokenizer = Tokenizer::new(HtmlTokens::default(), Default::default());
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.0.into_inner().document.finish()
}

pub(super) fn markdown(source: &str) -> Option<Document> {
    if source.len() > MAX_MARKUP_BYTES {
        return None;
    }
    let mut document = Builder::default();
    for event in Parser::new_ext(source, Options::ENABLE_TABLES) {
        let markup = &mut document.markup;
        match event {
            Event::Start(Tag::Table(alignments)) => document.start_table(alignments),
            Event::End(TagEnd::Table) => document.end_table(),
            Event::Start(Tag::TableHead) => {
                document
                    .table
                    .as_mut()
                    .expect("Markdown 表头属于表格")
                    .heading = true
            }
            Event::End(TagEnd::TableHead) => {
                document.end_row();
                document
                    .table
                    .as_mut()
                    .expect("Markdown 表头属于表格")
                    .heading = false;
            }
            Event::End(TagEnd::TableRow) => document.end_row(),
            Event::Start(Tag::TableCell) => {
                let table = document.table.as_ref().expect("Markdown 单元格属于表格");
                let alignment = match table.alignments.get(table.row.len()) {
                    Some(Alignment::Left) => RichAlignment::Left,
                    Some(Alignment::Center) => RichAlignment::Center,
                    Some(Alignment::Right) => RichAlignment::Right,
                    _ => RichAlignment::Start,
                };
                document.start_cell(RichTableCell {
                    span: 1,
                    heading: table.heading,
                    alignment,
                    ..Default::default()
                });
            }
            Event::End(TagEnd::TableCell) => document.end_cell(),
            Event::Start(Tag::Heading { .. } | Tag::Strong) => markup.push_str("**"),
            Event::End(TagEnd::Heading(_)) => markup.push_str("**\n\n"),
            Event::End(TagEnd::Strong) => markup.push_str("**"),
            Event::Start(Tag::Emphasis) | Event::End(TagEnd::Emphasis) => markup.push('*'),
            Event::Start(Tag::Item) => markup.push_str("• "),
            Event::End(TagEnd::Paragraph | TagEnd::Item | TagEnd::CodeBlock) => {
                if !markup.ends_with("\n\n") {
                    markup.push_str("\n\n");
                }
            }
            Event::SoftBreak | Event::HardBreak => markup.push_str("  \n"),
            Event::Text(text) => escape(&text, markup),
            Event::Code(text) if !text.contains('`') => {
                markup.push_str("` ");
                markup.push_str(&text);
                markup.push_str(" `");
            }
            Event::Code(text) => escape(&text, markup),
            _ => {}
        }
        if document.rejected {
            return None;
        }
    }
    document.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_preserves_basic_styles_and_leaves_external_content_inert() {
        let output = html(
            r#"<style>not shown</style><p><strong>剪贴板</strong> &amp; <em>text</em></p><img src="https://example.test/pixel" onload="alert()"><script>not shown</script><a href="https://example.test">link label</a>"#,
        );
        let expected = StyledText::from_markdown("**剪贴板** & *text*\n\nlink label").unwrap();
        assert!(matches!(output.unwrap().0.as_slice(), [Block::Text(body)] if *body == expected));
    }

    #[test]
    fn markdown_renders_headings_lists_and_code_without_active_links() {
        let output = markdown("# Heading\n\n- Read [the label](https://example.test) and `a+b`.");
        let expected =
            StyledText::from_markdown("**Heading**\n\n• Read the label and `a+b`.").unwrap();
        assert!(matches!(output.unwrap().0.as_slice(), [Block::Text(body)] if *body == expected));
        assert!(markdown(&"a".repeat(MAX_MARKUP_BYTES + 1)).is_none());
    }

    #[test]
    fn markdown_preserves_table_order_and_column_alignment() {
        let doc = markdown(
            "Before\n\n| Name | Qty | Note |\n| :--- | ---: | :---: |\n| 剪贴板 | 42 | **bold** |\n\nAfter",
        ).unwrap();
        let [Block::Text(before), Block::Table { columns: 3, rows }, Block::Text(after)] =
            doc.0.as_slice()
        else {
            panic!("expected text, table, and trailing text");
        };
        assert_eq!(before, &styled("Before").unwrap());
        assert_eq!(after, &styled("After").unwrap());
        assert_eq!(rows.len(), 2);
        assert!(rows[0].iter().all(|cell| cell.heading));
        assert!(rows[1].iter().all(|cell| !cell.heading && cell.span == 1));
        assert_eq!(rows[1][0].body, styled("剪贴板").unwrap());
        assert_eq!(rows[1][2].body, styled("**bold**").unwrap());
        assert_eq!(rows[1][0].alignment, RichAlignment::Left);
        assert_eq!(rows[1][1].alignment, RichAlignment::Right);
        assert_eq!(rows[1][2].alignment, RichAlignment::Center);
    }

    #[test]
    fn html_preserves_column_spans_and_keeps_cell_resources_inert() {
        let doc = html(concat!(
            "<p>Before</p><table><caption>表格说明</caption><tr><th colspan='2'>Header</th></tr>",
            "<tr><td align='RIGHT'><b>42</b><img src='https://example.test/pixel'>",
            "<script>not shown</script></td><td>剪贴板 &amp; 🦀</td></tr></table><p>After</p>"
        ))
        .unwrap();
        let [Block::Text(before), Block::Text(caption), Block::Table { columns: 2, rows }, Block::Text(after)] =
            doc.0.as_slice()
        else {
            panic!("expected text, caption, table, and trailing text");
        };
        assert_eq!(before, &styled("Before").unwrap());
        assert_eq!(caption, &styled("表格说明").unwrap());
        assert_eq!(after, &styled("After").unwrap());
        assert_eq!(rows[0].len(), 1);
        assert!(rows[0][0].heading);
        assert_eq!(rows[0][0].span, 2);
        assert_eq!(rows[1][0].body, styled("**42**").unwrap());
        assert_eq!(rows[1][0].alignment, RichAlignment::Right);
        assert_eq!(rows[1][1].body, styled("剪贴板 & 🦀").unwrap());
    }

    #[test]
    fn oversized_or_unsupported_tables_keep_the_plain_preview_available() {
        let html_rows = "<tr><td>a</td></tr>".repeat(MAX_TABLE_CELLS + 1);
        assert!(html(&format!("<table>{html_rows}</table>")).is_none());
        let header = "| a ".repeat(MAX_TABLE_COLUMNS + 1);
        let rule = "| --- ".repeat(MAX_TABLE_COLUMNS + 1);
        assert!(markdown(&format!("{header}|\n{rule}|")).is_none());
        assert!(html("<table><tr><td rowspan=2>a</td></tr></table>").is_none());
        assert!(html("<table><tr><td><table></table></td></tr></table>").is_none());
    }
}
