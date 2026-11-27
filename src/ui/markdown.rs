use adw::prelude::*;
use gtk::{gdk, pango};

#[derive(Debug, PartialEq, Eq)]
enum MarkdownBlock {
    Heading { level: u8, text: String },
    Paragraph(String),
    CodeBlock { language: String, code: String },
    BulletList(Vec<String>),
    OrderedList(Vec<(String, String)>),
    BlockQuote(String),
    HorizontalRule,
}

/// Builds a read-only `gtk::TextView` that supports continuous text selection
/// across the entire rendered Markdown response (headings, paragraphs, lists,
/// blockquotes, and fenced code blocks).
pub fn build_markdown_view() -> gtk::TextView {
    let buffer = gtk::TextBuffer::new(None);
    register_tags(&buffer);

    let view = gtk::TextView::builder()
        .buffer(&buffer)
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .pixels_above_lines(1)
        .pixels_below_lines(6)
        .pixels_inside_wrap(5)
        .hexpand(true)
        .build();
    view.add_css_class("markdown-view");
    view
}

pub fn set_markdown_content(view: &gtk::TextView, markdown: &str) {
    let buffer = view.buffer();
    buffer.set_text("");

    let blocks = parse_blocks(markdown);
    if blocks.is_empty() {
        buffer.set_text(markdown);
        return;
    }

    let mut iter = buffer.end_iter();

    for (idx, block) in blocks.iter().enumerate() {
        if idx > 0 {
            buffer.insert(&mut iter, "\n");
        }

        match block {
            MarkdownBlock::Heading { level, text } => {
                let start_offset = iter.offset();
                let markup = format_heading_markup(*level, text);
                buffer.insert_markup(&mut iter, &markup);
                let s = buffer.iter_at_offset(start_offset);
                buffer.apply_tag_by_name("heading", &s, &iter);
            }
            MarkdownBlock::Paragraph(text) => {
                let start_offset = iter.offset();
                let markup = inline_to_pango(text);
                buffer.insert_markup(&mut iter, &markup);
                let s = buffer.iter_at_offset(start_offset);
                buffer.apply_tag_by_name("paragraph", &s, &iter);
            }
            MarkdownBlock::CodeBlock { language, code } => {
                let start_offset = iter.offset();
                if !language.is_empty() {
                    let header_start = iter.offset();
                    buffer.insert(&mut iter, &format!("{language}\n"));
                    let s = buffer.iter_at_offset(header_start);
                    buffer.apply_tag_by_name("code_lang", &s, &iter);
                }
                buffer.insert(&mut iter, code);
                let s = buffer.iter_at_offset(start_offset);
                buffer.apply_tag_by_name("code_block", &s, &iter);
            }
            MarkdownBlock::BulletList(items) => {
                for (item_idx, item) in items.iter().enumerate() {
                    if item_idx > 0 {
                        buffer.insert(&mut iter, "\n");
                    }
                    let start_offset = iter.offset();
                    buffer.insert(&mut iter, "• ");
                    let markup = inline_to_pango(item);
                    buffer.insert_markup(&mut iter, &markup);
                    let s = buffer.iter_at_offset(start_offset);
                    buffer.apply_tag_by_name("list_item", &s, &iter);
                }
            }
            MarkdownBlock::OrderedList(items) => {
                for (item_idx, (num, item)) in items.iter().enumerate() {
                    if item_idx > 0 {
                        buffer.insert(&mut iter, "\n");
                    }
                    let start_offset = iter.offset();
                    buffer.insert(&mut iter, &format!("{num}. "));
                    let markup = inline_to_pango(item);
                    buffer.insert_markup(&mut iter, &markup);
                    let s = buffer.iter_at_offset(start_offset);
                    buffer.apply_tag_by_name("list_item", &s, &iter);
                }
            }
            MarkdownBlock::BlockQuote(text) => {
                let start_offset = iter.offset();
                let markup = inline_to_pango(text);
                buffer.insert_markup(&mut iter, &markup);
                let s = buffer.iter_at_offset(start_offset);
                buffer.apply_tag_by_name("quote", &s, &iter);
            }
            MarkdownBlock::HorizontalRule => {
                let start_offset = iter.offset();
                buffer.insert(&mut iter, "────────────────────────");
                let s = buffer.iter_at_offset(start_offset);
                buffer.apply_tag_by_name("dim", &s, &iter);
            }
        }
    }
}

pub fn set_plain_dim_content(view: &gtk::TextView, text: &str) {
    let buffer = view.buffer();
    buffer.set_text(text);
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    buffer.apply_tag_by_name("dim", &start, &end);
}

pub fn set_plain_content(view: &gtk::TextView, text: &str) {
    view.buffer().set_text(text);
}

fn register_tags(buffer: &gtk::TextBuffer) {
    let tag_table = buffer.tag_table();

    let heading_tag = gtk::TextTag::builder()
        .name("heading")
        .pixels_above_lines(10)
        .pixels_below_lines(6)
        .build();
    tag_table.add(&heading_tag);

    let paragraph_tag = gtk::TextTag::builder()
        .name("paragraph")
        .pixels_above_lines(2)
        .pixels_below_lines(10)
        .pixels_inside_wrap(6)
        .build();
    tag_table.add(&paragraph_tag);

    let code_bg = gdk::RGBA::new(0.5, 0.5, 0.5, 0.14);
    let code_block_tag = gtk::TextTag::builder()
        .name("code_block")
        .family("monospace")
        .scale(0.92)
        .left_margin(12)
        .right_margin(12)
        .pixels_above_lines(3)
        .pixels_below_lines(3)
        .pixels_inside_wrap(3)
        .paragraph_background_rgba(&code_bg)
        .build();
    tag_table.add(&code_block_tag);

    let code_lang_tag = gtk::TextTag::builder()
        .name("code_lang")
        .scale(0.82)
        .weight(600)
        .pixels_below_lines(4)
        .build();
    tag_table.add(&code_lang_tag);

    let list_item_tag = gtk::TextTag::builder()
        .name("list_item")
        .left_margin(18)
        .indent(-14)
        .pixels_above_lines(2)
        .pixels_below_lines(8)
        .pixels_inside_wrap(6)
        .build();
    tag_table.add(&list_item_tag);

    let quote_bg = gdk::RGBA::new(0.5, 0.5, 0.5, 0.08);
    let quote_tag = gtk::TextTag::builder()
        .name("quote")
        .left_margin(16)
        .right_margin(10)
        .pixels_above_lines(4)
        .pixels_below_lines(10)
        .pixels_inside_wrap(6)
        .style(pango::Style::Italic)
        .paragraph_background_rgba(&quote_bg)
        .build();
    tag_table.add(&quote_tag);

    let dim_tag = gtk::TextTag::builder()
        .name("dim")
        .foreground_rgba(&gdk::RGBA::new(0.55, 0.55, 0.55, 1.0))
        .build();
    tag_table.add(&dim_tag);
}

fn format_heading_markup(level: u8, text: &str) -> String {
    let inner = inline_to_pango(text);
    let size = match level {
        1 => "x-large",
        2 => "large",
        _ => "medium",
    };
    format!("<span size=\"{size}\" weight=\"bold\">{inner}</span>")
}

fn parse_blocks(input: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Fenced code block (supports unclosed blocks during streaming)
        if let Some(rest) = trimmed.strip_prefix("```") {
            let language = rest.trim().to_string();
            i += 1;
            let mut code_lines = Vec::new();
            while i < lines.len() {
                if lines[i].trim().starts_with("```") {
                    i += 1;
                    break;
                }
                code_lines.push(lines[i]);
                i += 1;
            }
            blocks.push(MarkdownBlock::CodeBlock {
                language,
                code: code_lines.join("\n"),
            });
            continue;
        }

        // Horizontal rule
        if is_horizontal_rule(trimmed) {
            blocks.push(MarkdownBlock::HorizontalRule);
            i += 1;
            continue;
        }

        // Heading
        if let Some((level, text)) = parse_heading(trimmed) {
            blocks.push(MarkdownBlock::Heading {
                level,
                text: text.to_string(),
            });
            i += 1;
            continue;
        }

        // Blockquote
        if trimmed.starts_with('>') {
            let mut quote_lines = Vec::new();
            while i < lines.len() {
                let t = lines[i].trim();
                if let Some(stripped) = t.strip_prefix('>') {
                    quote_lines.push(stripped.trim_start());
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(MarkdownBlock::BlockQuote(quote_lines.join("\n")));
            continue;
        }

        // Bullet list
        if parse_bullet_item(trimmed).is_some() {
            let mut items = Vec::new();
            while i < lines.len() {
                if let Some(item) = parse_bullet_item(lines[i].trim()) {
                    items.push(item.to_string());
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(MarkdownBlock::BulletList(items));
            continue;
        }

        // Ordered list
        if parse_ordered_item(trimmed).is_some() {
            let mut items = Vec::new();
            while i < lines.len() {
                if let Some((num, item)) = parse_ordered_item(lines[i].trim()) {
                    items.push((num.to_string(), item.to_string()));
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(MarkdownBlock::OrderedList(items));
            continue;
        }

        // Paragraph
        let mut para_lines = Vec::new();
        while i < lines.len() {
            let t = lines[i].trim();
            if t.is_empty()
                || t.starts_with("```")
                || is_horizontal_rule(t)
                || parse_heading(t).is_some()
                || t.starts_with('>')
                || parse_bullet_item(t).is_some()
                || parse_ordered_item(t).is_some()
            {
                break;
            }
            para_lines.push(lines[i]);
            i += 1;
        }
        if !para_lines.is_empty() {
            blocks.push(MarkdownBlock::Paragraph(para_lines.join("\n")));
        }
    }

    blocks
}

fn is_horizontal_rule(line: &str) -> bool {
    let chars: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    chars.len() >= 3 && chars.iter().all(|&c| c == '-' || c == '*' || c == '_') && {
        let first = chars[0];
        chars.iter().all(|&c| c == first)
    }
}

fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&hashes) {
        let rest = &line[hashes..];
        if let Some(text) = rest.strip_prefix(' ') {
            return Some((hashes as u8, text.trim()));
        }
    }
    None
}

fn parse_bullet_item(line: &str) -> Option<&str> {
    line.strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
}

fn parse_ordered_item(line: &str) -> Option<(&str, &str)> {
    let digits = line.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let rest = &line[digits..];
    let item = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))?;
    Some((&line[..digits], item))
}

pub fn inline_to_pango(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Inline code: `code`
        if chars[i] == '`' {
            if let Some(end) = find_closing_char(&chars, i + 1, '`') {
                let code: String = chars[i + 1..end].iter().collect();
                out.push_str("<tt>");
                out.push_str(&escape_pango(&code));
                out.push_str("</tt>");
                i = end + 1;
                continue;
            }
        }

        // Markdown link: [label](url)
        if chars[i] == '[' {
            if let Some((label, url, next_i)) = parse_link(&chars, i) {
                let safe_url = url.trim();
                if safe_url.starts_with("http://")
                    || safe_url.starts_with("https://")
                    || safe_url.starts_with("mailto:")
                {
                    out.push_str("<u>");
                    out.push_str(&inline_to_pango(&label));
                    out.push_str("</u> (");
                    out.push_str(&escape_pango(safe_url));
                    out.push(')');
                    i = next_i;
                    continue;
                }
            }
        }

        // Bold + Italic: ***text***
        if i + 2 < len && chars[i] == '*' && chars[i + 1] == '*' && chars[i + 2] == '*' {
            if let Some(end) = find_closing_seq(&chars, i + 3, &['*', '*', '*']) {
                let inner: String = chars[i + 3..end].iter().collect();
                out.push_str("<b><i>");
                out.push_str(&inline_to_pango(&inner));
                out.push_str("</i></b>");
                i = end + 3;
                continue;
            }
        }

        // Bold: **text** or __text__
        if i + 1 < len
            && ((chars[i] == '*' && chars[i + 1] == '*')
                || (chars[i] == '_' && chars[i + 1] == '_'))
        {
            let marker = chars[i];
            if let Some(end) = find_closing_seq(&chars, i + 2, &[marker, marker]) {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str("<b>");
                out.push_str(&inline_to_pango(&inner));
                out.push_str("</b>");
                i = end + 2;
                continue;
            }
        }

        // Strikethrough: ~~text~~
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            if let Some(end) = find_closing_seq(&chars, i + 2, &['~', '~']) {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str("<s>");
                out.push_str(&inline_to_pango(&inner));
                out.push_str("</s>");
                i = end + 2;
                continue;
            }
        }

        // Italic: *text*
        if chars[i] == '*' && (i + 1 < len && !chars[i + 1].is_whitespace()) {
            if let Some(end) = find_closing_char(&chars, i + 1, '*') {
                if end > i + 1 && !chars[end - 1].is_whitespace() {
                    let inner: String = chars[i + 1..end].iter().collect();
                    out.push_str("<i>");
                    out.push_str(&inline_to_pango(&inner));
                    out.push_str("</i>");
                    i = end + 1;
                    continue;
                }
            }
        }

        push_escaped_char(&mut out, chars[i]);
        i += 1;
    }

    out
}

fn find_closing_char(chars: &[char], start: usize, target: char) -> Option<usize> {
    (start..chars.len()).find(|&idx| chars[idx] == target)
}

fn find_closing_seq(chars: &[char], start: usize, seq: &[char]) -> Option<usize> {
    if seq.is_empty() || start + seq.len() > chars.len() {
        return None;
    }
    (start..=chars.len() - seq.len()).find(|&idx| &chars[idx..idx + seq.len()] == seq)
}

fn parse_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let close_bracket = find_closing_char(chars, start + 1, ']')?;
    if close_bracket + 1 >= chars.len() || chars[close_bracket + 1] != '(' {
        return None;
    }
    let close_paren = find_closing_char(chars, close_bracket + 2, ')')?;
    let label: String = chars[start + 1..close_bracket].iter().collect();
    let url: String = chars[close_bracket + 2..close_paren].iter().collect();
    Some((label, url, close_paren + 1))
}

fn escape_pango(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        push_escaped_char(&mut escaped, ch);
    }
    escaped
}

fn push_escaped_char(out: &mut String, ch: char) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&apos;"),
        other => out.push(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_inline_markdown_to_safe_pango() {
        assert_eq!(
            inline_to_pango("Hello **world** & `a < b` *italic*"),
            "Hello <b>world</b> &amp; <tt>a &lt; b</tt> <i>italic</i>"
        );
        assert_eq!(
            inline_to_pango("See [Rust](https://rust-lang.org) and ~~old~~"),
            "See <u>Rust</u> (https://rust-lang.org) and <s>old</s>"
        );
    }

    #[test]
    fn parses_markdown_blocks_including_unclosed_code() {
        let md = "# Title\n\nParagraph text\n\n- one\n- two\n\n```rust\nfn main() {}\n";
        let blocks = parse_blocks(md);
        assert_eq!(blocks.len(), 4);
        assert_eq!(
            blocks[0],
            MarkdownBlock::Heading {
                level: 1,
                text: "Title".into()
            }
        );
        assert_eq!(blocks[1], MarkdownBlock::Paragraph("Paragraph text".into()));
        assert_eq!(
            blocks[2],
            MarkdownBlock::BulletList(vec!["one".into(), "two".into()])
        );
        assert_eq!(
            blocks[3],
            MarkdownBlock::CodeBlock {
                language: "rust".into(),
                code: "fn main() {}".into()
            }
        );
    }
}
