use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, gio, glib, pango};

use crate::chats::{ChatAttachment, ChatStore, ToolCallRecord};
use crate::llm::{self, ChatStreamEvent, ModelOption, ShellToolConfig, ShellToolRequest};
use crate::settings::{AppSettings, ProviderAccount};

use super::message::{build_message_row, build_streaming_assistant_row, MessageRole};
use super::settings_dialog::present_settings_dialog;
use super::sidebar::SidebarView;

const MAX_CONTENT_WIDTH: i32 = 760;
const TIGHTENING_THRESHOLD: i32 = 560;

enum StreamChunk {
    Delta(String),
    ToolStarted {
        command: String,
        working_directory: Option<String>,
    },
    ToolFinished {
        command: String,
        exit_code: Option<i32>,
        output: String,
        denied: bool,
    },
    ApprovalRequest {
        request: ShellToolRequest,
        reply_tx: mpsc::Sender<bool>,
    },
    Finished(Result<String, String>),
}

struct ComposerWidgets {
    bar: gtk::Box,
    card: gtk::Box,
    attachments_box: gtk::Box,
    input_view: gtk::TextView,
    model_dropdown: gtk::DropDown,
    refresh_models_button: gtk::Button,
    tools_toggle_button: gtk::ToggleButton,
    attach_button: gtk::Button,
    send_button: gtk::Button,
}

pub fn build_window(app: &adw::Application) -> adw::ApplicationWindow {
    let settings = Rc::new(RefCell::new(AppSettings::load()));
    let chat_store = Rc::new(RefCell::new(ChatStore::load()));
    let active_chat_id: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let model_options: Rc<RefCell<Vec<ModelOption>>> = Rc::new(RefCell::new(Vec::new()));
    let pending_attachments: Rc<RefCell<Vec<ChatAttachment>>> = Rc::new(RefCell::new(Vec::new()));
    let suppress_model_selection = Rc::new(Cell::new(false));
    let suppress_tools_toggle = Rc::new(Cell::new(false));
    let refresh_generation = Rc::new(Cell::new(0u64));
    let active_stream_cancel: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::new(RefCell::new(None));

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

    let composer = build_composer(settings.borrow().shell_tool_enabled());
    let composer_card = composer.card.clone();
    let attachments_box = composer.attachments_box.clone();
    let input_view = composer.input_view.clone();
    let model_dropdown = composer.model_dropdown.clone();
    let refresh_models_button = composer.refresh_models_button.clone();
    let tools_toggle_button = composer.tools_toggle_button.clone();
    let attach_button = composer.attach_button.clone();
    let send_button = composer.send_button.clone();

    let content_toolbar_view = adw::ToolbarView::new();
    content_toolbar_view.add_top_bar(&header_bar);
    content_toolbar_view.set_content(Some(&content_stack));
    content_toolbar_view.add_bottom_bar(&composer.bar);

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

    let update_send_button_state: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        input_view,
        #[weak]
        send_button,
        #[strong]
        model_options,
        #[strong]
        pending_attachments,
        #[strong]
        active_stream_cancel,
        move || {
            let is_streaming = active_stream_cancel.borrow().is_some();
            if is_streaming {
                send_button.set_icon_name("media-playback-stop-symbolic");
                send_button.set_tooltip_text(Some("Stop generating"));
                send_button.remove_css_class("suggested-action");
                send_button.add_css_class("destructive-action");
                send_button.set_sensitive(true);
            } else {
                send_button.set_icon_name("go-up-symbolic");
                send_button.set_tooltip_text(Some("Send message (Enter)"));
                send_button.remove_css_class("destructive-action");
                send_button.add_css_class("suggested-action");
                let buffer = input_view.buffer();
                let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
                let has_content = !text.trim().is_empty() || !pending_attachments.borrow().is_empty();
                let has_model = !model_options.borrow().is_empty();
                send_button.set_sensitive(has_content && has_model);
            }
        }
    ));

    let refresh_attachments_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_attachment_chips: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        attachments_box,
        #[strong]
        pending_attachments,
        #[strong]
        update_send_button_state,
        #[strong]
        refresh_attachments_slot,
        move || {
            while let Some(child) = attachments_box.first_child() {
                attachments_box.remove(&child);
            }
            let items = pending_attachments.borrow().clone();
            attachments_box.set_visible(!items.is_empty());

            for (idx, att) in items.iter().enumerate() {
                let chip = gtk::Box::builder()
                    .orientation(gtk::Orientation::Horizontal)
                    .spacing(6)
                    .halign(gtk::Align::Start)
                    .hexpand(false)
                    .build();
                chip.add_css_class("attachment-chip");

                let icon_name = if att.is_image {
                    "image-x-generic-symbolic"
                } else {
                    "text-x-generic-symbolic"
                };
                let icon = gtk::Image::from_icon_name(icon_name);
                icon.add_css_class("dim-label");

                let label = gtk::Label::builder()
                    .label(&att.name)
                    .ellipsize(pango::EllipsizeMode::Middle)
                    .max_width_chars(24)
                    .build();
                label.add_css_class("caption");

                let remove_btn = gtk::Button::builder()
                    .icon_name("window-close-symbolic")
                    .tooltip_text(format!("Remove {}", att.name))
                    .valign(gtk::Align::Center)
                    .build();
                remove_btn.add_css_class("flat");
                remove_btn.add_css_class("circular");
                remove_btn.add_css_class("attachment-remove-button");

                remove_btn.connect_clicked({
                    let pending_attachments = Rc::clone(&pending_attachments);
                    let refresh_attachments_slot = Rc::clone(&refresh_attachments_slot);
                    move |_| {
                        {
                            let mut list = pending_attachments.borrow_mut();
                            if idx < list.len() {
                                list.remove(idx);
                            }
                        }
                        if let Some(refresh) = refresh_attachments_slot.borrow().as_ref() {
                            refresh();
                        }
                    }
                });

                chip.append(&icon);
                chip.append(&label);
                chip.append(&remove_btn);
                attachments_box.append(&chip);
            }

            update_send_button_state();
        }
    ));
    *refresh_attachments_slot.borrow_mut() = Some(Rc::clone(&refresh_attachment_chips));

    let cancel_active_stream: Rc<dyn Fn()> = Rc::new({
        let active_stream_cancel = Rc::clone(&active_stream_cancel);
        let update_send_button_state = Rc::clone(&update_send_button_state);
        move || {
            if let Some(flag) = active_stream_cancel.borrow_mut().take() {
                flag.store(true, Ordering::Relaxed);
            }
            update_send_button_state();
        }
    });

    input_view.buffer().connect_changed({
        let update_send_button_state = Rc::clone(&update_send_button_state);
        move |_| update_send_button_state()
    });

    let select_model_for_current_context: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        model_dropdown,
        #[strong]
        settings,
        #[strong]
        chat_store,
        #[strong]
        active_chat_id,
        #[strong]
        model_options,
        #[strong]
        suppress_model_selection,
        #[strong]
        update_send_button_state,
        move || {
            let options = model_options.borrow();
            if options.is_empty() {
                return;
            }

            let chat_pref = active_chat_id.borrow().as_deref().and_then(|id| {
                let store = chat_store.borrow();
                let chat = store.get(id)?;
                Some((chat.last_account_id.clone()?, chat.last_model.clone()?))
            });

            let default_pref = settings
                .borrow()
                .default_model()
                .map(|(a, m)| (a.to_string(), m.to_string()));

            let find_index = |(acc_id, model): &(String, String)| {
                options
                    .iter()
                    .position(|opt| opt.account_id == *acc_id && opt.model == *model)
            };

            let target_idx = chat_pref
                .as_ref()
                .and_then(find_index)
                .or_else(|| default_pref.as_ref().and_then(find_index))
                .unwrap_or(0);

            suppress_model_selection.set(true);
            model_dropdown.set_selected(target_idx as u32);
            suppress_model_selection.set(false);
            update_send_button_state();
        }
    ));

    let apply_model_options: Rc<dyn Fn(Vec<ModelOption>)> = Rc::new(glib::clone!(
        #[weak]
        model_dropdown,
        #[strong]
        model_options,
        #[strong]
        suppress_model_selection,
        #[strong]
        update_send_button_state,
        #[strong]
        select_model_for_current_context,
        move |options: Vec<ModelOption>| {
            suppress_model_selection.set(true);

            if options.is_empty() {
                let placeholder = gtk::StringList::new(&["No models — configure Settings"]);
                model_dropdown.set_model(Some(&placeholder));
                model_dropdown.set_selected(0);
                model_dropdown.set_sensitive(false);
                *model_options.borrow_mut() = Vec::new();
                suppress_model_selection.set(false);
                update_send_button_state();
                return;
            }

            let labels: Vec<String> = options.iter().map(ModelOption::display_label).collect();
            let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let list = gtk::StringList::new(&label_refs);
            model_dropdown.set_model(Some(&list));
            model_dropdown.set_sensitive(true);
            *model_options.borrow_mut() = options;
            suppress_model_selection.set(false);

            select_model_for_current_context();
        }
    ));

    model_dropdown.connect_selected_notify({
        let chat_store = Rc::clone(&chat_store);
        let active_chat_id = Rc::clone(&active_chat_id);
        let model_options = Rc::clone(&model_options);
        let suppress_model_selection = Rc::clone(&suppress_model_selection);
        let update_send_button_state = Rc::clone(&update_send_button_state);
        move |dropdown| {
            if suppress_model_selection.get() {
                return;
            }
            let idx = dropdown.selected() as usize;
            if let Some(chosen) = model_options.borrow().get(idx) {
                if let Some(chat_id) = active_chat_id.borrow().as_deref() {
                    chat_store.borrow_mut().set_chat_model(
                        chat_id,
                        chosen.account_id.clone(),
                        chosen.model.clone(),
                    );
                }
            }
            update_send_button_state();
        }
    });

    let refresh_models: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        refresh_models_button,
        #[strong]
        settings,
        #[strong]
        refresh_generation,
        #[strong]
        apply_model_options,
        move || {
            let generation = refresh_generation.get().wrapping_add(1);
            refresh_generation.set(generation);

            let configured_accounts: Vec<ProviderAccount> = settings
                .borrow()
                .accounts()
                .iter()
                .filter(|a| a.is_configured())
                .cloned()
                .collect();

            if configured_accounts.is_empty() {
                refresh_models_button.set_sensitive(false);
                apply_model_options(Vec::new());
                return;
            }

            // Populate fallback models immediately so the dropdown is usable right away.
            let initial_fallbacks = build_fallback_options(&configured_accounts);
            if !initial_fallbacks.is_empty() {
                apply_model_options(initial_fallbacks);
            }

            refresh_models_button.set_sensitive(false);
            let (tx, rx) = mpsc::channel::<Vec<ModelOption>>();

            thread::spawn(move || {
                let mut fetched_options = Vec::new();
                for account in configured_accounts {
                    match llm::fetch_models(&account) {
                        Ok(models) if !models.is_empty() => {
                            for model in models {
                                fetched_options.push(ModelOption {
                                    account_id: account.id.clone(),
                                    account_name: account.name.clone(),
                                    kind: account.kind,
                                    model,
                                });
                            }
                        }
                        _ => {
                            for &model in llm::fallback_models(account.kind) {
                                fetched_options.push(ModelOption {
                                    account_id: account.id.clone(),
                                    account_name: account.name.clone(),
                                    kind: account.kind,
                                    model: model.to_string(),
                                });
                            }
                        }
                    }
                }
                let _ = tx.send(fetched_options);
            });

            let refresh_generation_clone = Rc::clone(&refresh_generation);
            let apply_model_options_clone = Rc::clone(&apply_model_options);
            glib::timeout_add_local(
                Duration::from_millis(50),
                glib::clone!(
                    #[weak]
                    refresh_models_button,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || match rx.try_recv() {
                        Ok(options) => {
                            if refresh_generation_clone.get() == generation {
                                apply_model_options_clone(options);
                                refresh_models_button.set_sensitive(true);
                            }
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => {
                            if refresh_generation_clone.get() != generation {
                                glib::ControlFlow::Break
                            } else {
                                glib::ControlFlow::Continue
                            }
                        }
                        Err(mpsc::TryRecvError::Disconnected) => {
                            if refresh_generation_clone.get() == generation {
                                refresh_models_button.set_sensitive(true);
                            }
                            glib::ControlFlow::Break
                        }
                    }
                ),
            );
        }
    ));

    refresh_models_button.connect_clicked({
        let refresh_models = Rc::clone(&refresh_models);
        move |_| refresh_models()
    });

    let sync_tools_toggle_for_current_context: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        tools_toggle_button,
        #[strong]
        settings,
        #[strong]
        chat_store,
        #[strong]
        active_chat_id,
        #[strong]
        suppress_tools_toggle,
        move || {
            let default_enabled = settings.borrow().shell_tool_enabled();
            let enabled = active_chat_id
                .borrow()
                .as_deref()
                .and_then(|id| chat_store.borrow().get(id).and_then(|c| c.tool_use_enabled))
                .unwrap_or(default_enabled);

            suppress_tools_toggle.set(true);
            tools_toggle_button.set_active(enabled);
            update_tools_toggle_tooltip(&tools_toggle_button, enabled);
            suppress_tools_toggle.set(false);
        }
    ));

    tools_toggle_button.connect_toggled({
        let chat_store = Rc::clone(&chat_store);
        let active_chat_id = Rc::clone(&active_chat_id);
        let suppress_tools_toggle = Rc::clone(&suppress_tools_toggle);
        move |btn| {
            let enabled = btn.is_active();
            update_tools_toggle_tooltip(btn, enabled);
            if suppress_tools_toggle.get() {
                return;
            }
            if let Some(chat_id) = active_chat_id.borrow().as_deref() {
                chat_store.borrow_mut().set_chat_tool_use(chat_id, enabled);
            }
        }
    });

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
        pending_attachments,
        #[strong]
        refresh_attachment_chips,
        #[strong]
        sidebar_slot,
        #[strong]
        cancel_active_stream,
        #[strong]
        select_model_for_current_context,
        #[strong]
        sync_tools_toggle_for_current_context,
        move || {
            cancel_active_stream();
            *active_chat_id.borrow_mut() = None;
            pending_attachments.borrow_mut().clear();
            refresh_attachment_chips();
            clear_box(&messages_box);
            content_stack.set_visible_child_name("empty");
            title.set_title("BlabberLLM");
            if let Some(sidebar) = sidebar_slot.borrow().as_ref() {
                sidebar.clear_selection();
            }
            select_model_for_current_context();
            sync_tools_toggle_for_current_context();
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
        #[strong]
        cancel_active_stream,
        #[strong]
        select_model_for_current_context,
        #[strong]
        sync_tools_toggle_for_current_context,
        move |chat_id: String| {
            cancel_active_stream();
            let store = chat_store.borrow();
            let Some(chat) = store.get(&chat_id) else {
                return;
            };

            *active_chat_id.borrow_mut() = Some(chat.id.clone());
            clear_box(&messages_box);
            for msg in &chat.messages {
                messages_box.append(&build_message_row(
                    msg.role,
                    &msg.content,
                    &msg.attachments,
                    &msg.tool_calls,
                    msg.duration_ms,
                ));
            }
            title.set_title(&chat.title);
            content_stack.set_visible_child_name(if chat.messages.is_empty() {
                "empty"
            } else {
                "chat"
            });
            drop(store);

            select_model_for_current_context();
            sync_tools_toggle_for_current_context();

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

    let attach_paths: Rc<dyn Fn(Vec<PathBuf>)> = Rc::new(glib::clone!(
        #[weak]
        window,
        #[strong]
        pending_attachments,
        #[strong]
        refresh_attachment_chips,
        move |paths: Vec<PathBuf>| {
            let mut errors = Vec::new();
            {
                let mut list = pending_attachments.borrow_mut();
                for path in paths {
                    let path_str = path.to_string_lossy();
                    if list.iter().any(|a| a.path == path_str) {
                        continue;
                    }
                    match ChatAttachment::from_path(&path) {
                        Ok(att) => list.push(att),
                        Err(err) => errors.push(err),
                    }
                }
            }
            refresh_attachment_chips();

            if !errors.is_empty() {
                let alert = adw::AlertDialog::builder()
                    .heading("Could Not Attach File")
                    .body(errors.join("\n"))
                    .default_response("ok")
                    .close_response("ok")
                    .build();
                alert.add_responses(&[("ok", "OK")]);
                alert.present(Some(&window));
            }
        }
    ));

    attach_button.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[strong]
        attach_paths,
        move |_| {
            let file_dialog = gtk::FileDialog::builder()
                .title("Attach Files")
                .modal(true)
                .build();
            let attach_paths = Rc::clone(&attach_paths);
            file_dialog.open_multiple(
                Some(&window),
                gio::Cancellable::NONE,
                move |result| {
                    if let Ok(list_model) = result {
                        let mut paths = Vec::new();
                        for i in 0..list_model.n_items() {
                            if let Some(obj) = list_model.item(i) {
                                if let Ok(file) = obj.downcast::<gio::File>() {
                                    if let Some(path) = file.path() {
                                        paths.push(path);
                                    }
                                }
                            }
                        }
                        if !paths.is_empty() {
                            attach_paths(paths);
                        }
                    }
                },
            );
        }
    ));

    let drop_target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    drop_target.connect_drop({
        let attach_paths = Rc::clone(&attach_paths);
        move |_, value, _, _| {
            if let Ok(file_list) = value.get::<gdk::FileList>() {
                let paths: Vec<PathBuf> = file_list
                    .files()
                    .into_iter()
                    .filter_map(|f| f.path())
                    .collect();
                if !paths.is_empty() {
                    attach_paths(paths);
                    return true;
                }
            }
            false
        }
    });
    composer_card.add_controller(drop_target);

    let submit_message: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        window,
        #[weak]
        input_view,
        #[weak]
        model_dropdown,
        #[weak]
        tools_toggle_button,
        #[weak]
        messages_box,
        #[weak]
        content_stack,
        #[weak]
        scrolled_window,
        #[weak]
        title,
        #[strong]
        settings,
        #[strong]
        chat_store,
        #[strong]
        active_chat_id,
        #[strong]
        model_options,
        #[strong]
        pending_attachments,
        #[strong]
        refresh_attachment_chips,
        #[strong]
        active_stream_cancel,
        #[strong]
        update_send_button_state,
        #[strong]
        sidebar,
        move || {
            if active_stream_cancel.borrow().is_some() {
                return;
            }

            let buffer = input_view.buffer();
            let raw_text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let prompt = raw_text.trim();
            if prompt.is_empty() && pending_attachments.borrow().is_empty() {
                return;
            }

            let selected_option = {
                let idx = model_dropdown.selected() as usize;
                model_options.borrow().get(idx).cloned()
            };
            let Some(option) = selected_option else {
                return;
            };
            let session_tools_enabled = tools_toggle_button.is_active();
            let (account, tool_config) = {
                let guard = settings.borrow();
                let Some(acc) = guard.get(&option.account_id).cloned() else {
                    return;
                };
                let cfg = ShellToolConfig {
                    enabled: session_tools_enabled,
                    permission_mode: guard.shell_permission_mode(),
                    sudo_enabled: guard.shell_sudo_enabled(),
                    max_steps: guard.max_tool_steps(),
                };
                (acc, cfg)
            };

            let prompt_owned = prompt.to_string();
            let attachments: Vec<ChatAttachment> =
                std::mem::take(&mut *pending_attachments.borrow_mut());
            buffer.set_text("");
            refresh_attachment_chips();

            let title_seed = if !prompt_owned.is_empty() {
                prompt_owned.clone()
            } else if let Some(first) = attachments.first() {
                format!("File: {}", first.name)
            } else {
                "New Chat".to_string()
            };

            let chat_id = {
                let mut current = active_chat_id.borrow_mut();
                match current.as_ref() {
                    Some(existing) => {
                        let mut store = chat_store.borrow_mut();
                        store.set_chat_model(
                            existing,
                            option.account_id.clone(),
                            option.model.clone(),
                        );
                        store.set_chat_tool_use(existing, session_tools_enabled);
                        existing.clone()
                    }
                    None => {
                        let new_id = chat_store.borrow_mut().create_chat(
                            &title_seed,
                            Some(option.account_id.clone()),
                            Some(option.model.clone()),
                            Some(session_tools_enabled),
                        );
                        *current = Some(new_id.clone());
                        new_id
                    }
                }
            };

            let history = {
                let mut store = chat_store.borrow_mut();
                store.append_message_with_attachments(
                    &chat_id,
                    MessageRole::User,
                    prompt_owned.clone(),
                    attachments.clone(),
                );
                if let Some(chat) = store.get(&chat_id) {
                    title.set_title(&chat.title);
                }
                sidebar.refresh(store.chats(), Some(&chat_id));
                store
                    .get(&chat_id)
                    .map(|c| c.messages.clone())
                    .unwrap_or_default()
            };

            content_stack.set_visible_child_name("chat");
            messages_box.append(&build_message_row(
                MessageRole::User,
                &prompt_owned,
                &attachments,
                &[],
                None,
            ));

            let assistant_handle = build_streaming_assistant_row();
            messages_box.append(&assistant_handle.row);
            scroll_to_bottom(&scrolled_window);

            let started_at = Instant::now();

            let cancel_flag = Arc::new(AtomicBool::new(false));
            *active_stream_cancel.borrow_mut() = Some(Arc::clone(&cancel_flag));
            update_send_button_state();

            let (tx, rx) = mpsc::channel::<StreamChunk>();
            let model_name = option.model.clone();
            let worker_cancel = Arc::clone(&cancel_flag);
            thread::spawn(move || {
                let result = llm::stream_chat_with_tools(
                    &account,
                    &model_name,
                    &history,
                    &tool_config,
                    &worker_cancel,
                    |event| match event {
                        ChatStreamEvent::Delta(delta) => {
                            let _ = tx.send(StreamChunk::Delta(delta));
                        }
                        ChatStreamEvent::ToolStarted {
                            command,
                            working_directory,
                        } => {
                            let _ = tx.send(StreamChunk::ToolStarted {
                                command,
                                working_directory,
                            });
                        }
                        ChatStreamEvent::ToolFinished {
                            command,
                            exit_code,
                            output,
                            denied,
                        } => {
                            let _ = tx.send(StreamChunk::ToolFinished {
                                command,
                                exit_code,
                                output,
                                denied,
                            });
                        }
                    },
                    |request| {
                        let (reply_tx, reply_rx) = mpsc::channel::<bool>();
                        if tx
                            .send(StreamChunk::ApprovalRequest {
                                request: request.clone(),
                                reply_tx,
                            })
                            .is_err()
                        {
                            return false;
                        }
                        loop {
                            if worker_cancel.load(Ordering::Relaxed) {
                                return false;
                            }
                            match reply_rx.recv_timeout(Duration::from_millis(100)) {
                                Ok(allowed) => return allowed,
                                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                                Err(mpsc::RecvTimeoutError::Disconnected) => return false,
                            }
                        }
                    },
                );
                let _ = tx.send(StreamChunk::Finished(result));
            });

            let accumulated = Rc::new(RefCell::new(String::new()));
            let recorded_tools: Rc<RefCell<Vec<ToolCallRecord>>> = Rc::new(RefCell::new(Vec::new()));
            let chat_store_clone = Rc::clone(&chat_store);
            let active_chat_id_clone = Rc::clone(&active_chat_id);
            let active_stream_cancel_clone = Rc::clone(&active_stream_cancel);
            let update_send_button_clone = Rc::clone(&update_send_button_state);
            let sidebar_clone = Rc::clone(&sidebar);
            let chat_id_for_stream = chat_id.clone();

            glib::timeout_add_local(
                Duration::from_millis(25),
                glib::clone!(
                    #[weak]
                    window,
                    #[weak]
                    scrolled_window,
                    #[weak]
                    input_view,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        let mut text_changed = false;
                        loop {
                            match rx.try_recv() {
                                Ok(StreamChunk::Delta(delta)) => {
                                    accumulated.borrow_mut().push_str(&delta);
                                    text_changed = true;
                                }
                                Ok(StreamChunk::ToolStarted {
                                    command,
                                    working_directory,
                                }) => {
                                    recorded_tools.borrow_mut().push(ToolCallRecord {
                                        command: command.clone(),
                                        working_directory: working_directory.clone(),
                                        exit_code: None,
                                        output: String::new(),
                                        denied: false,
                                    });
                                    if active_chat_id_clone.borrow().as_deref()
                                        == Some(chat_id_for_stream.as_str())
                                    {
                                        assistant_handle.add_tool_started(
                                            &command,
                                            working_directory.as_deref(),
                                        );
                                        scroll_to_bottom(&scrolled_window);
                                    }
                                }
                                Ok(StreamChunk::ToolFinished {
                                    command,
                                    exit_code,
                                    output,
                                    denied,
                                }) => {
                                    {
                                        let mut tools = recorded_tools.borrow_mut();
                                        if let Some(existing) = tools
                                            .iter_mut()
                                            .rev()
                                            .find(|t| t.command == command && t.exit_code.is_none() && !t.denied)
                                        {
                                            existing.exit_code = exit_code;
                                            existing.output = output.clone();
                                            existing.denied = denied;
                                        } else {
                                            tools.push(ToolCallRecord {
                                                command: command.clone(),
                                                working_directory: None,
                                                exit_code,
                                                output: output.clone(),
                                                denied,
                                            });
                                        }
                                    }
                                    if active_chat_id_clone.borrow().as_deref()
                                        == Some(chat_id_for_stream.as_str())
                                    {
                                        assistant_handle.update_tool_finished(
                                            &command,
                                            exit_code,
                                            &output,
                                            denied,
                                        );
                                        scroll_to_bottom(&scrolled_window);
                                    }
                                }
                                Ok(StreamChunk::ApprovalRequest { request, reply_tx }) => {
                                    present_shell_approval_dialog(&window, &request, reply_tx);
                                }
                                Ok(StreamChunk::Finished(result)) => {
                                    let is_same_stream = active_stream_cancel_clone
                                        .borrow()
                                        .as_ref()
                                        .map(|f| Arc::ptr_eq(f, &cancel_flag))
                                        .unwrap_or(false);
                                    if is_same_stream {
                                        *active_stream_cancel_clone.borrow_mut() = None;
                                    }
                                    update_send_button_clone();

                                    let still_viewing = active_chat_id_clone.borrow().as_deref()
                                        == Some(chat_id_for_stream.as_str());

                                    match result {
                                        Ok(final_text) => {
                                            let elapsed_ms = started_at.elapsed().as_millis() as u64;
                                            let tools_snapshot = recorded_tools.borrow().clone();
                                            if !final_text.trim().is_empty()
                                                || !tools_snapshot.is_empty()
                                            {
                                                let mut store = chat_store_clone.borrow_mut();
                                                store.append_message_with_tools(
                                                    &chat_id_for_stream,
                                                    MessageRole::Assistant,
                                                    final_text.clone(),
                                                    tools_snapshot,
                                                    Some(elapsed_ms),
                                                );
                                                let current_active = active_chat_id_clone.borrow();
                                                sidebar_clone.refresh(
                                                    store.chats(),
                                                    current_active.as_deref(),
                                                );
                                                if still_viewing {
                                                    assistant_handle.set_finished_elapsed(elapsed_ms);
                                                    if !final_text.trim().is_empty() {
                                                        assistant_handle.set_content(&final_text);
                                                    } else {
                                                        assistant_handle.set_plain_status("");
                                                    }
                                                    scroll_to_bottom(&scrolled_window);
                                                }
                                            } else if still_viewing {
                                                assistant_handle.set_finished_elapsed(elapsed_ms);
                                                if cancel_flag.load(Ordering::Relaxed) {
                                                    assistant_handle
                                                        .set_plain_status("Generation stopped.");
                                                } else {
                                                    assistant_handle.set_error(
                                                        "The provider returned an empty response.",
                                                    );
                                                }
                                            }
                                        }
                                        Err(err) => {
                                            if still_viewing {
                                                assistant_handle.set_error(&err);
                                                scroll_to_bottom(&scrolled_window);
                                            }
                                        }
                                    }

                                    if still_viewing {
                                        input_view.grab_focus();
                                    }
                                    return glib::ControlFlow::Break;
                                }
                                Err(mpsc::TryRecvError::Empty) => {
                                    if active_chat_id_clone.borrow().as_deref()
                                        == Some(chat_id_for_stream.as_str())
                                    {
                                        assistant_handle.set_working_elapsed(
                                            started_at.elapsed().as_millis() as u64,
                                        );
                                        if text_changed {
                                            assistant_handle.set_content(&accumulated.borrow());
                                            scroll_to_bottom(&scrolled_window);
                                        }
                                    }
                                    return glib::ControlFlow::Continue;
                                }
                                Err(mpsc::TryRecvError::Disconnected) => {
                                    let is_same_stream = active_stream_cancel_clone
                                        .borrow()
                                        .as_ref()
                                        .map(|f| Arc::ptr_eq(f, &cancel_flag))
                                        .unwrap_or(false);
                                    if is_same_stream {
                                        *active_stream_cancel_clone.borrow_mut() = None;
                                    }
                                    update_send_button_clone();
                                    return glib::ControlFlow::Break;
                                }
                            }
                        }
                    }
                ),
            );
        }
    ));

    send_button.connect_clicked({
        let submit_message = Rc::clone(&submit_message);
        let cancel_active_stream = Rc::clone(&cancel_active_stream);
        let active_stream_cancel = Rc::clone(&active_stream_cancel);
        move |_| {
            if active_stream_cancel.borrow().is_some() {
                cancel_active_stream();
            } else {
                submit_message();
            }
        }
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

    let on_settings_changed: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        title,
        #[strong]
        settings,
        #[strong]
        active_chat_id,
        #[strong]
        refresh_models,
        #[strong]
        sync_tools_toggle_for_current_context,
        move || {
            title.set_subtitle(&providers_subtitle(&settings.borrow()));
            if active_chat_id.borrow().is_none() {
                sync_tools_toggle_for_current_context();
            }
            refresh_models();
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
            let model_options = Rc::clone(&model_options);
            let on_settings_changed = Rc::clone(&on_settings_changed);
            move |win: &adw::ApplicationWindow, _, _| {
                present_settings_dialog(
                    win,
                    Rc::clone(&settings),
                    Rc::clone(&model_options),
                    Rc::clone(&on_settings_changed),
                );
            }
        })
        .build();

    window.add_action_entries([new_chat_action, toggle_sidebar_action, preferences_action]);

    refresh_models();
    input_view.grab_focus();

    window
}

fn present_shell_approval_dialog(
    parent: &adw::ApplicationWindow,
    request: &ShellToolRequest,
    reply_tx: mpsc::Sender<bool>,
) {
    let heading = if request.elevated {
        "Allow Administrator (sudo) Command?"
    } else if request.dangerous {
        "Allow Potentially Dangerous Command?"
    } else {
        "Allow Shell Command?"
    };

    let mut body = if request.elevated {
        "The assistant wants to run this command with administrator privileges. You will also be prompted by the system to authenticate:".to_string()
    } else if request.dangerous {
        "The assistant wants to run a command that may modify files or system state:".to_string()
    } else {
        "The assistant wants to run the following shell command:".to_string()
    };
    if let Some(dir) = &request.working_directory {
        body.push_str(&format!("\n\nDirectory: {dir}"));
    }

    let cmd_label = gtk::Label::builder()
        .label(&request.command)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .build();
    cmd_label.add_css_class("monospace");
    cmd_label.add_css_class("markdown-code-body");

    let cmd_card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    cmd_card.add_css_class("markdown-code-block");
    cmd_card.append(&cmd_label);

    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(&body)
        .extra_child(&cmd_card)
        .default_response("deny")
        .close_response("deny")
        .build();

    let allow_label = if request.elevated {
        "Allow with sudo"
    } else {
        "Allow"
    };
    dialog.add_responses(&[("deny", "Deny"), ("allow", allow_label)]);
    dialog.set_response_appearance(
        "allow",
        if request.dangerous || request.elevated {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        },
    );

    let sender = RefCell::new(Some(reply_tx));
    dialog.connect_response(None, move |_, response| {
        if let Some(tx) = sender.borrow_mut().take() {
            let _ = tx.send(response == "allow");
        }
    });

    dialog.present(Some(parent));
}

fn build_fallback_options(accounts: &[ProviderAccount]) -> Vec<ModelOption> {
    let mut options = Vec::new();
    for account in accounts {
        for &model in llm::fallback_models(account.kind) {
            options.push(ModelOption {
                account_id: account.id.clone(),
                account_name: account.name.clone(),
                kind: account.kind,
                model: model.to_string(),
            });
        }
    }
    options
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

fn update_tools_toggle_tooltip(button: &gtk::ToggleButton, enabled: bool) {
    let tip = if enabled {
        "Shell tools enabled for this chat (click to disable)"
    } else {
        "Shell tools disabled for this chat (click to enable)"
    };
    button.set_tooltip_text(Some(tip));
}

fn build_composer(initial_tools_enabled: bool) -> ComposerWidgets {
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

    let buffer = input_view.buffer();
    buffer.connect_changed(glib::clone!(
        #[weak]
        placeholder,
        move |buf| {
            let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
            placeholder.set_visible(text.is_empty());
        }
    ));

    let empty_model = gtk::StringList::new(&["No models — configure Settings"]);
    let search_expression = gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        gtk::Expression::NONE,
        "string",
    );
    let model_dropdown = gtk::DropDown::builder()
        .model(&empty_model)
        .expression(&search_expression)
        .enable_search(true)
        .sensitive(false)
        .valign(gtk::Align::Center)
        .build();
    model_dropdown.add_css_class("flat");
    model_dropdown.add_css_class("model-dropdown");

    let refresh_models_button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh models from configured providers")
        .valign(gtk::Align::Center)
        .sensitive(false)
        .build();
    refresh_models_button.add_css_class("flat");
    refresh_models_button.add_css_class("circular");

    let tools_toggle_button = gtk::ToggleButton::builder()
        .icon_name("utilities-terminal-symbolic")
        .active(initial_tools_enabled)
        .valign(gtk::Align::Center)
        .build();
    tools_toggle_button.add_css_class("flat");
    tools_toggle_button.add_css_class("circular");
    tools_toggle_button.add_css_class("tools-toggle");
    update_tools_toggle_tooltip(&tools_toggle_button, initial_tools_enabled);

    let attach_button = gtk::Button::builder()
        .icon_name("mail-attachment-symbolic")
        .tooltip_text("Attach files (or drag and drop)")
        .valign(gtk::Align::Center)
        .build();
    attach_button.add_css_class("flat");
    attach_button.add_css_class("circular");

    let spacer = gtk::Box::builder().hexpand(true).build();

    let send_button = gtk::Button::builder()
        .icon_name("go-up-symbolic")
        .tooltip_text("Send message (Enter)")
        .valign(gtk::Align::Center)
        .sensitive(false)
        .build();
    send_button.add_css_class("suggested-action");
    send_button.add_css_class("circular");

    let composer_toolbar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .hexpand(true)
        .build();
    composer_toolbar.add_css_class("composer-toolbar");
    composer_toolbar.append(&model_dropdown);
    composer_toolbar.append(&refresh_models_button);
    composer_toolbar.append(&tools_toggle_button);
    composer_toolbar.append(&attach_button);
    composer_toolbar.append(&spacer);
    composer_toolbar.append(&send_button);

    let attachments_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Start)
        .visible(false)
        .build();
    attachments_box.add_css_class("attachments-strip");

    let attachments_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&attachments_box)
        .build();
    attachments_box
        .bind_property("visible", &attachments_scroll, "visible")
        .sync_create()
        .build();

    let composer_card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .build();
    composer_card.add_css_class("card");
    composer_card.add_css_class("composer-card");
    composer_card.append(&attachments_scroll);
    composer_card.append(&input_overlay);
    composer_card.append(&composer_toolbar);

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

    ComposerWidgets {
        bar: composer_bar,
        card: composer_card,
        attachments_box,
        input_view,
        model_dropdown,
        refresh_models_button,
        tools_toggle_button,
        attach_button,
        send_button,
    }
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
