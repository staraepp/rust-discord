//! Converts CommonMark into styled, native text. No HTML execution or networking.
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub style: Style,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RichText {
    pub text: String,
    pub spans: Vec<Span>,
    pub links: Vec<(Range<usize>, String)>,
}

pub fn parse(content: &str) -> RichText {
    let mut out = RichText::default();
    let (mut bold, mut italic, mut strike, mut code) = (0u32, 0u32, 0u32, 0u32);
    let mut link: Option<(usize, String)> = None;
    let newline = |out: &mut RichText| {
        if !out.text.is_empty() && !out.text.ends_with('\n') {
            out.text.push('\n');
        }
    };
    for event in Parser::new_ext(content, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Strong => bold += 1,
                Tag::Emphasis => italic += 1,
                Tag::Strikethrough => strike += 1,
                Tag::CodeBlock(_) => {
                    newline(&mut out);
                    code += 1;
                }
                Tag::Heading { .. } => {
                    newline(&mut out);
                    bold += 1;
                }
                Tag::Item => {
                    newline(&mut out);
                    out.text.push_str("• ");
                }
                Tag::Link { dest_url, .. } => link = Some((out.text.len(), dest_url.to_string())),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Strong => bold = bold.saturating_sub(1),
                TagEnd::Emphasis => italic = italic.saturating_sub(1),
                TagEnd::Strikethrough => strike = strike.saturating_sub(1),
                TagEnd::CodeBlock => {
                    code = code.saturating_sub(1);
                    newline(&mut out);
                }
                TagEnd::Heading(_) => {
                    bold = bold.saturating_sub(1);
                    newline(&mut out);
                }
                TagEnd::Paragraph | TagEnd::Item | TagEnd::BlockQuote(_) => newline(&mut out),
                TagEnd::Link => {
                    if let Some((start, url)) = link.take()
                        && (url.starts_with("https://") || url.starts_with("http://"))
                    {
                        out.links.push((start..out.text.len(), url));
                    }
                }
                _ => {}
            },
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                let start = out.text.len();
                out.text.push_str(&text);
                out.spans.push(Span {
                    range: start..out.text.len(),
                    style: Style {
                        bold: bold > 0,
                        italic: italic > 0,
                        strike: strike > 0,
                        code: code > 0,
                    },
                });
            }
            Event::Code(text) => {
                let start = out.text.len();
                out.text.push_str(&text);
                out.spans.push(Span {
                    range: start..out.text.len(),
                    style: Style {
                        bold: bold > 0,
                        italic: italic > 0,
                        strike: strike > 0,
                        code: true,
                    },
                });
            }
            Event::SoftBreak | Event::HardBreak | Event::Rule => out.text.push('\n'),
            _ => {}
        }
    }
    while out.text.ends_with('\n') {
        out.text.pop();
    }
    for span in &mut out.spans {
        span.range.end = span.range.end.min(out.text.len());
    }
    out.spans.retain(|span| !span.range.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_styles_and_unicode_have_valid_byte_ranges() {
        let rich = parse("hi **bold *😀*** and `code` ~~gone~~");
        assert_eq!(rich.text, "hi bold 😀 and code gone");
        assert!(
            rich.spans
                .iter()
                .any(|span| span.style.bold && span.style.italic)
        );
        for span in &rich.spans {
            assert!(rich.text.is_char_boundary(span.range.start));
            assert!(rich.text.is_char_boundary(span.range.end));
        }
    }
    #[test]
    fn links_and_html_are_inert() {
        let rich =
            parse("[good](https://example.invalid) [bad](javascript:alert(1)) <script>x</script>");
        assert_eq!(rich.links.len(), 1);
        assert!(rich.text.contains("<script>"));
    }
}
