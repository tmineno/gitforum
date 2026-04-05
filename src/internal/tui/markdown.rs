use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as RLine, Span};

/// Convert a markdown string to styled ratatui Text.
///
/// `width` is the available content width (e.g. inner area after borders).
/// When provided, tables and horizontal rules are constrained to fit.
///
/// Supports headings (bold), bold/italic inline, code spans (green),
/// code blocks (green on dark), list items with bullet, blockquotes,
/// tables (aligned columns with bold headers), links (text + URL),
/// images (placeholder), and strikethrough.
pub(super) fn markdown_to_text(input: &str, width: Option<usize>) -> ratatui::text::Text<'static> {
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
                render_table(&mut lines, &table_rows, &table_column_widths, width);
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
                let rule_width = width.unwrap_or(39).min(200);
                lines.push(RLine::from(Span::styled(
                    "─".repeat(rule_width),
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
///
/// When `available_width` is provided, columns are shrunk proportionally so
/// the total table width fits.  Cell content that overflows its allocated
/// column width wraps onto additional lines within the same row.
fn render_table(
    lines: &mut Vec<RLine<'static>>,
    rows: &[(Vec<Vec<Span<'static>>>, bool)],
    column_widths: &[usize],
    available_width: Option<usize>,
) {
    let num_cols = column_widths.len();
    if num_cols == 0 {
        return;
    }

    // Total natural width = sum of columns + 2-char separator between columns
    let separator_total = (num_cols.saturating_sub(1)) * 2;
    let natural_total: usize = column_widths.iter().sum::<usize>() + separator_total;

    // Decide effective column widths, possibly constrained.
    let effective_widths: Vec<usize> = if let Some(avail) = available_width {
        if natural_total <= avail || avail < separator_total + num_cols {
            // Fits, or viewport is too tiny to do anything useful
            column_widths.to_vec()
        } else {
            constrain_column_widths(column_widths, avail.saturating_sub(separator_total))
        }
    } else {
        column_widths.to_vec()
    };

    for (row_idx, (cells, is_header)) in rows.iter().enumerate() {
        // Wrap each cell's content into multiple lines within its column width
        let wrapped_cells: Vec<Vec<Vec<Span<'static>>>> = cells
            .iter()
            .enumerate()
            .map(|(col_idx, cell_spans)| {
                let w = effective_widths.get(col_idx).copied().unwrap_or(0);
                wrap_cell_spans(cell_spans, w)
            })
            .collect();

        let row_height = wrapped_cells.iter().map(|c| c.len()).max().unwrap_or(1);

        for line_idx in 0..row_height {
            let mut row_spans: Vec<Span<'static>> = Vec::new();
            for (col_idx, wrapped) in wrapped_cells.iter().enumerate() {
                if col_idx > 0 {
                    row_spans.push(Span::raw("  "));
                }
                let col_width = effective_widths.get(col_idx).copied().unwrap_or(0);
                if let Some(line_spans) = wrapped.get(line_idx) {
                    let content_width: usize = line_spans.iter().map(|s| s.content.len()).sum();
                    row_spans.extend(line_spans.iter().cloned());
                    let padding = col_width.saturating_sub(content_width);
                    if padding > 0 {
                        row_spans.push(Span::raw(" ".repeat(padding)));
                    }
                } else {
                    // This cell has fewer lines – emit blank padding
                    if col_width > 0 {
                        row_spans.push(Span::raw(" ".repeat(col_width)));
                    }
                }
            }
            lines.push(RLine::from(row_spans));
        }

        // Add separator line after header row
        if *is_header && row_idx == 0 {
            let mut sep_parts: Vec<Span<'static>> = Vec::new();
            for (i, &w) in effective_widths.iter().enumerate() {
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

/// Distribute `budget` characters among columns, shrinking wide columns first.
///
/// Each column gets at least `min(natural_width, MIN_COL)` where MIN_COL = 3.
/// Remaining budget is distributed proportionally to natural widths.
fn constrain_column_widths(natural: &[usize], budget: usize) -> Vec<usize> {
    const MIN_COL: usize = 3;
    let n = natural.len();
    let mut result = vec![0usize; n];

    // Phase 1: give each column its minimum
    let mins: Vec<usize> = natural.iter().map(|&w| w.min(MIN_COL)).collect();
    let min_total: usize = mins.iter().sum();
    if budget <= min_total {
        // Can't even fit minimums – distribute budget evenly
        let each = budget / n.max(1);
        let mut remainder = budget % n.max(1);
        for r in result.iter_mut() {
            *r = each
                + if remainder > 0 {
                    remainder -= 1;
                    1
                } else {
                    0
                };
        }
        return result;
    }

    // Phase 2: distribute remaining budget proportionally
    let extra_budget = budget - min_total;
    let extra_natural: Vec<usize> = natural
        .iter()
        .zip(&mins)
        .map(|(&w, &m)| w.saturating_sub(m))
        .collect();
    let extra_total: usize = extra_natural.iter().sum();

    if extra_total == 0 {
        return mins;
    }

    let mut assigned_extra = 0usize;
    for (i, &extra) in extra_natural.iter().enumerate() {
        let share = if extra_total > 0 {
            (extra as u64 * extra_budget as u64 / extra_total as u64) as usize
        } else {
            0
        };
        // Don't exceed the column's natural width
        let capped = share.min(extra);
        result[i] = mins[i] + capped;
        assigned_extra += capped;
    }

    // Distribute rounding remainder to columns that still have headroom
    let mut leftover = extra_budget.saturating_sub(assigned_extra);
    for i in 0..n {
        if leftover == 0 {
            break;
        }
        let headroom = natural[i].saturating_sub(result[i]);
        let give = headroom.min(leftover);
        result[i] += give;
        leftover -= give;
    }

    result
}

/// Wrap styled spans into multiple lines, each fitting within `width`.
///
/// Breaks at word boundaries (spaces) when possible, falling back to
/// character-level breaks when a single word exceeds the column width.
fn wrap_cell_spans(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    if width == 0 {
        return vec![vec![]];
    }

    let total_len: usize = spans.iter().map(|s| s.content.len()).sum();
    if total_len <= width {
        return vec![spans.to_vec()];
    }

    let mut result: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current_line: Vec<Span<'static>> = Vec::new();
    let mut line_len = 0usize;

    for span in spans {
        let style = span.style;
        let mut remaining = span.content.as_ref();

        while !remaining.is_empty() {
            let avail = width.saturating_sub(line_len);
            if avail == 0 {
                // Current line is full, start a new one
                result.push(std::mem::take(&mut current_line));
                line_len = 0;
                continue;
            }

            if remaining.len() <= avail {
                // Rest of this span fits on the current line
                current_line.push(Span::styled(remaining.to_string(), style));
                line_len += remaining.len();
                break;
            }

            // Need to break. Try to find a word boundary (last space within avail).
            let search_end = char_boundary(remaining, avail);
            let break_pos = remaining[..search_end].rfind(' ');

            if let Some(pos) = break_pos {
                if pos > 0 || line_len > 0 {
                    // Take text up to (not including) the space
                    if pos > 0 {
                        current_line.push(Span::styled(remaining[..pos].to_string(), style));
                    }
                    remaining = remaining[pos..].trim_start();
                    result.push(std::mem::take(&mut current_line));
                    line_len = 0;
                    continue;
                }
            }

            if line_len == 0 {
                // Single word wider than column — force a character-level break
                let end = char_boundary(remaining, avail);
                if end > 0 {
                    current_line.push(Span::styled(remaining[..end].to_string(), style));
                    remaining = &remaining[end..];
                    result.push(std::mem::take(&mut current_line));
                    line_len = 0;
                } else {
                    break; // shouldn't happen with width >= 1
                }
            } else {
                // Wrap to next line and retry
                result.push(std::mem::take(&mut current_line));
                line_len = 0;
            }
        }
    }

    if !current_line.is_empty() {
        result.push(current_line);
    }
    if result.is_empty() {
        result.push(Vec::new());
    }

    result
}

/// Find the largest byte index <= `max_bytes` that is a char boundary.
fn char_boundary(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }
    let mut i = max_bytes;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
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
        let result = markdown_to_text(input, None);
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
        let result = markdown_to_text(input, None);
        assert!(
            has_modifier(&result, Modifier::BOLD),
            "table headers should be bold"
        );
    }

    #[test]
    fn table_has_separator_after_header() {
        let input = "| H1 | H2 |\n|---|---|\n| a | b |\n";
        let result = markdown_to_text(input, None);
        let text = plain_text(&result);
        assert!(text.contains('─'), "should have separator line: {text}");
    }

    // --- Link tests ---

    #[test]
    fn link_shows_text_and_url() {
        let input = "See [docs](https://example.com) for details.";
        let result = markdown_to_text(input, None);
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
        let result = markdown_to_text(input, None);
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
        let result = markdown_to_text(input, None);
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
        let result = markdown_to_text(input, None);
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
        let result = markdown_to_text(input, None);
        let text = plain_text(&result);
        assert!(text.contains('•'), "should have bullet: {text}");
        assert!(text.contains("item one"), "should have item: {text}");
    }

    #[test]
    fn code_block_still_renders_green() {
        let input = "```\nfn main() {}\n```\n";
        let result = markdown_to_text(input, None);
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
        let result = markdown_to_text(input, None);
        let text = plain_text(&result);
        assert!(text.contains("cargo build"), "inline code: {text}");
        let has_green = result
            .lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.style.fg == Some(Color::Green)));
        assert!(has_green, "inline code should be green");
    }

    // --- Width-constrained table tests ---

    #[test]
    fn table_fits_within_width() {
        // Table with short content should not be truncated even with a width
        let input = "| A | B |\n|---|---|\n| x | y |\n";
        let result = markdown_to_text(input, Some(40));
        let text = plain_text(&result);
        // No ellipsis since the table fits
        assert!(
            !text.contains('…'),
            "small table should not truncate: {text}"
        );
        // Content intact
        assert!(text.contains('x'), "cell content preserved: {text}");
    }

    #[test]
    fn wide_table_wraps_within_width() {
        let input = "| File | Change |\n|---|---|\n| tools/pipintegrator/node_editor.h | New draw_orthogonal_link(), new is_point_near_polyline(), update draw_link() and process_links() |\n";
        let result = markdown_to_text(input, Some(50));
        let text = plain_text(&result);
        // Every non-empty line should fit within 50 display columns
        for line in text.lines().filter(|l| !l.is_empty()) {
            let display_width = line.chars().count();
            assert!(
                display_width <= 50,
                "line exceeds width 50 ({} chars): {line:?}",
                display_width
            );
        }
        // Content wraps instead of being truncated — no ellipsis
        assert!(!text.contains('…'), "should wrap, not truncate: {text}");
        // Tail portions of each cell's content appear on later lines
        assert!(
            text.contains("tor.h"),
            "end of file path should appear on a wrapped line: {text}"
        );
        assert!(
            text.contains("process_links()"),
            "last part of change text should be present: {text}"
        );
        // Data row should span multiple display lines
        let non_empty: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        assert!(
            non_empty.len() > 3,
            "wide content should wrap into extra lines (got {}): {non_empty:?}",
            non_empty.len()
        );
    }

    #[test]
    fn constrain_column_widths_proportional() {
        // 3 columns: natural widths [20, 10, 30] = 60, budget = 30
        let result = constrain_column_widths(&[20, 10, 30], 30);
        let total: usize = result.iter().sum();
        assert_eq!(total, 30, "total should match budget: {result:?}");
        // Wider column should get more space
        assert!(
            result[2] >= result[1],
            "wider column should get more: {result:?}"
        );
    }

    #[test]
    fn horizontal_rule_respects_width() {
        let input = "---\n";
        let result = markdown_to_text(input, Some(20));
        let text = plain_text(&result);
        let rule_line = text.lines().find(|l| l.contains('─'));
        assert!(rule_line.is_some(), "should have rule: {text}");
        let rule = rule_line.unwrap();
        let display_width = rule.chars().count();
        assert!(
            display_width <= 20,
            "rule should respect width (got {} chars): {rule:?}",
            display_width
        );
    }
}
