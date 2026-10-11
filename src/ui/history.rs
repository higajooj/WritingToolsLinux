//! The history window: every past use, newest first.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::app::App;
use crate::history::{self, Entry};

const PREVIEW_CHARS: usize = 300;

thread_local! {
    static OPEN: RefCell<glib::WeakRef<adw::ApplicationWindow>> = RefCell::new(glib::WeakRef::new());
}

/// A row's collapsed preview and, once it has been expanded, its details.
struct Row {
    entry: Entry,
    content: gtk::Box,
    preview: gtk::Label,
    details: RefCell<Option<gtk::Box>>,
}

pub fn open(app: &Rc<App>) {
    // Rebuild rather than re-present, so the list is current.
    if let Some(window) = OPEN.with(|open| open.borrow().upgrade()) {
        window.close();
    }
    // Not titled "Writing Tools": that is what the popup's window rules match.
    let window = adw::ApplicationWindow::builder()
        .application(&app.gtk)
        .title("History")
        .default_width(1000)
        .default_height(700)
        .width_request(400)
        .height_request(300)
        .build();
    OPEN.with(|open| open.borrow().set(Some(&window)));

    let header = adw::HeaderBar::new();
    let clear = gtk::Button::with_label("Clear");
    clear.set_tooltip_text(Some("Clear history"));
    header.pack_start(&clear);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    window.set_content(Some(&toolbar));

    let rows: Vec<Row> = app.history.borrow().iter().rev().map(row).collect();
    if rows.is_empty() {
        clear.set_sensitive(false);
        toolbar.set_content(Some(&empty_page()));
    } else {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");
        list.set_valign(gtk::Align::Start);
        list.set_margin_top(12);
        list.set_margin_bottom(12);
        list.set_margin_start(12);
        list.set_margin_end(12);
        for row in &rows {
            list.append(&row.content);
        }
        list.connect_row_activated(move |_, activated| {
            if let Some(row) = usize::try_from(activated.index()).ok().and_then(|index| rows.get(index)) {
                row.toggle();
            }
        });
        let clamp = adw::Clamp::builder().child(&list).maximum_size(800).build();
        let scroller = gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).build();
        toolbar.set_content(Some(&scroller));
    }

    clear.connect_clicked({
        let app = app.clone();
        let window = window.downgrade();
        let toolbar = toolbar.downgrade();
        move |clear| {
            let Some(window) = window.upgrade() else { return };
            let dialog = adw::AlertDialog::new(Some("Clear history?"), Some("Every past use will be forgotten."));
            dialog.add_responses(&[("cancel", "Cancel"), ("clear", "Clear")]);
            dialog.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            dialog.connect_response(None, {
                let app = app.clone();
                let toolbar = toolbar.clone();
                let clear = clear.downgrade();
                move |_, response| {
                    if response != "clear" {
                        return;
                    }
                    app.clear_history();
                    if let Some(toolbar) = toolbar.upgrade() {
                        toolbar.set_content(Some(&empty_page()));
                    }
                    if let Some(clear) = clear.upgrade() {
                        clear.set_sensitive(false);
                    }
                }
            });
            dialog.present(Some(&window));
        }
    });

    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed({
        let window = window.downgrade();
        move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(window) = window.upgrade() {
                    window.close();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    window.add_controller(escape);

    window.present();
}

fn empty_page() -> adw::StatusPage {
    adw::StatusPage::builder().title("No history yet").description("Your past uses will appear here.").build()
}

fn row(entry: &Entry) -> Row {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    content.add_css_class("history-row");

    let option = gtk::Label::new(Some(&entry.option));
    option.add_css_class("heading");
    option.set_xalign(0.0);
    option.set_hexpand(true);
    option.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let time = gtk::Label::new(Some(&format_time(entry.time)));
    time.add_css_class("dim-label");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    top.append(&option);
    top.append(&time);
    content.append(&top);

    let prompt = match &entry.extra {
        Some(extra) => {
            let label = wrapped(extra);
            label.add_css_class("instruction-message");
            label
        }
        None => {
            let label = wrapped("No custom prompt");
            label.add_css_class("dim-label");
            label
        }
    };
    content.append(&prompt);

    let preview = wrapped(&history::snippet(&entry.text, PREVIEW_CHARS));
    preview.set_lines(3);
    preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&preview);

    Row { entry: entry.clone(), content, preview, details: RefCell::new(None) }
}

impl Row {
    /// Switch between the preview and the full text with its response.
    fn toggle(&self) {
        let mut details = self.details.borrow_mut();
        let details = details.get_or_insert_with(|| {
            let details = self.build_details();
            details.set_visible(false);
            self.content.append(&details);
            details
        });
        let expand = !details.is_visible();
        details.set_visible(expand);
        self.preview.set_visible(!expand);
    }

    fn build_details(&self) -> gtk::Box {
        let details = gtk::Box::new(gtk::Orientation::Vertical, 4);
        details.append(&selectable(&self.entry.text));
        let caption = wrapped("Response");
        caption.add_css_class("heading");
        caption.set_margin_top(8);
        details.append(&caption);
        match &self.entry.response {
            Some(response) => details.append(&selectable(response)),
            None => {
                let label = wrapped("No response");
                label.add_css_class("dim-label");
                details.append(&label);
            }
        }
        details
    }
}

fn wrapped(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label
}

fn selectable(text: &str) -> gtk::Label {
    let label = wrapped(text);
    label.set_selectable(true);
    label.set_can_focus(false);
    label
}

fn format_time(time: i64) -> String {
    glib::DateTime::from_unix_local(time)
        .and_then(|time| time.format("%Y-%m-%d %H:%M"))
        .map(String::from)
        .unwrap_or_default()
}
