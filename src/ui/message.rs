use std::cell::RefCell;

use adw::prelude::*;
use gtk::pango;

use super::markdown::render_markdown_into;

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

struct ToolCardEntry {
    command: String,
    status_label: gtk::Label,
    container: gtk::Box,
}

pub struct StreamingMessageHandle {
    pub row: gtk::Box,
    role_label: gtk::Label,
    tools_box: gtk::Box,
    content_box: gtk::Box,
    tool_cards: RefCell<Vec<ToolCardEntry>>,
}

impl StreamingMessageHandle {
    pub fn set_content(&self, markdown: &str) {
        render_markdown_into(&self.content_box, markdown);
    }

    pub fn set_plain_status(&self, text: &str) {
        while let Some(child) = self.content_box.first_child() {
            self.content_box.remove(&child);
        }
        let label = gtk::Label::builder()
            .label(text)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .selectable(true)
            .xalign(0.0)
            .build();
        label.add_css_class("dim-label");
        self.content_box.append(&label);
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
        let status_text = if denied {
            "Denied".to_string()
        } else if let Some(code) = exit_code {
            format!("Exit {code}")
        } else {
            "Done".to_string()
        };

        let mut cards = self.tool_cards.borrow_mut();
        if let Some(existing) = cards.iter().rev().find(|c| c.command == command) {
            existing.status_label.set_label(&status_text);
            existing.status_label.remove_css_class("dim-label");
            if denied || exit_code.unwrap_or(0) != 0 {
                existing.status_label.add_css_class("error");
            } else {
                existing.status_label.add_css_class("success");
            }
            append_tool_output(&existing.container, output);
        } else {
            let entry = build_tool_card(command, None, &status_text, Some(output));
            if denied || exit_code.unwrap_or(0) != 0 {
                entry.status_label.add_css_class("error");
            } else {
                entry.status_label.add_css_class("success");
            }
            self.tools_box.append(&entry.container);
            cards.push(entry);
        }
    }

    pub fn set_error(&self, message: &str) {
        self.role_label.remove_css_class("dim-label");
        self.role_label.add_css_class("error");
        self.role_label.set_label("Error");

        while let Some(child) = self.content_box.first_child() {
            self.content_box.remove(&child);
        }
        let label = gtk::Label::builder()
            .label(message)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .selectable(true)
            .xalign(0.0)
            .build();
        self.content_box.append(&label);
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

    let icon = gtk::Image::from_icon_name("utilities-terminal-symbolic");
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

    let status_label = gtk::Label::builder().label(status).build();
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

pub fn build_message_row(role: MessageRole, text: &str) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .build();

    let bubble = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .hexpand(true)
        .build();
    bubble.add_css_class("message-bubble");

    match role {
        MessageRole::User => {
            bubble.set_halign(gtk::Align::End);
            bubble.set_margin_start(64);
            bubble.add_css_class("user-bubble");

            let content_label = gtk::Label::builder()
                .label(text)
                .wrap(true)
                .wrap_mode(pango::WrapMode::WordChar)
                .selectable(true)
                .xalign(0.0)
                .build();
            bubble.append(&content_label);
        }
        MessageRole::Assistant => {
            bubble.set_halign(gtk::Align::Start);
            bubble.set_margin_end(64);
            bubble.add_css_class("card");
            bubble.add_css_class("assistant-bubble");

            let role_label = gtk::Label::builder()
                .label("Assistant")
                .xalign(0.0)
                .build();
            role_label.add_css_class("dim-label");
            role_label.add_css_class("message-role");
            bubble.append(&role_label);

            let content_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(6)
                .build();
            render_markdown_into(&content_box, text);
            bubble.append(&content_box);
        }
    }

    row.append(&bubble);
    row
}

pub fn build_streaming_assistant_row(model_label: &str) -> StreamingMessageHandle {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .build();

    let bubble = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .hexpand(true)
        .build();
    bubble.add_css_class("message-bubble");
    bubble.set_halign(gtk::Align::Start);
    bubble.set_margin_end(64);
    bubble.add_css_class("card");
    bubble.add_css_class("assistant-bubble");

    let role_label = gtk::Label::builder()
        .label(model_label)
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    role_label.add_css_class("dim-label");
    role_label.add_css_class("message-role");
    bubble.append(&role_label);

    let tools_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .visible(false)
        .build();
    bubble.append(&tools_box);

    let content_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    bubble.append(&content_box);

    row.append(&bubble);

    let handle = StreamingMessageHandle {
        row,
        role_label,
        tools_box,
        content_box,
        tool_cards: RefCell::new(Vec::new()),
    };
    handle.set_plain_status("Thinking…");
    handle
}
