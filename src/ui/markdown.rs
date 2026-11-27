use adw::prelude::*;
use gtk::{gdk, glib, pango};

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

pub fn render_markdown_into(container: &gtk::Box, markdown: &str) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let blocks = parse_blocks(markdown);
    if blocks.is_empty() {
        let label = gtk::Label::builder()
            .label(markdown)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .selectable(true)
            .xalign(0.0)
            .build();
        container.append(&label);
        return;
    }

    for block in blocks {
        match block {
            MarkdownBlock::Heading { level, text } => {
                let markup = format_heading_markup(level, &text);
                let label = build_markup_label(&markup);
                label.add_css_class("markdown-heading");
                container.append(&label);
            }
            MarkdownBlock::Paragraph(text) => {
                let markup = inline_to_pango(&text);
                container.append(&build_markup_label(&markup));
            }
            MarkdownBlock::CodeBlock { language, code } => {
                container.append(&build_code_block(&language, &code));
            }
            MarkdownBlock::BulletList(items) => {
                let list_box = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(4)
                    .build();
                list_box.add_css_class("markdown-list");
                for item in items {
                    let row = gtk::Box::builder()
                        .orientation(gtk::Orientation::Horizontal)
                        .spacing(8)
                        .build();
                    let bullet = gtk::Label::builder()
                        .label("•")
                        .valign(gtk::Align::Start)
                        .build();
                    bullet.add_css_class("dim-label");
                    let content = build_markup_label(&inline_to_pango(&item));
                    content.set_hexpand(true);
                    row.append(&bullet);
                    row.append(&content);
                    list_box.append(&row);
                }
                container.append(&list_box);
            }
            MarkdownBlock::OrderedList(items) => {
                let list_box = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(4)
                    .build();
                list_box.add_css_class("markdown-list");
                for (num, item) in items {
                    let row = gtk::Box::builder()
                        .orientation(gtk::Orientation::Horizontal)
                        .spacing(8)
                        .build();
                    let prefix = gtk::Label::builder()
                        .label(format!("{num}."))
                        .valign(gtk::Align::Start)
                        .build();
                    prefix.add_css_class("dim-label");
                    let content = build_markup_label(&inline_to_pango(&item));
                    content.set_hexpand(true);
                    row.append(&prefix);
                    row.append(&content);
                    list_box.append(&row);
                }
                container.append(&list_box);
            }
            MarkdownBlock::BlockQuote(text) => {
                let quote_box = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .build();
                quote_box.add_css_class("markdown-quote");
                let label = build_markup_label(&inline_to_pango(&text));
                label.add_css_class("dim-label");
                quote_box.append(&label);
                container.append(&quote_box);
            }
            MarkdownBlock::HorizontalRule => {
                let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
                sep.set_margin_top(4);
                sep.set_margin_bottom(4);
                container.append(&sep);
            }
        }
    }
}

fn build_markup_label(markup: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .use_markup(true)
        .build();
    label.set_markup(markup);
    label
}

fn build_code_block(language: &str, code: &str) -> gtk::Box {
    let wrapper = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    wrapper.add_css_class("markdown-code-block");

    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    header.add_css_class("markdown-code-header");

    let lang_label = gtk::Label::builder()
        .label(if language.is_empty() {
            "code"
        } else {
            language
        })
        .xalign(0.0)
        .hexpand(true)
        .build();
    lang_label.add_css_class("dim-label");
    lang_label.add_css_class("caption");

    let copy_button = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy code")
        .valign(gtk::Align::Center)
        .build();
    copy_button.add_css_class("flat");
    copy_button.add_css_class("circular");
    copy_button.add_css_class("code-copy-button");

    let code_owned = code.to_string();
    copy_button.connect_clicked(move |btn| {
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(&code_owned);
            btn.set_icon_name("object-select-symbolic");
            glib::timeout_add_local_once(
                std::time::Duration::from_millis(1200),
                glib::clone!(
                    #[weak]
                    btn,
                    move || {
                        btn.set_icon_name("edit-copy-symbolic");
                    }
                ),
            );
        }
    });

    header.append(&lang_label);
    header.append(&copy_button);

    let code_label = gtk::Label::builder()
        .label(code)
        .selectable(true)
        .xalign(0.0)
        .build();
    code_label.add_css_class("monospace");
    code_label.add_css_class("markdown-code-body");

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&code_label)
        .build();

    wrapper.append(&header);
    wrapper.append(&scroller);
    wrapper
}

fn format_heading_markup(level: u8, text: &str) -> String {
    let inner = inline_to_pango(text);
    let size = match level {
        1 => "x-large",
        2 => "large",
        3 => "medium",
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
                    out.push_str("<a href=\"");
                    out.push_str(&escape_pango(safe_url));
                    out.push_str("\">");
                    out.push_str(&inline_to_pango(&label));
                    out.push_str("</a>");
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
        if i + 1 < len && ((chars[i] == '*' && chars[i + 1] == '*') || (chars[i] == '_' && chars[i + 1] == '_')) {
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
            "See <a href=\"https://rust-lang.org\">Rust</a> and <s>old</s>"
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
