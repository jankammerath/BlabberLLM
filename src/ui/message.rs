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

pub fn build_message_row(role: MessageRole, text: &str) -> gtk::Box {
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

    match role {
        MessageRole::User => {
            bubble.set_halign(gtk::Align::End);
            bubble.set_margin_start(64);
            bubble.add_css_class("user-bubble");
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
        }
    }

    let content_label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .build();

    bubble.append(&content_label);
    row.append(&bubble);
    row
}
