use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::settings::{AppSettings, ProviderAccount, ProviderKind};

pub fn present_settings_dialog(
    parent: &adw::ApplicationWindow,
    settings: Rc<RefCell<AppSettings>>,
    on_changed: Rc<dyn Fn()>,
) {
    let dialog = adw::PreferencesDialog::builder()
        .title("Settings")
        .content_width(600)
        .content_height(520)
        .search_enabled(false)
        .build();

    let providers_page = adw::PreferencesPage::builder()
        .title("Providers")
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
    dialog.add(&providers_page);

    let refresh_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let refresh_list: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        dialog,
        #[weak]
        list_box,
        #[strong]
        settings,
        #[strong]
        on_changed,
        #[strong]
        refresh_slot,
        move || {
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
                    dialog,
                    #[strong]
                    settings,
                    #[strong]
                    on_changed,
                    #[strong]
                    refresh_slot,
                    move |_| {
                        if let Some(refresh) = refresh_slot.borrow().as_ref() {
                            push_edit_provider_page(
                                &dialog,
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
        dialog,
        #[strong]
        settings,
        #[strong]
        refresh_list,
        #[strong]
        on_changed,
        move |_| {
            push_add_provider_page(
                &dialog,
                Rc::clone(&settings),
                Rc::clone(&refresh_list),
                Rc::clone(&on_changed),
            );
        }
    ));

    dialog.present(Some(parent));
}

fn push_add_provider_page(
    dialog: &adw::PreferencesDialog,
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

    let add_confirm_button = gtk::Button::builder()
        .label("Add Provider")
        .halign(gtk::Align::Center)
        .margin_top(12)
        .build();
    add_confirm_button.add_css_class("suggested-action");
    add_confirm_button.add_css_class("pill");

    add_confirm_button.connect_clicked(glib::clone!(
        #[weak]
        dialog,
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

            let account = ProviderAccount::new(kind, name, endpoint, api_key);
            {
                let mut guard = settings.borrow_mut();
                guard.add_account(account);
                let _ = guard.save();
            }

            refresh_list();
            on_changed();
            dialog.pop_subpage();
        }
    ));

    let action_group = adw::PreferencesGroup::new();
    action_group.add(&add_confirm_button);

    let page_content = adw::PreferencesPage::new();
    page_content.add(&form_group);
    page_content.add(&action_group);

    let header_bar = adw::HeaderBar::new();
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&page_content));

    let nav_page = adw::NavigationPage::builder()
        .title("Add Provider")
        .child(&toolbar_view)
        .build();

    dialog.push_subpage(&nav_page);
}

fn push_edit_provider_page(
    dialog: &adw::PreferencesDialog,
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
        dialog,
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
            dialog.pop_subpage();
        }
    ));

    let danger_group = adw::PreferencesGroup::new();
    danger_group.add(&remove_button);

    let page_content = adw::PreferencesPage::new();
    page_content.add(&form_group);
    page_content.add(&danger_group);

    let header_bar = adw::HeaderBar::new();
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&page_content));

    let nav_page = adw::NavigationPage::builder()
        .title(&account.name)
        .child(&toolbar_view)
        .build();

    dialog.push_subpage(&nav_page);
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

fn api_key_label(kind: ProviderKind) -> &'static str {
    if kind.api_key_optional() {
        "API Key (Optional)"
    } else {
        "API Key"
    }
}
