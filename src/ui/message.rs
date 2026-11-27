use adw::prelude::*;
use gtk::pango;

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

pub struct StreamingMessageHandle {
    pub row: gtk::Box,
    role_label: gtk::Label,
    content_label: gtk::Label,
}

impl StreamingMessageHandle {
    pub fn set_content(&self, text: &str) {
        self.content_label.remove_css_class("dim-label");
        self.content_label.set_label(text);
    }

    pub fn set_error(&self, message: &str) {
        self.role_label.remove_css_class("dim-label");
        self.role_label.add_css_class("error");
        self.role_label.set_label("Error");
        self.content_label.remove_css_class("dim-label");
        self.content_label.set_label(message);
    }
}

pub fn build_message_row(role: MessageRole, text: &str) -> gtk::Box {
    build_message_row_with_label(role, "Assistant", text, false).0
}

pub fn build_streaming_assistant_row(model_label: &str) -> StreamingMessageHandle {
    let (row, role_label, content_label) =
        build_message_row_with_label(MessageRole::Assistant, model_label, "Thinking…", true);
    StreamingMessageHandle {
        row,
        role_label: role_label.expect("assistant rows always have a role label"),
        content_label,
    }
}

fn build_message_row_with_label(
    role: MessageRole,
    assistant_label: &str,
    text: &str,
    dim_initial_content: bool,
) -> (gtk::Box, Option<gtk::Label>, gtk::Label) {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .build();

    let bubble = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .build();
    bubble.add_css_class("message-bubble");

    let role_label = match role {
        MessageRole::User => {
            bubble.set_halign(gtk::Align::End);
            bubble.set_margin_start(64);
            bubble.add_css_class("user-bubble");
            None
        }
        MessageRole::Assistant => {
            bubble.set_halign(gtk::Align::Start);
            bubble.set_margin_end(64);
            bubble.add_css_class("card");
            bubble.add_css_class("assistant-bubble");

            let label = gtk::Label::builder()
                .label(assistant_label)
                .xalign(0.0)
                .ellipsize(pango::EllipsizeMode::End)
                .build();
            label.add_css_class("dim-label");
            label.add_css_class("message-role");
            bubble.append(&label);
            Some(label)
        }
    };

    let content_label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .build();
    if dim_initial_content {
        content_label.add_css_class("dim-label");
    }

    bubble.append(&content_label);
    row.append(&bubble);
    (row, role_label, content_label)
}
