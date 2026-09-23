//! First-run welcome: pick the shortcut, then set up a provider.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::app::{App, show_message};
use crate::config::DEFAULT_SHORTCUT;
use crate::portal::validate_trigger;

const FEATURES: &str = "• Instantly optimize your writing with AI by copying your text and invoking Writing Tools with \"ctrl+space\", anywhere.

• Get a summary you can chat with of articles, YouTube videos, or documents by copying all their text (or the YouTube transcript from its description), invoking Writing Tools, and choosing Summary.

• Supports an extensive range of AI models:
    - Gemini
    - ChatGPT subscription access through OpenAI Codex
    - ANY OpenAI Compatible API — including local LLMs!";

pub fn show(app: &Rc<App>) {
    let window = adw::ApplicationWindow::builder()
        .application(&app.gtk)
        .title("Welcome to Writing Tools")
        .default_width(600)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_top(12);
    content.set_margin_bottom(24);
    content.set_margin_start(30);
    content.set_margin_end(30);

    let title = gtk::Label::new(Some("Welcome to Writing Tools!"));
    title.add_css_class("title-1");
    content.append(&title);
    let features = gtk::Label::new(Some(FEATURES));
    features.set_wrap(true);
    features.set_xalign(0.0);
    content.append(&features);

    let group = adw::PreferencesGroup::builder().title("Customize your shortcut key (default: \"ctrl+space\")").build();
    let shortcut = adw::EntryRow::builder().title("Shortcut Key").text(DEFAULT_SHORTCUT).build();
    group.add(&shortcut);
    content.append(&group);

    let next = gtk::Button::with_label("Next");
    next.add_css_class("suggested-action");
    next.add_css_class("pill");
    next.set_halign(gtk::Align::Center);
    content.append(&next);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&content));
    window.set_content(Some(&toolbar));

    let finished = Rc::new(Cell::new(false));
    next.connect_clicked({
        let app = app.clone();
        let window = window.downgrade();
        let finished = finished.clone();
        move |_| {
            let Some(window) = window.upgrade() else { return };
            let text = shortcut.text().trim().to_owned();
            if let Err(problem) = validate_trigger(&text) {
                show_message(Some(&window), "Invalid shortcut", &format!("Invalid shortcut: '{text}'.\n\n{problem}"));
                return;
            }
            log::debug!("User selected shortcut: {text}");
            app.config.borrow_mut().shortcut = Some(text);
            finished.set(true);
            super::settings::show(&app, true);
            window.close();
        }
    });
    window.connect_close_request({
        let app = app.clone();
        move |_| {
            if !finished.get() {
                app.exit();
            }
            glib::Propagation::Proceed
        }
    });
    window.present();
}
