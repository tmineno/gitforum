use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as RLine, Span};

/// Convert a markdown string to styled ratatui Text.
///
/// Supports headings (bold), bold/italic inline, code spans (green),
/// code blocks (green on dark), list items with bullet, and blockquotes.
pub(super) fn markdown_to_text(input: &str) -> ratatui::text::Text<'static> {
    use pulldown_cmark::{Event as MdEvent, Options, Parser, Tag, TagEnd};

    let parser = Parser::new_ext(input, Options::all());

    let mut lines: Vec<RLine<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default()];
    let mut in_heading = false;
    let mut in_code_block = false;
    let mut list_depth: usize = 0;

    let current_style = |stack: &[Style]| -> Style { stack.last().copied().unwrap_or_default() };

    let flush_line = |lines: &mut Vec<RLine<'static>>, spans: &mut Vec<Span<'static>>| {
        if !spans.is_empty() {
            lines.push(RLine::from(std::mem::take(spans)));
        }
    };

    for event in parser {
        match event {
            MdEvent::Start(Tag::Heading { .. }) => {
                flush_line(&mut lines, &mut current_spans);
                in_heading = true;
                style_stack.push(
                    current_style(&style_stack)
                        .add_modifier(Modifier::BOLD)
                        .fg(Color::Cyan),
                );
            }
            MdEvent::End(TagEnd::Heading(_)) => {
                flush_line(&mut lines, &mut current_spans);
                lines.push(RLine::default());
                in_heading = false;
                style_stack.pop();
            }
            MdEvent::Start(Tag::Emphasis) => {
                style_stack.push(current_style(&style_stack).add_modifier(Modifier::ITALIC));
            }
            MdEvent::End(TagEnd::Emphasis) => {
                style_stack.pop();
            }
            MdEvent::Start(Tag::Strong) => {
                style_stack.push(current_style(&style_stack).add_modifier(Modifier::BOLD));
            }
            MdEvent::End(TagEnd::Strong) => {
                style_stack.pop();
            }
            MdEvent::Start(Tag::CodeBlock(_)) => {
                flush_line(&mut lines, &mut current_spans);
                in_code_block = true;
                style_stack.push(Style::default().fg(Color::Green));
            }
            MdEvent::End(TagEnd::CodeBlock) => {
                flush_line(&mut lines, &mut current_spans);
                in_code_block = false;
                style_stack.pop();
            }
            MdEvent::Start(Tag::List(_)) => {
                flush_line(&mut lines, &mut current_spans);
                list_depth += 1;
            }
            MdEvent::End(TagEnd::List(_)) => {
                flush_line(&mut lines, &mut current_spans);
                list_depth = list_depth.saturating_sub(1);
            }
            MdEvent::Start(Tag::Item) => {
                flush_line(&mut lines, &mut current_spans);
                let indent = "  ".repeat(list_depth.saturating_sub(1));
                current_spans.push(Span::styled(
                    format!("{indent}• "),
                    current_style(&style_stack),
                ));
            }
            MdEvent::End(TagEnd::Item) => {
                flush_line(&mut lines, &mut current_spans);
            }
            MdEvent::Start(Tag::BlockQuote(_)) => {
                flush_line(&mut lines, &mut current_spans);
                style_stack.push(Style::default().fg(Color::DarkGray));
            }
            MdEvent::End(TagEnd::BlockQuote(_)) => {
                flush_line(&mut lines, &mut current_spans);
                style_stack.pop();
            }
            MdEvent::Start(Tag::Paragraph) => {
                if !in_heading {
                    flush_line(&mut lines, &mut current_spans);
                }
            }
            MdEvent::End(TagEnd::Paragraph) => {
                flush_line(&mut lines, &mut current_spans);
                if !in_heading {
                    lines.push(RLine::default());
                }
            }
            MdEvent::Code(code) => {
                current_spans.push(Span::styled(
                    code.to_string(),
                    Style::default().fg(Color::Green),
                ));
            }
            MdEvent::Text(text) => {
                if in_code_block {
                    // Preserve newlines in code blocks
                    for (i, line) in text.split('\n').enumerate() {
                        if i > 0 {
                            flush_line(&mut lines, &mut current_spans);
                        }
                        if !line.is_empty() {
                            current_spans
                                .push(Span::styled(line.to_string(), current_style(&style_stack)));
                        }
                    }
                } else {
                    current_spans.push(Span::styled(text.to_string(), current_style(&style_stack)));
                }
            }
            MdEvent::SoftBreak => {
                current_spans.push(Span::raw(" "));
            }
            MdEvent::HardBreak => {
                flush_line(&mut lines, &mut current_spans);
            }
            MdEvent::Rule => {
                flush_line(&mut lines, &mut current_spans);
                lines.push(RLine::from(Span::styled(
                    "───────────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(RLine::default());
            }
            _ => {}
        }
    }
    flush_line(&mut lines, &mut current_spans);

    ratatui::text::Text::from(lines)
}
