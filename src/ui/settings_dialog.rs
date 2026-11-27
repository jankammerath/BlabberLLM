use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

use crate::llm::{self, GroundingToolInfo, ModelOption};
use crate::settings::{
    AppSettings, ProviderAccount, ProviderKind, ShellPermissionMode, MAX_MAX_TOOL_STEPS,
    MIN_MAX_TOOL_STEPS,
};

pub fn present_settings_dialog(
    parent: &adw::ApplicationWindow,
    settings: Rc<RefCell<AppSettings>>,
    model_options: Rc<RefCell<Vec<ModelOption>>>,
    on_changed: Rc<dyn Fn()>,
) {
    let parent_height = parent.height().max(parent.default_height()).max(500);
    let target_height = ((parent_height as f64) * 0.8).round() as i32;

    let nav_view = adw::NavigationView::new();

    let window = adw::Window::builder()
        .title("Settings")
        .transient_for(parent)
        .modal(true)
        .resizable(true)
        .default_width(620)
        .default_height(target_height)
        .width_request(360)
        .height_request(400)
        .content(&nav_view)
        .build();

    let key_controller = gtk::EventControllerKey::new();
    key_controller.connect_key_pressed(glib::clone!(
        #[weak]
        window,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                window.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    ));
    window.add_controller(key_controller);

    let providers_page = adw::PreferencesPage::builder()
        .title("Settings")
        .icon_name("network-server-symbolic")
        .build();

    let list_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list_box.add_css_class("boxed-list");

    let providers_group = adw::PreferencesGroup::builder()
        .title("Configured Providers")
        .description("Add and manage provider endpoints and API keys. You can configure multiple accounts for the same provider.")
        .build();
    providers_group.add(&list_box);

    let add_button = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .label("Add Provider")
        .valign(gtk::Align::Center)
        .build();
    add_button.add_css_class("flat");
    providers_group.set_header_suffix(Some(&add_button));

    providers_page.add(&providers_group);

    let defaults_group = adw::PreferencesGroup::builder()
        .title("Default Model")
        .description("Provider and model selected automatically when starting a new chat.")
        .build();

    let default_provider_row = adw::ComboRow::builder()
        .title("Default Provider")
        .sensitive(false)
        .build();

    let default_model_search_expr = gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        gtk::Expression::NONE,
        "string",
    );
    let default_model_row = adw::ComboRow::builder()
        .title("Default Model")
        .expression(&default_model_search_expr)
        .enable_search(true)
        .sensitive(false)
        .build();

    defaults_group.add(&default_provider_row);
    defaults_group.add(&default_model_row);
    providers_page.add(&defaults_group);

    let default_accounts_state: Rc<RefCell<Vec<ProviderAccount>>> = Rc::new(RefCell::new(Vec::new()));
    let default_models_state: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let suppress_defaults = Rc::new(Cell::new(false));

    let populate_default_models: Rc<dyn Fn(&ProviderAccount, Option<&str>)> = Rc::new(glib::clone!(
        #[weak]
        default_model_row,
        #[strong]
        model_options,
        #[strong]
        default_models_state,
        #[strong]
        suppress_defaults,
        move |account: &ProviderAccount, preferred_model: Option<&str>| {
            let prev_suppress = suppress_defaults.get();
            suppress_defaults.set(true);

            let mut models: Vec<String> = model_options
                .borrow()
                .iter()
                .filter(|opt| opt.account_id == account.id)
                .map(|opt| opt.model.clone())
                .collect();

            if models.is_empty() {
                models = llm::fallback_models(account.kind)
                    .iter()
                    .map(|m| (*m).to_string())
                    .collect();
            }

            if let Some(pref) = preferred_model {
                if !pref.trim().is_empty() && !models.iter().any(|m| m == pref) {
                    models.insert(0, pref.to_string());
                }
            }

            if models.is_empty() {
                let placeholder = gtk::StringList::new(&["No models available"]);
                default_model_row.set_model(Some(&placeholder));
                default_model_row.set_selected(0);
                default_model_row.set_sensitive(false);
                *default_models_state.borrow_mut() = Vec::new();
            } else {
                let refs: Vec<&str> = models.iter().map(String::as_str).collect();
                let list = gtk::StringList::new(&refs);
                default_model_row.set_model(Some(&list));
                default_model_row.set_sensitive(true);
                let idx = preferred_model
                    .and_then(|pref| models.iter().position(|m| m == pref))
                    .unwrap_or(0);
                default_model_row.set_selected(idx as u32);
                *default_models_state.borrow_mut() = models;
            }

            suppress_defaults.set(prev_suppress);
        }
    ));

    let refresh_defaults: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        default_provider_row,
        #[weak]
        default_model_row,
        #[strong]
        settings,
        #[strong]
        default_accounts_state,
        #[strong]
        default_models_state,
        #[strong]
        suppress_defaults,
        #[strong]
        populate_default_models,
        move || {
            suppress_defaults.set(true);

            let configured: Vec<ProviderAccount> = settings
                .borrow()
                .accounts()
                .iter()
                .filter(|a| a.is_configured())
                .cloned()
                .collect();

            if configured.is_empty() {
                let no_providers = gtk::StringList::new(&["No configured providers"]);
                default_provider_row.set_model(Some(&no_providers));
                default_provider_row.set_selected(0);
                default_provider_row.set_sensitive(false);

                let no_models = gtk::StringList::new(&["No models available"]);
                default_model_row.set_model(Some(&no_models));
                default_model_row.set_selected(0);
                default_model_row.set_sensitive(false);

                *default_accounts_state.borrow_mut() = Vec::new();
                *default_models_state.borrow_mut() = Vec::new();
                suppress_defaults.set(false);
                return;
            }

            let labels: Vec<String> = configured
                .iter()
                .map(|a| format!("{} ({})", a.name, a.kind.display_name()))
                .collect();
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let list = gtk::StringList::new(&refs);
            default_provider_row.set_model(Some(&list));
            default_provider_row.set_sensitive(true);

            let (saved_acc_id, saved_model) = settings
                .borrow()
                .default_model()
                .map(|(a, m)| (Some(a.to_string()), Some(m.to_string())))
                .unwrap_or((None, None));

            let acc_idx = saved_acc_id
                .as_deref()
                .and_then(|id| configured.iter().position(|a| a.id == id))
                .unwrap_or(0);
            default_provider_row.set_selected(acc_idx as u32);

            let selected_account = configured[acc_idx].clone();
            let preferred_model = if saved_acc_id.as_deref() == Some(selected_account.id.as_str()) {
                saved_model.as_deref()
            } else {
                None
            };
            populate_default_models(&selected_account, preferred_model);

            *default_accounts_state.borrow_mut() = configured;
            suppress_defaults.set(false);
        }
    ));

    default_provider_row.connect_selected_notify(glib::clone!(
        #[strong]
        settings,
        #[strong]
        default_accounts_state,
        #[strong]
        default_models_state,
        #[strong]
        suppress_defaults,
        #[strong]
        populate_default_models,
        #[strong]
        on_changed,
        move |row| {
            if suppress_defaults.get() {
                return;
            }
            let idx = row.selected() as usize;
            let Some(account) = default_accounts_state.borrow().get(idx).cloned() else {
                return;
            };
            populate_default_models(&account, None);
            let chosen_model = default_models_state.borrow().first().cloned();
            {
                let mut guard = settings.borrow_mut();
                guard.set_default_model(Some(account.id), chosen_model);
                let _ = guard.save();
            }
            on_changed();
        }
    ));

    default_model_row.connect_selected_notify(glib::clone!(
        #[weak]
        default_provider_row,
        #[strong]
        settings,
        #[strong]
        default_accounts_state,
        #[strong]
        default_models_state,
        #[strong]
        suppress_defaults,
        #[strong]
        on_changed,
        move |row| {
            if suppress_defaults.get() {
                return;
            }
            let acc_idx = default_provider_row.selected() as usize;
            let model_idx = row.selected() as usize;
            let Some(account) = default_accounts_state.borrow().get(acc_idx).cloned() else {
                return;
            };
            let Some(model) = default_models_state.borrow().get(model_idx).cloned() else {
                return;
            };
            {
                let mut guard = settings.borrow_mut();
                guard.set_default_model(Some(account.id), Some(model));
                let _ = guard.save();
            }
            on_changed();
        }
    ));

    let tools_group = adw::PreferencesGroup::builder()
        .title("Tools")
        .description("Allow the assistant to run local shell commands when answering prompts.")
        .build();

    let shell_enabled = settings.borrow().shell_tool_enabled();
    let shell_switch = adw::SwitchRow::builder()
        .title("Shell Tool")
        .subtitle("Enable bash command execution for the assistant")
        .active(shell_enabled)
        .build();

    let permission_labels: Vec<&str> = ShellPermissionMode::ALL
        .iter()
        .map(|m| m.display_name())
        .collect();
    let permission_model = gtk::StringList::new(&permission_labels);
    let permission_row = adw::ComboRow::builder()
        .title("Tool Use Permission")
        .subtitle("Choose when to ask before running a shell command")
        .model(&permission_model)
        .selected(settings.borrow().shell_permission_mode().index() as u32)
        .sensitive(shell_enabled)
        .build();

    let sudo_switch = adw::SwitchRow::builder()
        .title("Allow Elevated Commands (sudo)")
        .subtitle("Let the assistant request administrator privileges via system authentication")
        .active(settings.borrow().shell_sudo_enabled())
        .sensitive(shell_enabled)
        .build();

    let max_steps_adj = gtk::Adjustment::new(
        settings.borrow().max_tool_steps() as f64,
        MIN_MAX_TOOL_STEPS as f64,
        MAX_MAX_TOOL_STEPS as f64,
        1.0,
        5.0,
        0.0,
    );
    let max_steps_row = adw::SpinRow::builder()
        .title("Maximum Agent Steps")
        .subtitle("Maximum number of tool-use turns per prompt")
        .adjustment(&max_steps_adj)
        .numeric(true)
        .digits(0)
        .sensitive(shell_enabled)
        .build();

    shell_switch
        .bind_property("active", &permission_row, "sensitive")
        .sync_create()
        .build();

    shell_switch
        .bind_property("active", &sudo_switch, "sensitive")
        .sync_create()
        .build();

    shell_switch
        .bind_property("active", &max_steps_row, "sensitive")
        .sync_create()
        .build();

    shell_switch.connect_active_notify({
        let settings = Rc::clone(&settings);
        let on_changed = Rc::clone(&on_changed);
        move |row| {
            {
                let mut guard = settings.borrow_mut();
                guard.set_shell_tool_enabled(row.is_active());
                let _ = guard.save();
            }
            on_changed();
        }
    });

    permission_row.connect_selected_notify({
        let settings = Rc::clone(&settings);
        move |row| {
            let mode = ShellPermissionMode::from_index(row.selected() as usize);
            let mut guard = settings.borrow_mut();
            guard.set_shell_permission_mode(mode);
            let _ = guard.save();
        }
    });

    sudo_switch.connect_active_notify({
        let settings = Rc::clone(&settings);
        move |row| {
            let mut guard = settings.borrow_mut();
            guard.set_shell_sudo_enabled(row.is_active());
            let _ = guard.save();
        }
    });

    max_steps_row.connect_value_notify({
        let settings = Rc::clone(&settings);
        move |row| {
            let steps = row.value().round().max(1.0) as usize;
            let mut guard = settings.borrow_mut();
            guard.set_max_tool_steps(steps);
            let _ = guard.save();
        }
    });

    tools_group.add(&shell_switch);
    tools_group.add(&permission_row);
    tools_group.add(&sudo_switch);
    tools_group.add(&max_steps_row);
    providers_page.add(&tools_group);

    let about_group = adw::PreferencesGroup::builder()
        .title("About")
        .build();

    let version_label = gtk::Label::builder()
        .label(format!("v{}", env!("CARGO_PKG_VERSION")))
        .valign(gtk::Align::Center)
        .selectable(true)
        .build();
    version_label.add_css_class("dim-label");
    version_label.add_css_class("monospace");

    let about_row = adw::ActionRow::builder()
        .title("BlabberLLM")
        .use_markup(false)
        .activatable(false)
        .build();
    let icon_bytes = glib::Bytes::from_static(include_bytes!(
        "../../packaging/icons/hicolor/128x128/apps/org.blabberllm.BlabberLLM.png"
    ));
    if let Ok(texture) = gtk::gdk::Texture::from_bytes(&icon_bytes) {
        let icon_img = gtk::Image::builder()
            .paintable(&texture)
            .pixel_size(32)
            .valign(gtk::Align::Center)
            .build();
        about_row.add_prefix(&icon_img);
    }
    about_row.add_suffix(&version_label);

    about_group.add(&about_row);
    providers_page.add(&about_group);

    let main_header = adw::HeaderBar::new();
    let main_toolbar = adw::ToolbarView::new();
    main_toolbar.add_top_bar(&main_header);
    main_toolbar.set_content(Some(&providers_page));

    let root_page = adw::NavigationPage::builder()
        .title("Settings")
        .child(&main_toolbar)
        .build();
    nav_view.add(&root_page);

    let refresh_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let refresh_list: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        nav_view,
        #[weak]
        list_box,
        #[strong]
        settings,
        #[strong]
        on_changed,
        #[strong]
        refresh_slot,
        #[strong]
        refresh_defaults,
        move || {
            refresh_defaults();

            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }

            let accounts = settings.borrow().accounts().to_vec();
            if accounts.is_empty() {
                let empty_row = adw::ActionRow::builder()
                    .title("No providers configured")
                    .subtitle("Click Add Provider to set up Claude, Gemini, Grok, OpenAI, OpenRouter, or Ollama.")
                    .activatable(false)
                    .build();
                empty_row.set_use_markup(false);
                list_box.append(&empty_row);
                return;
            }

            for account in accounts {
                let status = if account.is_configured() {
                    account.endpoint.clone()
                } else {
                    format!("{} · API key required", account.endpoint)
                };
                let subtitle = format!("{} · {}", account.kind.display_name(), status);

                let row = adw::ActionRow::builder()
                    .title(&account.name)
                    .subtitle(&subtitle)
                    .activatable(true)
                    .build();
                row.set_use_markup(false);

                let chevron = gtk::Image::from_icon_name("go-next-symbolic");
                chevron.add_css_class("dim-label");
                row.add_suffix(&chevron);

                let account_id = account.id.clone();
                row.connect_activated(glib::clone!(
                    #[weak]
                    nav_view,
                    #[strong]
                    settings,
                    #[strong]
                    on_changed,
                    #[strong]
                    refresh_slot,
                    move |_| {
                        if let Some(refresh) = refresh_slot.borrow().as_ref() {
                            push_edit_provider_page(
                                &nav_view,
                                Rc::clone(&settings),
                                account_id.clone(),
                                Rc::clone(refresh),
                                Rc::clone(&on_changed),
                            );
                        }
                    }
                ));

                list_box.append(&row);
            }
        }
    ));

    *refresh_slot.borrow_mut() = Some(Rc::clone(&refresh_list));
    refresh_list();

    add_button.connect_clicked(glib::clone!(
        #[weak]
        nav_view,
        #[strong]
        settings,
        #[strong]
        refresh_list,
        #[strong]
        on_changed,
        move |_| {
            push_add_provider_page(
                &nav_view,
                Rc::clone(&settings),
                Rc::clone(&refresh_list),
                Rc::clone(&on_changed),
            );
        }
    ));

    window.present();
}

fn push_add_provider_page(
    nav_view: &adw::NavigationView,
    settings: Rc<RefCell<AppSettings>>,
    refresh_list: Rc<dyn Fn()>,
    on_changed: Rc<dyn Fn()>,
) {
    let initial_kind = ProviderKind::Claude;
    let initial_name = settings.borrow().suggested_name(initial_kind);

    let (kind_row, name_row, endpoint_row, api_key_row, form_group) = build_provider_form(
        initial_kind,
        &initial_name,
        initial_kind.default_endpoint(),
        "",
    );

    let last_auto_name = Rc::new(RefCell::new(initial_name));
    let last_auto_endpoint = Rc::new(RefCell::new(initial_kind.default_endpoint().to_string()));

    kind_row.connect_selected_notify(glib::clone!(
        #[weak]
        name_row,
        #[weak]
        endpoint_row,
        #[weak]
        api_key_row,
        #[strong]
        settings,
        #[strong]
        last_auto_name,
        #[strong]
        last_auto_endpoint,
        move |row| {
            let kind = ProviderKind::from_index(row.selected() as usize);

            let current_name = name_row.text().to_string();
            if current_name.trim().is_empty() || current_name == *last_auto_name.borrow() {
                let suggested = settings.borrow().suggested_name(kind);
                name_row.set_text(&suggested);
                *last_auto_name.borrow_mut() = suggested;
            }

            let current_endpoint = endpoint_row.text().to_string();
            if current_endpoint.trim().is_empty()
                || current_endpoint == *last_auto_endpoint.borrow()
            {
                let default_ep = kind.default_endpoint().to_string();
                endpoint_row.set_text(&default_ep);
                *last_auto_endpoint.borrow_mut() = default_ep;
            }

            api_key_row.set_title(api_key_label(kind));
        }
    ));

    let enabled_grounding_tools: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let grounding_group = build_grounding_tools_section(
        &kind_row,
        &endpoint_row,
        &api_key_row,
        Rc::clone(&enabled_grounding_tools),
        Rc::new(|| {}),
    );

    let add_confirm_button = gtk::Button::builder()
        .label("Add Provider")
        .halign(gtk::Align::Center)
        .margin_top(12)
        .build();
    add_confirm_button.add_css_class("suggested-action");
    add_confirm_button.add_css_class("pill");

    add_confirm_button.connect_clicked(glib::clone!(
        #[weak]
        nav_view,
        #[weak]
        kind_row,
        #[weak]
        name_row,
        #[weak]
        endpoint_row,
        #[weak]
        api_key_row,
        #[strong]
        settings,
        #[strong]
        enabled_grounding_tools,
        #[strong]
        refresh_list,
        #[strong]
        on_changed,
        move |_| {
            let kind = ProviderKind::from_index(kind_row.selected() as usize);
            let raw_name = name_row.text().trim().to_string();
            let name = if raw_name.is_empty() {
                settings.borrow().suggested_name(kind)
            } else {
                raw_name
            };
            let raw_endpoint = endpoint_row.text().trim().to_string();
            let endpoint = if raw_endpoint.is_empty() {
                kind.default_endpoint().to_string()
            } else {
                raw_endpoint
            };
            let api_key = api_key_row.text().to_string();

            let mut account = ProviderAccount::new(kind, name, endpoint, api_key);
            account.enabled_grounding_tools = enabled_grounding_tools.borrow().clone();
            {
                let mut guard = settings.borrow_mut();
                guard.add_account(account);
                let _ = guard.save();
            }

            refresh_list();
            on_changed();
            nav_view.pop();
        }
    ));

    let action_group = adw::PreferencesGroup::new();
    action_group.add(&add_confirm_button);

    let page_content = adw::PreferencesPage::new();
    page_content.add(&form_group);
    page_content.add(&grounding_group);
    page_content.add(&action_group);

    let header_bar = adw::HeaderBar::new();
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&page_content));

    let nav_page = adw::NavigationPage::builder()
        .title("Add Provider")
        .child(&toolbar_view)
        .build();

    nav_view.push(&nav_page);
}

fn push_edit_provider_page(
    nav_view: &adw::NavigationView,
    settings: Rc<RefCell<AppSettings>>,
    account_id: String,
    refresh_list: Rc<dyn Fn()>,
    on_changed: Rc<dyn Fn()>,
) {
    let Some(account) = settings.borrow().get(&account_id).cloned() else {
        return;
    };

    let (kind_row, name_row, endpoint_row, api_key_row, form_group) = build_provider_form(
        account.kind,
        &account.name,
        &account.endpoint,
        &account.api_key,
    );

    let enabled_grounding_tools: Rc<RefCell<Vec<String>>> =
        Rc::new(RefCell::new(account.enabled_grounding_tools.clone()));

    let save_edits: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        kind_row,
        #[weak]
        name_row,
        #[weak]
        endpoint_row,
        #[weak]
        api_key_row,
        #[strong]
        settings,
        #[strong]
        account_id,
        #[strong]
        enabled_grounding_tools,
        #[strong]
        refresh_list,
        #[strong]
        on_changed,
        move || {
            let kind = ProviderKind::from_index(kind_row.selected() as usize);
            let raw_name = name_row.text().trim().to_string();
            let name = if raw_name.is_empty() {
                kind.display_name().to_string()
            } else {
                raw_name
            };
            let updated = ProviderAccount {
                id: account_id.clone(),
                kind,
                name,
                endpoint: endpoint_row.text().trim().to_string(),
                api_key: api_key_row.text().to_string(),
                enabled_grounding_tools: enabled_grounding_tools.borrow().clone(),
            };
            {
                let mut guard = settings.borrow_mut();
                guard.update_account(updated);
                let _ = guard.save();
            }
            refresh_list();
            on_changed();
        }
    ));

    let grounding_group = build_grounding_tools_section(
        &kind_row,
        &endpoint_row,
        &api_key_row,
        Rc::clone(&enabled_grounding_tools),
        Rc::clone(&save_edits),
    );

    kind_row.connect_selected_notify(glib::clone!(
        #[weak]
        api_key_row,
        #[strong]
        save_edits,
        move |row| {
            let kind = ProviderKind::from_index(row.selected() as usize);
            api_key_row.set_title(api_key_label(kind));
            save_edits();
        }
    ));

    name_row.connect_changed({
        let save_edits = Rc::clone(&save_edits);
        move |_| save_edits()
    });
    endpoint_row.connect_changed({
        let save_edits = Rc::clone(&save_edits);
        move |_| save_edits()
    });
    api_key_row.connect_changed({
        let save_edits = Rc::clone(&save_edits);
        move |_| save_edits()
    });

    let remove_button = gtk::Button::builder()
        .label("Remove Provider")
        .halign(gtk::Align::Center)
        .margin_top(12)
        .build();
    remove_button.add_css_class("destructive-action");
    remove_button.add_css_class("pill");

    remove_button.connect_clicked(glib::clone!(
        #[weak]
        nav_view,
        #[strong]
        settings,
        #[strong]
        account_id,
        #[strong]
        refresh_list,
        #[strong]
        on_changed,
        move |_| {
            {
                let mut guard = settings.borrow_mut();
                guard.remove_account(&account_id);
                let _ = guard.save();
            }
            refresh_list();
            on_changed();
            nav_view.pop();
        }
    ));

    let danger_group = adw::PreferencesGroup::new();
    danger_group.add(&remove_button);

    let page_content = adw::PreferencesPage::new();
    page_content.add(&form_group);
    page_content.add(&grounding_group);
    page_content.add(&danger_group);

    let header_bar = adw::HeaderBar::new();
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&page_content));

    let nav_page = adw::NavigationPage::builder()
        .title(&account.name)
        .child(&toolbar_view)
        .build();

    nav_view.push(&nav_page);
}

fn build_provider_form(
    kind: ProviderKind,
    name: &str,
    endpoint: &str,
    api_key: &str,
) -> (
    adw::ComboRow,
    adw::EntryRow,
    adw::EntryRow,
    adw::PasswordEntryRow,
    adw::PreferencesGroup,
) {
    let provider_names: Vec<&str> = ProviderKind::ALL
        .iter()
        .map(|k| k.display_name())
        .collect();
    let provider_model = gtk::StringList::new(&provider_names);

    let kind_row = adw::ComboRow::builder()
        .title("Provider")
        .model(&provider_model)
        .selected(kind.index() as u32)
        .build();

    let name_row = adw::EntryRow::builder()
        .title("Account Name")
        .text(name)
        .build();

    let endpoint_row = adw::EntryRow::builder()
        .title("Endpoint URL")
        .text(endpoint)
        .build();

    let reset_endpoint_button = gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text("Reset endpoint to provider default")
        .valign(gtk::Align::Center)
        .build();
    reset_endpoint_button.add_css_class("flat");
    reset_endpoint_button.connect_clicked(glib::clone!(
        #[weak]
        kind_row,
        #[weak]
        endpoint_row,
        move |_| {
            let selected_kind = ProviderKind::from_index(kind_row.selected() as usize);
            endpoint_row.set_text(selected_kind.default_endpoint());
        }
    ));
    endpoint_row.add_suffix(&reset_endpoint_button);

    let api_key_row = adw::PasswordEntryRow::builder()
        .title(api_key_label(kind))
        .text(api_key)
        .build();

    let group = adw::PreferencesGroup::builder()
        .title("Provider Details")
        .build();
    group.add(&kind_row);
    group.add(&name_row);
    group.add(&endpoint_row);
    group.add(&api_key_row);

    (kind_row, name_row, endpoint_row, api_key_row, group)
}

fn build_grounding_tools_section(
    kind_row: &adw::ComboRow,
    endpoint_row: &adw::EntryRow,
    api_key_row: &adw::PasswordEntryRow,
    enabled_tools: Rc<RefCell<Vec<String>>>,
    on_tools_changed: Rc<dyn Fn()>,
) -> adw::PreferencesGroup {
    let list_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list_box.add_css_class("boxed-list");

    let refresh_button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Load available grounding tools")
        .valign(gtk::Align::Center)
        .build();
    refresh_button.add_css_class("flat");

    let group = adw::PreferencesGroup::builder()
        .title("Grounding Tools")
        .description("Provider-native web search and URL grounding capabilities.")
        .build();
    group.set_header_suffix(Some(&refresh_button));
    group.add(&list_box);

    let load_gen = Rc::new(Cell::new(0u64));
    let debounce_gen = Rc::new(Cell::new(0u64));

    let load_tools: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        kind_row,
        #[weak]
        endpoint_row,
        #[weak]
        api_key_row,
        #[weak]
        list_box,
        #[weak]
        refresh_button,
        #[strong]
        enabled_tools,
        #[strong]
        on_tools_changed,
        #[strong]
        load_gen,
        move || {
            let gen = load_gen.get().wrapping_add(1);
            load_gen.set(gen);

            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }

            let kind = ProviderKind::from_index(kind_row.selected() as usize);
            let endpoint = endpoint_row.text().trim().to_string();
            let api_key = api_key_row.text().to_string();

            if !kind.api_key_optional() && api_key.trim().is_empty() {
                let prompt_row = adw::ActionRow::builder()
                    .title("API key required")
                    .subtitle("Insert an API key above to load available grounding tools.")
                    .activatable(false)
                    .build();
                prompt_row.set_use_markup(false);
                list_box.append(&prompt_row);
                refresh_button.set_sensitive(false);
                return;
            }

            refresh_button.set_sensitive(false);
            let spinner = gtk::Spinner::builder()
                .spinning(true)
                .valign(gtk::Align::Center)
                .build();
            let loading_row = adw::ActionRow::builder()
                .title("Loading grounding tools…")
                .subtitle(format!("Checking {} capabilities", kind.display_name()))
                .activatable(false)
                .build();
            loading_row.set_use_markup(false);
            loading_row.add_suffix(&spinner);
            list_box.append(&loading_row);

            let probe = ProviderAccount {
                id: "probe".to_string(),
                kind,
                name: kind.display_name().to_string(),
                endpoint: if endpoint.is_empty() {
                    kind.default_endpoint().to_string()
                } else {
                    endpoint
                },
                api_key,
                enabled_grounding_tools: Vec::new(),
            };

            let (tx, rx) = mpsc::channel::<Result<Vec<GroundingToolInfo>, String>>();
            thread::spawn(move || {
                let _ = tx.send(llm::fetch_grounding_tools(&probe));
            });

            let load_gen_clone = Rc::clone(&load_gen);
            let enabled_tools_clone = Rc::clone(&enabled_tools);
            let on_tools_changed_clone = Rc::clone(&on_tools_changed);
            glib::timeout_add_local(
                Duration::from_millis(50),
                glib::clone!(
                    #[weak]
                    list_box,
                    #[weak]
                    refresh_button,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || match rx.try_recv() {
                        Ok(result) => {
                            if load_gen_clone.get() != gen {
                                return glib::ControlFlow::Break;
                            }
                            refresh_button.set_sensitive(true);
                            while let Some(child) = list_box.first_child() {
                                list_box.remove(&child);
                            }

                            match result {
                                Ok(tools) if tools.is_empty() => {
                                    let empty_row = adw::ActionRow::builder()
                                        .title("No grounding tools available")
                                        .subtitle(format!(
                                            "{} does not expose built-in cloud grounding tools.",
                                            kind.display_name()
                                        ))
                                        .activatable(false)
                                        .build();
                                    empty_row.set_use_markup(false);
                                    list_box.append(&empty_row);
                                }
                                Ok(tools) => {
                                    for tool in tools {
                                        let is_active = enabled_tools_clone
                                            .borrow()
                                            .iter()
                                            .any(|id| id == &tool.id);
                                        let switch_row = adw::SwitchRow::builder()
                                            .title(&tool.title)
                                            .subtitle(&tool.description)
                                            .active(is_active)
                                            .build();
                                        switch_row.set_use_markup(false);

                                        let tool_id = tool.id.clone();
                                        switch_row.connect_active_notify({
                                            let enabled_tools = Rc::clone(&enabled_tools_clone);
                                            let on_tools_changed =
                                                Rc::clone(&on_tools_changed_clone);
                                            move |row| {
                                                {
                                                    let mut guard = enabled_tools.borrow_mut();
                                                    if row.is_active() {
                                                        if !guard.iter().any(|id| id == &tool_id) {
                                                            guard.push(tool_id.clone());
                                                        }
                                                    } else {
                                                        guard.retain(|id| id != &tool_id);
                                                    }
                                                }
                                                on_tools_changed();
                                            }
                                        });

                                        list_box.append(&switch_row);
                                    }
                                }
                                Err(err) => {
                                    let err_row = adw::ActionRow::builder()
                                        .title("Could not load grounding tools")
                                        .subtitle(&err)
                                        .activatable(false)
                                        .build();
                                    err_row.set_use_markup(false);
                                    list_box.append(&err_row);
                                }
                            }
                            glib::ControlFlow::Break
                        }
                        Err(mpsc::TryRecvError::Empty) => {
                            if load_gen_clone.get() != gen {
                                glib::ControlFlow::Break
                            } else {
                                glib::ControlFlow::Continue
                            }
                        }
                        Err(mpsc::TryRecvError::Disconnected) => {
                            if load_gen_clone.get() == gen {
                                refresh_button.set_sensitive(true);
                            }
                            glib::ControlFlow::Break
                        }
                    }
                ),
            );
        }
    ));

    let schedule_load: Rc<dyn Fn()> = Rc::new({
        let debounce_gen = Rc::clone(&debounce_gen);
        let load_tools = Rc::clone(&load_tools);
        move || {
            let next = debounce_gen.get().wrapping_add(1);
            debounce_gen.set(next);
            let debounce_gen_clone = Rc::clone(&debounce_gen);
            let load_tools_clone = Rc::clone(&load_tools);
            glib::timeout_add_local_once(Duration::from_millis(650), move || {
                if debounce_gen_clone.get() == next {
                    load_tools_clone();
                }
            });
        }
    });

    refresh_button.connect_clicked({
        let load_tools = Rc::clone(&load_tools);
        move |_| load_tools()
    });

    api_key_row.connect_changed({
        let schedule_load = Rc::clone(&schedule_load);
        move |_| schedule_load()
    });
    endpoint_row.connect_changed({
        let schedule_load = Rc::clone(&schedule_load);
        move |_| schedule_load()
    });
    kind_row.connect_selected_notify({
        let load_tools = Rc::clone(&load_tools);
        move |_| load_tools()
    });

    load_tools();
    group
}

fn api_key_label(kind: ProviderKind) -> &'static str {
    if kind.api_key_optional() {
        "API Key (Optional)"
    } else {
        "API Key"
    }
}
