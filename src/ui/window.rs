use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use super::message::{build_message_row, MessageRole};

const MAX_CONTENT_WIDTH: i32 = 760;
const TIGHTENING_THRESHOLD: i32 = 560;

pub fn build_window(app: &adw::Application) -> adw::ApplicationWindow {
    let title = adw::WindowTitle::new("BlabberLLM", "Ready");

    let new_chat_button = gtk::Button::builder()
        .icon_name("chat-message-new-symbolic")
        .tooltip_text("New Chat (Ctrl+N)")
        .action_name("win.new-chat")
        .build();

    let header_bar = adw::HeaderBar::builder().title_widget(&title).build();
    header_bar.pack_start(&new_chat_button);

    let status_page = adw::StatusPage::builder()
        .icon_name("chat-message-new-symbolic")
        .title("BlabberLLM")
        .description("Type a message below to start chatting. Ollama and cloud LLM providers can be wired in next.")
        .vexpand(true)
        .build();

    let messages_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .vexpand(true)
        .build();
    messages_box.add_css_class("chat-messages");

    let messages_clamp = adw::Clamp::builder()
        .maximum_size(MAX_CONTENT_WIDTH)
        .tightening_threshold(TIGHTENING_THRESHOLD)
        .child(&messages_box)
        .build();

    let scrolled_window = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&messages_clamp)
        .build();

    let content_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vexpand(true)
        .build();
    content_stack.add_named(&status_page, Some("empty"));
    content_stack.add_named(&scrolled_window, Some("chat"));
    content_stack.set_visible_child_name("empty");

    let (composer_bar, input_view, send_button) = build_composer();

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&content_stack));
    toolbar_view.add_bottom_bar(&composer_bar);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("BlabberLLM")
        .default_width(820)
        .default_height(640)
        .width_request(360)
        .height_request(420)
        .content(&toolbar_view)
        .build();

    let submit_message: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        input_view,
        #[weak]
        messages_box,
        #[weak]
        content_stack,
        #[weak]
        scrolled_window,
        move || {
            let buffer = input_view.buffer();
            let raw_text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let prompt = raw_text.trim();
            if prompt.is_empty() {
                return;
            }

            let prompt_owned = prompt.to_string();
            buffer.set_text("");

            content_stack.set_visible_child_name("chat");
            messages_box.append(&build_message_row(MessageRole::User, &prompt_owned));

            let reply = format!(
                "No LLM backend is connected yet. Once Ollama or a provider API is configured, BlabberLLM will respond to:\n\n“{prompt_owned}”"
            );
            messages_box.append(&build_message_row(MessageRole::Assistant, &reply));

            scroll_to_bottom(&scrolled_window);
            input_view.grab_focus();
        }
    ));

    send_button.connect_clicked({
        let submit_message = Rc::clone(&submit_message);
        move |_| submit_message()
    });

    let key_controller = gtk::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    key_controller.connect_key_pressed({
        let submit_message = Rc::clone(&submit_message);
        move |_, key, _, modifiers| {
            let is_enter = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter);
            if is_enter && !modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
                submit_message();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    input_view.add_controller(key_controller);

    let new_chat_action = gio::ActionEntry::builder("new-chat")
        .activate(glib::clone!(
            #[weak]
            messages_box,
            #[weak]
            content_stack,
            #[weak]
            input_view,
            move |_: &adw::ApplicationWindow, _, _| {
                while let Some(child) = messages_box.first_child() {
                    messages_box.remove(&child);
                }
                content_stack.set_visible_child_name("empty");
                input_view.grab_focus();
            }
        ))
        .build();
    window.add_action_entries([new_chat_action]);

    input_view.grab_focus();

    window
}

fn build_composer() -> (gtk::Box, gtk::TextView, gtk::Button) {
    let input_view = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(6)
        .bottom_margin(6)
        .left_margin(4)
        .right_margin(4)
        .hexpand(true)
        .build();
    input_view.add_css_class("composer-input");

    let input_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(140)
        .hexpand(true)
        .child(&input_view)
        .build();

    let placeholder = gtk::Label::builder()
        .label("Message BlabberLLM…")
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .can_target(false)
        .build();
    placeholder.add_css_class("dim-label");
    placeholder.add_css_class("composer-placeholder");

    let input_overlay = gtk::Overlay::builder()
        .hexpand(true)
        .valign(gtk::Align::Center)
        .child(&input_scroll)
        .build();
    input_overlay.add_overlay(&placeholder);

    let send_button = gtk::Button::builder()
        .icon_name("go-up-symbolic")
        .tooltip_text("Send message (Enter)")
        .valign(gtk::Align::End)
        .sensitive(false)
        .build();
    send_button.add_css_class("suggested-action");
    send_button.add_css_class("circular");

    let buffer = input_view.buffer();
    buffer.connect_changed(glib::clone!(
        #[weak]
        placeholder,
        #[weak]
        send_button,
        move |buf| {
            let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
            placeholder.set_visible(text.is_empty());
            send_button.set_sensitive(!text.trim().is_empty());
        }
    ));

    let composer_card = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .build();
    composer_card.add_css_class("card");
    composer_card.add_css_class("composer-card");
    composer_card.append(&input_overlay);
    composer_card.append(&send_button);

    let composer_clamp = adw::Clamp::builder()
        .maximum_size(MAX_CONTENT_WIDTH)
        .tightening_threshold(TIGHTENING_THRESHOLD)
        .child(&composer_card)
        .build();

    let composer_bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    composer_bar.add_css_class("composer-bar");
    composer_bar.append(&composer_clamp);

    (composer_bar, input_view, send_button)
}

fn scroll_to_bottom(scrolled_window: &gtk::ScrolledWindow) {
    glib::idle_add_local_once(glib::clone!(
        #[weak]
        scrolled_window,
        move || {
            let adj = scrolled_window.vadjustment();
            let target = (adj.upper() - adj.page_size()).max(adj.lower());
            adj.set_value(target);
        }
    ));
}
