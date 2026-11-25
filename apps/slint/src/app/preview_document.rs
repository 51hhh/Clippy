use html5ever::tokenizer::{
    states::RawKind, BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer,
};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use slint::StyledText;
use std::cell::RefCell;

const MAX_MARKUP_BYTES: usize = 128 * 1024;

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
    output: String,
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
                for character in text.chars() {
                    // HTML 正文的空白折叠为一个空格，段落由标签控制。
                    if character.is_ascii_whitespace() {
                        if !state.output.is_empty() && !state.output.ends_with([' ', '\n']) {
                            state.output.push(' ');
                        }
                    } else {
                        if character.is_ascii_punctuation() {
                            state.output.push('\\');
                        }
                        state.output.push(character);
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
                ("b" | "strong", _) => state.output.push_str("**"),
                ("i" | "em", _) => state.output.push('*'),
                ("u", TagKind::StartTag) => state.output.push_str("<u>"),
                ("u", TagKind::EndTag) => state.output.push_str("</u>"),
                ("br", TagKind::StartTag) => state.output.push_str("  \n"),
                ("div" | "p" | "li" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6", _) => {
                    if !state.output.is_empty() && !state.output.ends_with("\n\n") {
                        state.output.push_str("\n\n");
                    }
                    if tag.name.as_ref() == "li" && tag.kind == TagKind::StartTag {
                        state.output.push_str("• ");
                    }
                }
                _ => {}
            },
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

pub(super) fn html(source: &str) -> Option<StyledText> {
    if source.len() > MAX_MARKUP_BYTES {
        return None;
    }
    let input = BufferQueue::default();
    input.push_back(source.into());
    let tokenizer = Tokenizer::new(HtmlTokens::default(), Default::default());
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    styled(&tokenizer.sink.0.into_inner().output)
}

pub(super) fn markdown(source: &str) -> Option<StyledText> {
    if source.len() > MAX_MARKUP_BYTES {
        return None;
    }
    let mut markup = String::new();
    for event in Parser::new(source) {
        match event {
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
            Event::Text(text) => escape(&text, &mut markup),
            Event::Code(text) if !text.contains('`') => {
                markup.push_str("` ");
                markup.push_str(&text);
                markup.push_str(" `");
            }
            Event::Code(text) => escape(&text, &mut markup),
            _ => {}
        }
    }
    styled(&markup)
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
        assert_eq!(output, Some(expected));
    }

    #[test]
    fn markdown_renders_headings_lists_and_code_without_active_links() {
        let output = markdown("# Heading\n\n- Read [the label](https://example.test) and `a+b`.");
        let expected =
            StyledText::from_markdown("**Heading**\n\n• Read the label and `a+b`.").unwrap();
        assert_eq!(output, Some(expected));
        assert!(markdown(&"a".repeat(MAX_MARKUP_BYTES + 1)).is_none());
    }
}
