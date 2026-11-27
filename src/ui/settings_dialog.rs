use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::settings::{AppSettings, ProviderKind};

pub fn present_settings_dialog(
    parent: &adw::ApplicationWindow,
    settings: Rc<RefCell<AppSettings>>,
    on_changed: Rc<dyn Fn()>,
) {
    let dialog = adw::PreferencesDialog::builder()
        .title("Settings")
        .search_enabled(true)
        .build();

    let providers_page = adw::PreferencesPage::builder()
        .title("Providers")
        .icon_name("network-server-symbolic")
        .build();

    for kind in ProviderKind::ALL {
        let current = settings.borrow().provider(kind).clone();

        let group = adw::PreferencesGroup::builder()
            .title(kind.display_name())
            .description(kind.description())
            .build();

        let endpoint_row = adw::EntryRow::builder()
            .title("Endpoint URL")
            .text(&current.endpoint)
            .build();

        let api_key_title = if kind.api_key_optional() {
            "API Key (Optional)"
        } else {
            "API Key"
        };
        let api_key_row = adw::PasswordEntryRow::builder()
            .title(api_key_title)
            .text(&current.api_key)
            .build();

        let reset_button = gtk::Button::builder()
            .icon_name("edit-undo-symbolic")
            .tooltip_text(format!("Reset endpoint to {}", kind.default_endpoint()))
            .valign(gtk::Align::Center)
            .build();
        reset_button.add_css_class("flat");
        reset_button.connect_clicked(glib::clone!(
            #[weak]
            endpoint_row,
            move |_| {
                endpoint_row.set_text(kind.default_endpoint());
            }
        ));
        group.set_header_suffix(Some(&reset_button));

        endpoint_row.connect_changed({
            let settings = Rc::clone(&settings);
            let on_changed = Rc::clone(&on_changed);
            move |row| {
                let mut guard = settings.borrow_mut();
                guard.set_endpoint(kind, row.text().to_string());
                let _ = guard.save();
                drop(guard);
                on_changed();
            }
        });

        api_key_row.connect_changed({
            let settings = Rc::clone(&settings);
            let on_changed = Rc::clone(&on_changed);
            move |row| {
                let mut guard = settings.borrow_mut();
                guard.set_api_key(kind, row.text().to_string());
                let _ = guard.save();
                drop(guard);
                on_changed();
            }
        });

        group.add(&endpoint_row);
        group.add(&api_key_row);
        providers_page.add(&group);
    }

    dialog.add(&providers_page);
    dialog.present(Some(parent));
}
