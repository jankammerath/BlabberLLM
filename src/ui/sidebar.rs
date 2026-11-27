use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::pango;

use crate::chats::ChatSession;

pub struct SidebarView {
    pub root: adw::ToolbarView,
    list_box: gtk::ListBox,
    stack: gtk::Stack,
    suppress_selection: Rc<Cell<bool>>,
    on_delete: Rc<dyn Fn(String)>,
}

impl SidebarView {
    pub fn new(on_select: Rc<dyn Fn(String)>, on_delete: Rc<dyn Fn(String)>) -> Self {
        let title = adw::WindowTitle::new("Chats", "");

        let new_chat_button = gtk::Button::builder()
            .icon_name("chat-message-new-symbolic")
            .tooltip_text("New Chat (Ctrl+N)")
            .action_name("win.new-chat")
            .build();

        let header_bar = adw::HeaderBar::builder().title_widget(&title).build();
        header_bar.pack_start(&new_chat_button);

        let list_box = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .vexpand(true)
            .build();
        list_box.add_css_class("navigation-sidebar");

        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list_box)
            .build();

        let empty_page = adw::StatusPage::builder()
            .icon_name("chat-message-new-symbolic")
            .title("No Chats Yet")
            .description("Previous conversations will appear here.")
            .vexpand(true)
            .build();
        empty_page.add_css_class("compact");

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .vexpand(true)
            .build();
        stack.add_named(&empty_page, Some("empty"));
        stack.add_named(&scrolled, Some("list"));
        stack.set_visible_child_name("empty");

        let root = adw::ToolbarView::new();
        root.add_top_bar(&header_bar);
        root.set_content(Some(&stack));

        let suppress_selection = Rc::new(Cell::new(false));
        list_box.connect_row_selected({
            let suppress_selection = Rc::clone(&suppress_selection);
            let on_select = Rc::clone(&on_select);
            move |_, row| {
                if suppress_selection.get() {
                    return;
                }
                if let Some(row) = row {
                    let chat_id = row.widget_name().to_string();
                    if !chat_id.is_empty() {
                        on_select(chat_id);
                    }
                }
            }
        });

        Self {
            root,
            list_box,
            stack,
            suppress_selection,
            on_delete,
        }
    }

    pub fn refresh(&self, chats: &[ChatSession], active_id: Option<&str>) {
        self.suppress_selection.set(true);

        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }

        if chats.is_empty() {
            self.stack.set_visible_child_name("empty");
            self.suppress_selection.set(false);
            return;
        }

        self.stack.set_visible_child_name("list");

        let mut row_to_select: Option<gtk::ListBoxRow> = None;

        for chat in chats {
            let row = build_chat_row(chat, Rc::clone(&self.on_delete));
            if active_id == Some(chat.id.as_str()) {
                row_to_select = Some(row.clone());
            }
            self.list_box.append(&row);
        }

        if let Some(row) = row_to_select {
            self.list_box.select_row(Some(&row));
        } else {
            self.list_box.unselect_all();
        }

        self.suppress_selection.set(false);
    }

    pub fn clear_selection(&self) {
        self.suppress_selection.set(true);
        self.list_box.unselect_all();
        self.suppress_selection.set(false);
    }
}

fn build_chat_row(chat: &ChatSession, on_delete: Rc<dyn Fn(String)>) -> gtk::ListBoxRow {
    let title_label = gtk::Label::builder()
        .label(&chat.title)
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .single_line_mode(true)
        .hexpand(true)
        .build();

    let subtitle_label = gtk::Label::builder()
        .label(chat.subtitle())
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .single_line_mode(true)
        .build();
    subtitle_label.add_css_class("dim-label");
    subtitle_label.add_css_class("caption");

    let text_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    text_box.append(&title_label);
    text_box.append(&subtitle_label);

    let delete_button = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Delete chat")
        .valign(gtk::Align::Center)
        .build();
    delete_button.add_css_class("flat");
    delete_button.add_css_class("circular");
    delete_button.add_css_class("sidebar-delete-button");

    let chat_id = chat.id.clone();
    delete_button.connect_clicked(move |_| {
        on_delete(chat_id.clone());
    });

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    content.add_css_class("sidebar-chat-row");
    content.append(&text_box);
    content.append(&delete_button);

    let row = gtk::ListBoxRow::builder()
        .child(&content)
        .build();
    row.set_widget_name(&chat.id);
    row
}
