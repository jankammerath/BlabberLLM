use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::chats::ChatStore;
use crate::settings::AppSettings;

use super::message::{build_message_row, MessageRole};
use super::settings_dialog::present_settings_dialog;
use super::sidebar::SidebarView;

const MAX_CONTENT_WIDTH: i32 = 760;
const TIGHTENING_THRESHOLD: i32 = 560;

pub fn build_window(app: &adw::Application) -> adw::ApplicationWindow {
    let settings = Rc::new(RefCell::new(AppSettings::load()));
    let chat_store = Rc::new(RefCell::new(ChatStore::load()));
    let active_chat_id: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    let title = adw::WindowTitle::new("BlabberLLM", &providers_subtitle(&settings.borrow()));

    let sidebar_toggle_button = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Toggle Sidebar (F9)")
        .active(true)
        .build();

    let header_new_chat_button = gtk::Button::builder()
        .icon_name("chat-message-new-symbolic")
        .tooltip_text("New Chat (Ctrl+N)")
        .action_name("win.new-chat")
        .build();

    let settings_button = gtk::Button::builder()
        .icon_name("preferences-system-symbolic")
        .tooltip_text("Settings (Ctrl+,)")
        .action_name("win.preferences")
        .build();

    let header_bar = adw::HeaderBar::builder().title_widget(&title).build();
    header_bar.pack_start(&sidebar_toggle_button);
    header_bar.pack_start(&header_new_chat_button);
    header_bar.pack_end(&settings_button);

    let status_page = adw::StatusPage::builder()
        .title("Where do we start?")
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

    let content_toolbar_view = adw::ToolbarView::new();
    content_toolbar_view.add_top_bar(&header_bar);
    content_toolbar_view.set_content(Some(&content_stack));
    content_toolbar_view.add_bottom_bar(&composer_bar);

    let split_view = adw::OverlaySplitView::builder()
        .sidebar_position(gtk::PackType::Start)
        .show_sidebar(true)
        .enable_show_gesture(true)
        .enable_hide_gesture(true)
        .min_sidebar_width(240.0)
        .max_sidebar_width(320.0)
        .sidebar_width_fraction(0.28)
        .content(&content_toolbar_view)
        .build();

    split_view
        .bind_property("show-sidebar", &sidebar_toggle_button, "active")
        .bidirectional()
        .sync_create()
        .build();

    split_view
        .bind_property("show-sidebar", &header_new_chat_button, "visible")
        .flags(glib::BindingFlags::SYNC_CREATE | glib::BindingFlags::INVERT_BOOLEAN)
        .build();

    let sidebar_slot: Rc<RefCell<Option<Rc<SidebarView>>>> = Rc::new(RefCell::new(None));

    let reset_to_empty_chat: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        messages_box,
        #[weak]
        content_stack,
        #[weak]
        title,
        #[weak]
        input_view,
        #[strong]
        active_chat_id,
        #[strong]
        sidebar_slot,
        move || {
            *active_chat_id.borrow_mut() = None;
            clear_box(&messages_box);
            content_stack.set_visible_child_name("empty");
            title.set_title("BlabberLLM");
            if let Some(sidebar) = sidebar_slot.borrow().as_ref() {
                sidebar.clear_selection();
            }
            input_view.grab_focus();
        }
    ));

    let open_chat: Rc<dyn Fn(String)> = Rc::new(glib::clone!(
        #[weak]
        messages_box,
        #[weak]
        content_stack,
        #[weak]
        scrolled_window,
        #[weak]
        title,
        #[weak]
        split_view,
        #[weak]
        input_view,
        #[strong]
        chat_store,
        #[strong]
        active_chat_id,
        move |chat_id: String| {
            let store = chat_store.borrow();
            let Some(chat) = store.get(&chat_id) else {
                return;
            };

            *active_chat_id.borrow_mut() = Some(chat.id.clone());
            clear_box(&messages_box);
            for msg in &chat.messages {
                messages_box.append(&build_message_row(msg.role, &msg.content));
            }
            title.set_title(&chat.title);
            content_stack.set_visible_child_name(if chat.messages.is_empty() {
                "empty"
            } else {
                "chat"
            });

            if split_view.is_collapsed() {
                split_view.set_show_sidebar(false);
            }

            scroll_to_bottom(&scrolled_window);
            input_view.grab_focus();
        }
    ));

    let delete_chat: Rc<dyn Fn(String)> = Rc::new(glib::clone!(
        #[strong]
        chat_store,
        #[strong]
        active_chat_id,
        #[strong]
        sidebar_slot,
        #[strong]
        reset_to_empty_chat,
        move |chat_id: String| {
            let was_active = active_chat_id.borrow().as_deref() == Some(chat_id.as_str());
            chat_store.borrow_mut().delete_chat(&chat_id);
            if was_active {
                reset_to_empty_chat();
            }
            if let Some(sidebar) = sidebar_slot.borrow().as_ref() {
                let current_active = active_chat_id.borrow();
                sidebar.refresh(chat_store.borrow().chats(), current_active.as_deref());
            }
        }
    ));

    let sidebar = Rc::new(SidebarView::new(Rc::clone(&open_chat), delete_chat));
    sidebar.refresh(chat_store.borrow().chats(), None);
    split_view.set_sidebar(Some(&sidebar.root));
    *sidebar_slot.borrow_mut() = Some(Rc::clone(&sidebar));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("BlabberLLM")
        .default_width(960)
        .default_height(660)
        .width_request(360)
        .height_request(420)
        .content(&split_view)
        .build();

    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        680.0,
        adw::LengthUnit::Sp,
    ));
    breakpoint.add_setter(&split_view, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(breakpoint);

    let submit_message: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        input_view,
        #[weak]
        messages_box,
        #[weak]
        content_stack,
        #[weak]
        scrolled_window,
        #[weak]
        title,
        #[strong]
        chat_store,
        #[strong]
        active_chat_id,
        #[strong]
        sidebar,
        move || {
            let buffer = input_view.buffer();
            let raw_text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let prompt = raw_text.trim();
            if prompt.is_empty() {
                return;
            }

            let prompt_owned = prompt.to_string();
            buffer.set_text("");

            let chat_id = {
                let mut current = active_chat_id.borrow_mut();
                match current.as_ref() {
                    Some(existing) => existing.clone(),
                    None => {
                        let new_id = chat_store.borrow_mut().create_chat(&prompt_owned);
                        *current = Some(new_id.clone());
                        new_id
                    }
                }
            };

            let reply = format!(
                "No LLM backend is connected yet. Once Ollama or a provider API is configured, BlabberLLM will respond to:\n\n“{prompt_owned}”"
            );

            {
                let mut store = chat_store.borrow_mut();
                store.append_message(&chat_id, MessageRole::User, prompt_owned.clone());
                store.append_message(&chat_id, MessageRole::Assistant, reply.clone());
                if let Some(chat) = store.get(&chat_id) {
                    title.set_title(&chat.title);
                }
                sidebar.refresh(store.chats(), Some(&chat_id));
            }

            content_stack.set_visible_child_name("chat");
            messages_box.append(&build_message_row(MessageRole::User, &prompt_owned));
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

    let update_subtitle: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        title,
        #[strong]
        settings,
        move || {
            title.set_subtitle(&providers_subtitle(&settings.borrow()));
        }
    ));

    let new_chat_action = gio::ActionEntry::builder("new-chat")
        .activate({
            let reset_to_empty_chat = Rc::clone(&reset_to_empty_chat);
            move |_: &adw::ApplicationWindow, _, _| reset_to_empty_chat()
        })
        .build();

    let toggle_sidebar_action = gio::ActionEntry::builder("toggle-sidebar")
        .activate(glib::clone!(
            #[weak]
            split_view,
            move |_: &adw::ApplicationWindow, _, _| {
                split_view.set_show_sidebar(!split_view.shows_sidebar());
            }
        ))
        .build();

    let preferences_action = gio::ActionEntry::builder("preferences")
        .activate({
            let settings = Rc::clone(&settings);
            let update_subtitle = Rc::clone(&update_subtitle);
            move |win: &adw::ApplicationWindow, _, _| {
                present_settings_dialog(win, Rc::clone(&settings), Rc::clone(&update_subtitle));
            }
        })
        .build();

    window.add_action_entries([new_chat_action, toggle_sidebar_action, preferences_action]);

    input_view.grab_focus();

    window
}

fn providers_subtitle(settings: &AppSettings) -> String {
    match settings.configured_provider_count() {
        0 => "No providers configured".to_string(),
        1 => "1 provider configured".to_string(),
        n => format!("{n} providers configured"),
    }
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
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
