use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib, pango};

use crate::chats::{ChatAttachment, ToolCallRecord};

use super::markdown::{
    build_markdown_view, set_markdown_content, set_plain_content, set_plain_dim_content,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

impl MessageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "user" => Some(MessageRole::User),
            "assistant" => Some(MessageRole::Assistant),
            _ => None,
        }
    }
}

pub fn format_worked_duration(duration_ms: u64) -> String {
    let total_secs = duration_ms / 1000;
    if total_secs == 0 {
        return "Worked for <1s".to_string();
    }
    if total_secs < 60 {
        return format!("Worked for {total_secs}s");
    }
    let total_mins = total_secs / 60;
    let rem_secs = total_secs % 60;
    if total_mins < 60 {
        if rem_secs == 0 {
            if total_mins == 1 {
                "Worked for 1 min".to_string()
            } else {
                format!("Worked for {total_mins} mins")
            }
        } else {
            format!("Worked for {total_mins}m {rem_secs}s")
        }
    } else {
        let hours = total_mins / 60;
        let rem_mins = total_mins % 60;
        if rem_mins == 0 {
            if hours == 1 {
                "Worked for 1 hr".to_string()
            } else {
                format!("Worked for {hours} hrs")
            }
        } else {
            format!("Worked for {hours}h {rem_mins}m")
        }
    }
}

pub fn format_working_duration(duration_ms: u64) -> String {
    let total_secs = duration_ms / 1000;
    if total_secs == 0 {
        "Working…".to_string()
    } else if total_secs < 60 {
        format!("Working… ({total_secs}s)")
    } else {
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("Working… ({mins}m {secs}s)")
    }
}

struct ToolCardEntry {
    command: String,
    status_label: gtk::Label,
    container: gtk::Box,
}

pub struct StreamingMessageHandle {
    pub row: gtk::Box,
    status_label: gtk::Label,
    tools_box: gtk::Box,
    markdown_view: gtk::Box,
    raw_text: Rc<RefCell<String>>,
    tool_cards: RefCell<Vec<ToolCardEntry>>,
}

impl StreamingMessageHandle {
    pub fn set_content(&self, markdown: &str) {
        *self.raw_text.borrow_mut() = markdown.to_string();
        set_markdown_content(&self.markdown_view, markdown);
    }

    pub fn set_plain_status(&self, text: &str) {
        *self.raw_text.borrow_mut() = text.to_string();
        set_plain_dim_content(&self.markdown_view, text);
    }

    pub fn set_working_elapsed(&self, duration_ms: u64) {
        self.status_label
            .set_label(&format_working_duration(duration_ms));
    }

    pub fn set_finished_elapsed(&self, duration_ms: u64) {
        self.status_label
            .set_label(&format_worked_duration(duration_ms));
    }

    pub fn add_tool_started(&self, command: &str, working_dir: Option<&str>) {
        self.tools_box.set_visible(true);
        let entry = build_tool_card(command, working_dir, "Running…", None);
        self.tools_box.append(&entry.container);
        self.tool_cards.borrow_mut().push(entry);
    }

    pub fn update_tool_finished(
        &self,
        command: &str,
        exit_code: Option<i32>,
        output: &str,
        denied: bool,
    ) {
        self.tools_box.set_visible(true);
        let status_text = format_tool_status(exit_code, denied);

        let mut cards = self.tool_cards.borrow_mut();
        if let Some(existing) = cards.iter().rev().find(|c| c.command == command) {
            existing.status_label.set_label(&status_text);
            apply_tool_status_style(&existing.status_label, exit_code, denied);
            append_tool_output(&existing.container, output);
        } else {
            let entry = build_tool_card(command, None, &status_text, Some(output));
            apply_tool_status_style(&entry.status_label, exit_code, denied);
            self.tools_box.append(&entry.container);
            cards.push(entry);
        }
    }

    pub fn set_error(&self, message: &str) {
        self.status_label.remove_css_class("dim-label");
        self.status_label.add_css_class("error");
        self.status_label.set_label("Error");
        *self.raw_text.borrow_mut() = message.to_string();
        set_plain_content(&self.markdown_view, message);
    }
}

fn format_tool_status(exit_code: Option<i32>, denied: bool) -> String {
    if denied {
        "Denied".to_string()
    } else if let Some(code) = exit_code {
        format!("Exit {code}")
    } else {
        "Done".to_string()
    }
}

fn apply_tool_status_style(label: &gtk::Label, exit_code: Option<i32>, denied: bool) {
    label.remove_css_class("dim-label");
    if denied || exit_code.unwrap_or(0) != 0 {
        label.add_css_class("error");
    } else {
        label.add_css_class("success");
    }
}

fn build_tool_card(
    command: &str,
    working_dir: Option<&str>,
    status: &str,
    output: Option<&str>,
) -> ToolCardEntry {
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    container.add_css_class("tool-card");

    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();

    let icon = gtk::Image::builder()
        .icon_name("utilities-terminal-symbolic")
        .valign(gtk::Align::Start)
        .margin_top(2)
        .build();
    icon.add_css_class("dim-label");

    let cmd_text = if let Some(dir) = working_dir {
        format!("$ {command}  ({dir})")
    } else {
        format!("$ {command}")
    };
    let cmd_label = gtk::Label::builder()
        .label(&cmd_text)
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .hexpand(true)
        .selectable(true)
        .build();
    cmd_label.add_css_class("monospace");
    cmd_label.add_css_class("caption");

    let status_label = gtk::Label::builder()
        .label(status)
        .valign(gtk::Align::Start)
        .build();
    status_label.add_css_class("caption");
    status_label.add_css_class("dim-label");

    header.append(&icon);
    header.append(&cmd_label);
    header.append(&status_label);
    container.append(&header);

    if let Some(out) = output {
        append_tool_output(&container, out);
    }

    ToolCardEntry {
        command: command.to_string(),
        status_label,
        container,
    }
}

fn append_tool_output(container: &gtk::Box, output: &str) {
    let out_label = gtk::Label::builder()
        .label(output)
        .xalign(0.0)
        .selectable(true)
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .build();
    out_label.add_css_class("monospace");
    out_label.add_css_class("tool-output-body");

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(180)
        .child(&out_label)
        .build();

    let expander = gtk::Expander::builder()
        .label("Output")
        .expanded(false)
        .child(&scroller)
        .build();
    expander.add_css_class("caption");
    container.append(&expander);
}

fn build_copy_button(text_source: Rc<RefCell<String>>) -> gtk::Button {
    let copy_button = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy response")
        .valign(gtk::Align::Center)
        .build();
    copy_button.add_css_class("flat");
    copy_button.add_css_class("circular");
    copy_button.add_css_class("code-copy-button");

    copy_button.connect_clicked(move |btn| {
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(&text_source.borrow());
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

    copy_button
}

pub fn build_message_row(
    role: MessageRole,
    text: &str,
    attachments: &[ChatAttachment],
    tool_calls: &[ToolCallRecord],
    duration_ms: Option<u64>,
) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .build();

    match role {
        MessageRole::User => {
            let bubble = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(6)
                .hexpand(true)
                .halign(gtk::Align::End)
                .margin_start(64)
                .build();
            bubble.add_css_class("message-bubble");
            bubble.add_css_class("user-bubble");

            if !attachments.is_empty() {
                let att_row = gtk::Box::builder()
                    .orientation(gtk::Orientation::Horizontal)
                    .spacing(6)
                    .halign(gtk::Align::Start)
                    .build();
                for att in attachments {
                    let pill = gtk::Box::builder()
                        .orientation(gtk::Orientation::Horizontal)
                        .spacing(5)
                        .halign(gtk::Align::Start)
                        .hexpand(false)
                        .build();
                    pill.add_css_class("user-attachment-pill");

                    let icon_name = if att.is_image {
                        "image-x-generic-symbolic"
                    } else {
                        "text-x-generic-symbolic"
                    };
                    let icon = gtk::Image::from_icon_name(icon_name);
                    let name_label = gtk::Label::builder()
                        .label(&att.name)
                        .ellipsize(pango::EllipsizeMode::Middle)
                        .max_width_chars(24)
                        .build();
                    name_label.add_css_class("caption");
                    pill.append(&icon);
                    pill.append(&name_label);
                    att_row.append(&pill);
                }
                bubble.append(&att_row);
            }

            if !text.trim().is_empty() {
                let content_label = gtk::Label::builder()
                    .label(text)
                    .wrap(true)
                    .wrap_mode(pango::WrapMode::WordChar)
                    .selectable(true)
                    .xalign(0.0)
                    .build();
                bubble.append(&content_label);
            }
            row.append(&bubble);
        }
        MessageRole::Assistant => {
            let container = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(6)
                .hexpand(true)
                .build();
            container.add_css_class("assistant-response");

            let header = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(6)
                .build();

            let status_text = duration_ms.map(format_worked_duration).unwrap_or_default();
            let status_label = gtk::Label::builder()
                .label(&status_text)
                .xalign(0.0)
                .hexpand(true)
                .build();
            status_label.add_css_class("dim-label");
            status_label.add_css_class("message-role");

            let raw_text = Rc::new(RefCell::new(text.to_string()));
            let copy_button = build_copy_button(raw_text);

            header.append(&status_label);
            header.append(&copy_button);
            container.append(&header);

            if !tool_calls.is_empty() {
                let tools_box = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(6)
                    .build();
                for tool in tool_calls {
                    let status = format_tool_status(tool.exit_code, tool.denied);
                    let entry = build_tool_card(
                        &tool.command,
                        tool.working_directory.as_deref(),
                        &status,
                        Some(&tool.output),
                    );
                    apply_tool_status_style(&entry.status_label, tool.exit_code, tool.denied);
                    tools_box.append(&entry.container);
                }
                container.append(&tools_box);
            }

            if !text.trim().is_empty() {
                let markdown_view = build_markdown_view();
                set_markdown_content(&markdown_view, text);
                container.append(&markdown_view);
            }

            row.append(&container);
        }
    }

    row
}

pub fn build_streaming_assistant_row() -> StreamingMessageHandle {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .build();

    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .hexpand(true)
        .build();
    container.add_css_class("assistant-response");

    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();

    let status_label = gtk::Label::builder()
        .label("Working…")
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    status_label.add_css_class("dim-label");
    status_label.add_css_class("message-role");

    let raw_text = Rc::new(RefCell::new(String::new()));
    let copy_button = build_copy_button(Rc::clone(&raw_text));

    header.append(&status_label);
    header.append(&copy_button);
    container.append(&header);

    let tools_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .visible(false)
        .build();
    container.append(&tools_box);

    let markdown_view = build_markdown_view();
    container.append(&markdown_view);

    row.append(&container);

    let handle = StreamingMessageHandle {
        row,
        status_label,
        tools_box,
        markdown_view,
        raw_text,
        tool_cards: RefCell::new(Vec::new()),
    };
    handle.set_plain_status("Thinking…");
    handle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_worked_durations() {
        assert_eq!(format_worked_duration(400), "Worked for <1s");
        assert_eq!(format_worked_duration(4_200), "Worked for 4s");
        assert_eq!(format_worked_duration(60_000), "Worked for 1 min");
        assert_eq!(format_worked_duration(180_000), "Worked for 3 mins");
        assert_eq!(format_worked_duration(125_000), "Worked for 2m 5s");
    }
}
