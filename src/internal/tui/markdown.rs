use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as RLine, Span};

/// Convert a markdown string to styled ratatui Text.
///
/// Supports headings (bold), bold/italic inline, code spans (green),
/// code blocks (green on dark), list items with bullet, blockquotes,
/// tables (aligned columns with bold headers), links (text + URL),
/// images (placeholder), and strikethrough.
pub(super) fn markdown_to_text(input: &str) -> ratatui::text::Text<'static> {
    use pulldown_cmark::{Event as MdEvent, Options, Parser, Tag, TagEnd};

    // Only enable extensions we actually handle. Unhandled extension syntax
    // (math, footnotes, superscript, subscript) passes through as plain text.
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_HEADING_ATTRIBUTES;
    let parser = Parser::new_ext(input, options);

    let mut lines: Vec<RLine<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default()];
    let mut in_heading = false;
    let mut in_code_block = false;
    let mut list_depth: usize = 0;

    // Table state
    let mut in_table = false;
    let mut in_table_head = false;
    let mut table_row_cells: Vec<Vec<Span<'static>>> = Vec::new();
    let mut table_column_widths: Vec<usize> = Vec::new();
    let mut table_rows: Vec<(Vec<Vec<Span<'static>>>, bool)> = Vec::new(); // (cells, is_header)

    // Link state
    let mut link_url: Option<String> = None;

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
            MdEvent::Start(Tag::Strikethrough) => {
                style_stack.push(current_style(&style_stack).add_modifier(Modifier::CROSSED_OUT));
            }
            MdEvent::End(TagEnd::Strikethrough) => {
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
                if !in_heading && !in_table {
                    lines.push(RLine::default());
                }
            }

            // --- Tables ---
            MdEvent::Start(Tag::Table(_)) => {
                flush_line(&mut lines, &mut current_spans);
                in_table = true;
                table_rows.clear();
                table_column_widths.clear();
            }
            MdEvent::End(TagEnd::Table) => {
                // Render the collected table
                render_table(&mut lines, &table_rows, &table_column_widths);
                in_table = false;
                table_rows.clear();
                table_column_widths.clear();
            }
            MdEvent::Start(Tag::TableHead) => {
                in_table_head = true;
                table_row_cells.clear();
            }
            MdEvent::End(TagEnd::TableHead) => {
                // TableHead contains cells directly (no TableRow wrapper)
                collect_table_row(
                    &mut table_rows,
                    &mut table_row_cells,
                    &mut table_column_widths,
                    true,
                );
                in_table_head = false;
            }
            MdEvent::Start(Tag::TableRow) => {
                table_row_cells.clear();
            }
            MdEvent::End(TagEnd::TableRow) => {
                collect_table_row(
                    &mut table_rows,
                    &mut table_row_cells,
                    &mut table_column_widths,
                    false,
                );
            }
            MdEvent::Start(Tag::TableCell) => {
                current_spans.clear();
                if in_table_head {
                    style_stack.push(current_style(&style_stack).add_modifier(Modifier::BOLD));
                }
            }
            MdEvent::End(TagEnd::TableCell) => {
                if in_table_head {
                    style_stack.pop();
                }
                table_row_cells.push(std::mem::take(&mut current_spans));
            }

            // --- Links ---
            MdEvent::Start(Tag::Link { dest_url, .. }) => {
                link_url = Some(dest_url.to_string());
            }
            MdEvent::End(TagEnd::Link) => {
                if let Some(url) = link_url.take() {
                    current_spans.push(Span::styled(
                        format!(" ({url})"),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }

            // --- Images ---
            MdEvent::Start(Tag::Image { .. }) => {
                current_spans.push(Span::styled(
                    "[image: ".to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            MdEvent::End(TagEnd::Image) => {
                current_spans.push(Span::styled(
                    "]".to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
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

/// Collect a completed table row, updating column widths.
fn collect_table_row(
    table_rows: &mut Vec<(Vec<Vec<Span<'static>>>, bool)>,
    table_row_cells: &mut Vec<Vec<Span<'static>>>,
    table_column_widths: &mut Vec<usize>,
    is_header: bool,
) {
    for (i, cell) in table_row_cells.iter().enumerate() {
        let cell_width: usize = cell.iter().map(|s| s.content.len()).sum();
        if i >= table_column_widths.len() {
            table_column_widths.push(cell_width);
        } else if cell_width > table_column_widths[i] {
            table_column_widths[i] = cell_width;
        }
    }
    table_rows.push((std::mem::take(table_row_cells), is_header));
}

/// Render collected table rows into styled lines with aligned columns.
fn render_table(
    lines: &mut Vec<RLine<'static>>,
    rows: &[(Vec<Vec<Span<'static>>>, bool)],
    column_widths: &[usize],
) {
    for (row_idx, (cells, is_header)) in rows.iter().enumerate() {
        let mut row_spans: Vec<Span<'static>> = Vec::new();
        for (col_idx, cell_spans) in cells.iter().enumerate() {
            if col_idx > 0 {
                row_spans.push(Span::raw("  "));
            }
            let content_width: usize = cell_spans.iter().map(|s| s.content.len()).sum();
            row_spans.extend(cell_spans.iter().cloned());
            let col_width = column_widths.get(col_idx).copied().unwrap_or(content_width);
            let padding = col_width.saturating_sub(content_width);
            if padding > 0 {
                row_spans.push(Span::raw(" ".repeat(padding)));
            }
        }
        lines.push(RLine::from(row_spans));

        // Add separator line after header row
        if *is_header && row_idx == 0 {
            let mut sep_parts: Vec<Span<'static>> = Vec::new();
            for (i, &w) in column_widths.iter().enumerate() {
                if i > 0 {
                    sep_parts.push(Span::styled("  ", Style::default().fg(Color::DarkGray)));
                }
                sep_parts.push(Span::styled(
                    "─".repeat(w),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            lines.push(RLine::from(sep_parts));
        }
    }
    lines.push(RLine::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: extract plain text from rendered output (strips styling).
    fn plain_text(text: &ratatui::text::Text) -> String {
        text.lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Helper: check if any span in the output has a specific modifier.
    fn has_modifier(text: &ratatui::text::Text, modifier: Modifier) -> bool {
        text.lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.style.add_modifier.contains(modifier))
        })
    }

    // --- Table tests ---

    #[test]
    fn table_renders_aligned_columns() {
        let input = "| Name | Age |\n|------|-----|\n| Alice | 30 |\n| Bob | 7 |\n";
        let result = markdown_to_text(input);
        let text = plain_text(&result);
        // Each row should be on its own line with columns separated
        assert!(text.contains("Alice"), "should contain Alice: {text}");
        assert!(text.contains("Bob"), "should contain Bob: {text}");
        // Verify rows are on separate lines
        let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
        assert!(
            lines.len() >= 3,
            "should have header + separator + 2 data rows, got {}: {:?}",
            lines.len(),
            lines
        );
    }

    #[test]
    fn table_header_is_bold() {
        let input = "| H1 | H2 |\n|---|---|\n| a | b |\n";
        let result = markdown_to_text(input);
        assert!(
            has_modifier(&result, Modifier::BOLD),
            "table headers should be bold"
        );
    }

    #[test]
    fn table_has_separator_after_header() {
        let input = "| H1 | H2 |\n|---|---|\n| a | b |\n";
        let result = markdown_to_text(input);
        let text = plain_text(&result);
        assert!(text.contains('─'), "should have separator line: {text}");
    }

    // --- Link tests ---

    #[test]
    fn link_shows_text_and_url() {
        let input = "See [docs](https://example.com) for details.";
        let result = markdown_to_text(input);
        let text = plain_text(&result);
        assert!(text.contains("docs"), "should contain link text: {text}");
        assert!(
            text.contains("https://example.com"),
            "should contain URL: {text}"
        );
    }

    // --- Image tests ---

    #[test]
    fn image_renders_placeholder() {
        let input = "![screenshot](img.png)";
        let result = markdown_to_text(input);
        let text = plain_text(&result);
        assert!(
            text.contains("[image:") && text.contains("screenshot"),
            "should render image placeholder: {text}"
        );
    }

    // --- Strikethrough tests ---

    #[test]
    fn strikethrough_has_crossed_out_modifier() {
        let input = "This is ~~deleted~~ text.";
        let result = markdown_to_text(input);
        assert!(
            has_modifier(&result, Modifier::CROSSED_OUT),
            "strikethrough should use CROSSED_OUT modifier"
        );
        let text = plain_text(&result);
        assert!(
            text.contains("deleted"),
            "strikethrough text should be present: {text}"
        );
    }

    // --- Non-regression tests ---

    #[test]
    fn heading_still_renders_bold_cyan() {
        let input = "# Title\n\nBody text.";
        let result = markdown_to_text(input);
        let text = plain_text(&result);
        assert!(text.contains("Title"), "heading text: {text}");
        assert!(text.contains("Body text"), "body text: {text}");
        assert!(
            has_modifier(&result, Modifier::BOLD),
            "heading should be bold"
        );
    }

    #[test]
    fn list_still_renders_bullets() {
        let input = "- item one\n- item two\n";
        let result = markdown_to_text(input);
        let text = plain_text(&result);
        assert!(text.contains('•'), "should have bullet: {text}");
        assert!(text.contains("item one"), "should have item: {text}");
    }

    #[test]
    fn code_block_still_renders_green() {
        let input = "```\nfn main() {}\n```\n";
        let result = markdown_to_text(input);
        let text = plain_text(&result);
        assert!(text.contains("fn main()"), "code block content: {text}");
        // Check green color
        let has_green = result
            .lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.style.fg == Some(Color::Green)));
        assert!(has_green, "code should be green");
    }

    #[test]
    fn inline_code_still_renders_green() {
        let input = "Use `cargo build` to compile.";
        let result = markdown_to_text(input);
        let text = plain_text(&result);
        assert!(text.contains("cargo build"), "inline code: {text}");
        let has_green = result
            .lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.style.fg == Some(Color::Green)));
        assert!(has_green, "inline code should be green");
    }
}
