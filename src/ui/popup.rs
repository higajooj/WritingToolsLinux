//! The shortcut popup: a free-text change, the option buttons, and the
//! button editor.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use super::button_dialog::{self, ButtonData, shortcut_identity};
use crate::app::{App, show_message};
use crate::icons;
use crate::options::{self, Options};
use crate::portal::validate_trigger;
use crate::prompt::CUSTOM;

const TITLE: &str = "Writing Tools";
const PLACEHOLDER: &str = "Describe your change...";
const PLACEHOLDER_SELECTED: &str = "Add instructions (optional)...";

/// Translate `ctrl+shift+j` into GTK's `<Control><Shift>j` trigger syntax.
pub fn gtk_trigger(trigger: &str) -> String {
    let mut result = String::new();
    let mut key = String::new();
    for part in trigger.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => result.push_str("<Control>"),
            "alt" => result.push_str("<Alt>"),
            "shift" => result.push_str("<Shift>"),
            "super" | "meta" | "win" | "cmd" => result.push_str("<Super>"),
            "space" => key = "space".into(),
            "enter" | "return" => key = "Return".into(),
            "tab" => key = "Tab".into(),
            "esc" | "escape" => key = "Escape".into(),
            lower if lower.len() > 1 && lower.starts_with('f') && lower[1..].parse::<u8>().is_ok() => {
                key = lower.to_uppercase()
            }
            lower if lower.chars().count() == 1 => key = lower.into(),
            _ => key = part.into(),
        }
    }
    result + &key
}

struct Popup {
    app: Rc<App>,
    window: gtk::Window,
    entry: gtk::Entry,
    input_row: gtk::Box,
    grid: gtk::Grid,
    add_button: gtk::Button,
    edit_button: gtk::Button,
    reset_button: gtk::Button,
    close_button: gtk::Button,
    drag_label: gtk::Label,
    shortcuts: gtk::ShortcutController,
    edit_mode: Cell<bool>,
    selected: RefCell<Option<String>>,
    buttons: RefCell<Vec<(String, gtk::Button)>>,
    drop_indicator: RefCell<Option<gtk::Button>>,
}

pub fn show(app: &Rc<App>) -> gtk::Window {
    // Wayland cannot position windows; placement is up to the compositor,
    // matched by this title (see README window rules).
    let window = gtk::Window::builder().application(&app.gtk).title(TITLE).decorated(false).resizable(false).build();
    window.add_css_class("popup");

    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(4);
    content.set_margin_bottom(10);
    content.set_margin_start(10);
    content.set_margin_end(10);

    let edit_button = icons::button("pencil", 16, "Edit buttons");
    let drag_label = gtk::Label::new(Some("Drag to rearrange"));
    drag_label.add_css_class("heading");
    drag_label.set_hexpand(true);
    drag_label.set_visible(false);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let reset_button = icons::button("restore", 16, "Reset to defaults");
    reset_button.set_visible(false);
    let close_button = gtk::Button::with_label("×");
    close_button.set_tooltip_text(Some("Close"));
    close_button.add_css_class("popup-close");
    for button in [&edit_button, &reset_button, &close_button] {
        button.add_css_class("flat");
        button.add_css_class("popup-bar-button");
    }
    let top_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    top_bar.append(&edit_button);
    top_bar.append(&drag_label);
    top_bar.append(&spacer);
    top_bar.append(&reset_button);
    top_bar.append(&close_button);
    content.append(&top_bar);

    let entry = gtk::Entry::builder().placeholder_text(PLACEHOLDER).hexpand(true).build();
    let send = icons::button("send", 16, "Send");
    send.add_css_class("suggested-action");
    let input_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    input_row.append(&entry);
    input_row.append(&send);
    content.append(&input_row);

    let grid = gtk::Grid::builder().row_spacing(10).column_spacing(10).column_homogeneous(true).build();
    content.append(&grid);
    let add_button = gtk::Button::with_label("+ Add New");
    add_button.set_visible(false);
    content.append(&add_button);
    window.set_child(Some(&content));

    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_scope(gtk::ShortcutScope::Local);
    window.add_controller(shortcuts.clone());

    let popup = Rc::new(Popup {
        app: app.clone(),
        window: window.clone(),
        entry: entry.clone(),
        input_row,
        grid,
        add_button: add_button.clone(),
        edit_button: edit_button.clone(),
        reset_button: reset_button.clone(),
        close_button: close_button.clone(),
        drag_label,
        shortcuts,
        edit_mode: Cell::new(false),
        selected: RefCell::new(None),
        buttons: RefCell::new(Vec::new()),
        drop_indicator: RefCell::new(None),
    });

    // Options may have been edited by another popup; start from the file.
    match options::load() {
        Ok(fresh) => *app.options.borrow_mut() = fresh,
        Err(e) => log::error!("Could not reload options: {e}"),
    }
    popup.rebuild();

    let weak = Rc::downgrade(&popup);
    let with = move |f: fn(&Rc<Popup>)| {
        let weak = weak.clone();
        move |_: &gtk::Button| {
            if let Some(popup) = weak.upgrade() {
                f(&popup);
            }
        }
    };
    edit_button.connect_clicked(with(Popup::toggle_edit_mode));
    reset_button.connect_clicked(with(Popup::confirm_reset));
    add_button.connect_clicked(with(Popup::add_button_clicked));
    send.connect_clicked(with(Popup::submit));
    close_button.connect_clicked({
        let window = window.clone();
        move |_| window.close()
    });
    entry.connect_activate({
        let weak = Rc::downgrade(&popup);
        move |_| {
            if let Some(popup) = weak.upgrade() {
                popup.submit();
            }
        }
    });

    // No hide-on-deactivate: under focus-follows-mouse, moving the pointer
    // away would dismiss the popup. It closes on Escape, the close button,
    // submitting, or pressing the hotkey again.
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed({
        let window = window.clone();
        move |_, key, _, _| {
            if key == gdk::Key::Escape {
                window.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    window.add_controller(escape);

    window.connect_close_request({
        let popup = popup.clone();
        move |window| {
            popup.app.forget_popup(window);
            glib::Propagation::Proceed
        }
    });

    window.present();
    entry.grab_focus();
    window
}

impl Popup {
    fn option_names(&self) -> Vec<String> {
        self.app.options.borrow().keys().filter(|name| *name != CUSTOM).cloned().collect()
    }

    /// Recreate the button grid and shortcuts from the current options.
    fn rebuild(self: &Rc<Self>) {
        self.drop_indicator.replace(None);
        while let Some(child) = self.grid.first_child() {
            self.grid.remove(&child);
        }
        let mut buttons = Vec::new();
        let options = self.app.options.borrow().clone();
        for (index, name) in self.option_names().into_iter().enumerate() {
            let entry = &options[&name];
            let button = self.option_button(&name, entry);
            let widget: gtk::Widget =
                if self.edit_mode.get() { self.edit_overlay(&name, &button).upcast() } else { button.clone().upcast() };
            self.grid.attach(&widget, (index % 2) as i32, (index / 2) as i32, 1, 1);
            buttons.push((name, button));
        }
        self.buttons.replace(buttons);
        self.add_button.set_visible(self.edit_mode.get());
        if self.edit_mode.get() {
            self.grid.add_css_class("editing");
        } else {
            self.grid.remove_css_class("editing");
        }
        self.rebuild_shortcuts(&options);
        self.update_selection();
    }

    fn option_button(self: &Rc<Self>, name: &str, entry: &crate::options::OptionEntry) -> gtk::Button {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&icons::image(icons::option_icon_name(&entry.icon), 18));
        let label = gtk::Label::new(Some(name));
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_xalign(0.0);
        row.append(&label);
        let button = gtk::Button::builder().child(&row).width_request(140).build();
        button.add_css_class("option-button");
        // Surface popup-local shortcuts without cluttering buttons lacking one.
        if let Some(hotkey) = entry.hotkey.as_deref().map(str::trim).filter(|h| !h.is_empty()) {
            button.set_tooltip_text(Some(&format!("Popup shortcut (while active): {hotkey}")));
        }

        let weak = Rc::downgrade(self);
        let key = name.to_owned();
        button.connect_clicked(move |_| {
            let Some(popup) = weak.upgrade() else { return };
            if popup.edit_mode.get() {
                return;
            }
            // Clicking the selected button again returns to a custom change.
            let selected = popup.selected.borrow().clone();
            popup.selected.replace(if selected.as_deref() == Some(&key) { None } else { Some(key.clone()) });
            popup.update_selection();
            popup.entry.grab_focus();
        });

        self.add_drag_and_drop(name, &button);
        button
    }

    fn edit_overlay(self: &Rc<Self>, name: &str, button: &gtk::Button) -> gtk::Overlay {
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(button));
        let edit = icons::button("pencil", 10, "Edit");
        edit.set_halign(gtk::Align::Start);
        let delete = icons::button("cross", 10, "Delete");
        delete.set_halign(gtk::Align::End);
        for corner in [&edit, &delete] {
            corner.set_valign(gtk::Align::Start);
            corner.add_css_class("circular");
            corner.add_css_class("corner-button");
            overlay.add_overlay(corner);
        }
        let weak = Rc::downgrade(self);
        let key = name.to_owned();
        edit.connect_clicked(move |_| {
            if let Some(popup) = weak.upgrade() {
                popup.edit_button_clicked(&key);
            }
        });
        let weak = Rc::downgrade(self);
        let key = name.to_owned();
        delete.connect_clicked(move |_| {
            if let Some(popup) = weak.upgrade() {
                popup.confirm_delete(&key);
            }
        });
        overlay
    }

    fn update_selection(&self) {
        let selected = self.selected.borrow();
        for (name, button) in self.buttons.borrow().iter() {
            if selected.as_deref() == Some(name.as_str()) {
                button.add_css_class("selected");
            } else {
                button.remove_css_class("selected");
            }
        }
        // The highlighted button already names the action.
        self.entry.set_placeholder_text(Some(if selected.is_some() { PLACEHOLDER_SELECTED } else { PLACEHOLDER }));
    }

    fn submit(self: &Rc<Self>) {
        let text = self.entry.text().trim().to_owned();
        let extra = (!text.is_empty()).then_some(text);
        let selected = self.selected.borrow().clone();
        match (selected, extra) {
            (Some(option), extra) => {
                self.app.process_option(option, extra);
                self.window.close();
            }
            (None, Some(text)) => {
                self.app.process_option(CUSTOM.into(), Some(text));
                self.window.close();
            }
            (None, None) => {}
        }
    }

    // ------------------------------------------------------------ shortcuts

    /// Bind each button's shortcut to this popup only. Invalid, duplicate, or
    /// main-hotkey combinations are skipped with a warning.
    fn rebuild_shortcuts(self: &Rc<Self>, options: &Options) {
        while let Some(shortcut) = self.shortcuts.item(0).and_downcast::<gtk::Shortcut>() {
            self.shortcuts.remove_shortcut(&shortcut);
        }
        if self.edit_mode.get() {
            return;
        }
        let mut taken = vec![shortcut_identity(self.app.config.borrow().shortcut())];
        for (name, entry) in options {
            if name == CUSTOM {
                continue;
            }
            let Some(trigger_text) = entry.hotkey.as_deref().map(str::trim).filter(|h| !h.is_empty()) else { continue };
            let trigger = validate_trigger(trigger_text).map_err(str::to_owned).and_then(|()| {
                gtk::ShortcutTrigger::parse_string(&gtk_trigger(trigger_text))
                    .ok_or_else(|| "GTK could not parse the key combination".to_owned())
            });
            let trigger = match trigger {
                Ok(trigger) => trigger,
                Err(problem) => {
                    log::warn!("Ignoring invalid popup shortcut \"{trigger_text}\" for \"{name}\": {problem}");
                    continue;
                }
            };
            let identity = shortcut_identity(trigger_text);
            if taken.contains(&identity) {
                log::warn!("Ignoring duplicate popup shortcut \"{trigger_text}\" for \"{name}\"");
                continue;
            }
            taken.push(identity);
            let weak = Rc::downgrade(self);
            let key = name.clone();
            let action = gtk::CallbackAction::new(move |_, _| {
                if let Some(popup) = weak.upgrade()
                    && !popup.edit_mode.get()
                    && popup.window.is_active()
                {
                    popup.app.process_option(key.clone(), None);
                    popup.window.close();
                }
                glib::Propagation::Stop
            });
            self.shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
        }
    }

    // ------------------------------------------------------------ edit mode

    fn toggle_edit_mode(self: &Rc<Self>) {
        let editing = !self.edit_mode.get();
        self.edit_mode.set(editing);
        if editing {
            self.selected.replace(None);
        }
        self.edit_button.set_child(Some(&icons::image(if editing { "check" } else { "pencil" }, 16)));
        self.edit_button.set_tooltip_text(Some(if editing { "Done" } else { "Edit buttons" }));
        self.close_button.set_visible(!editing);
        self.reset_button.set_visible(editing);
        self.drag_label.set_visible(editing);
        self.input_row.set_visible(!editing);
        self.rebuild();
        self.fit();
        if !editing {
            self.entry.grab_focus();
        }
    }

    /// A toplevel grows with its content but does not shrink on its own.
    fn fit(&self) {
        self.window.set_default_size(-1, -1);
        self.window.queue_resize();
    }

    /// Persist and apply an options change. Returns false (after telling the
    /// user) when saving failed.
    fn commit(self: &Rc<Self>, updated: Options, action: &str) -> bool {
        if let Err(e) = options::save(&updated) {
            log::error!("Error saving options.json: {e}");
            show_message(Some(&self.window), "Error", &format!("An error occurred while {action}: {e}"));
            return false;
        }
        *self.app.options.borrow_mut() = updated;
        self.rebuild();
        self.fit();
        true
    }

    fn confirm(&self, heading: &str, body: &str, on_yes: impl Fn() + 'static) {
        let dialog = adw::AlertDialog::new(Some(heading), Some(body));
        dialog.add_responses(&[("no", "No"), ("yes", "Yes")]);
        dialog.set_response_appearance("yes", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("no"));
        dialog.set_close_response("no");
        dialog.connect_response(None, move |_, response| {
            if response == "yes" {
                on_yes();
            }
        });
        dialog.present(Some(&self.window));
    }

    fn confirm_reset(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.confirm(
            "Confirm Reset to Defaults?",
            "Reset all buttons to their original configuration? This will replace your custom buttons and edits.",
            move || {
                if let Some(popup) = weak.upgrade() {
                    popup.commit(options::defaults(), "resetting the buttons");
                }
            },
        );
    }

    fn confirm_delete(self: &Rc<Self>, name: &str) {
        let weak = Rc::downgrade(self);
        let key = name.to_owned();
        self.confirm("Confirm Delete?", &format!("Delete the '{name}' button?"), move || {
            if let Some(popup) = weak.upgrade() {
                let mut updated = popup.app.options.borrow().clone();
                updated.shift_remove(&key);
                popup.commit(updated, "deleting the button");
            }
        });
    }

    fn add_button_clicked(self: &Rc<Self>) {
        self.open_editor("Add New Button", None);
    }

    fn edit_button_clicked(self: &Rc<Self>, name: &str) {
        self.open_editor("Edit Button", Some(name.to_owned()));
    }

    fn open_editor(self: &Rc<Self>, title: &str, current: Option<String>) {
        let options = self.app.options.borrow().clone();
        let initial_name = current.clone();
        let initial = initial_name.as_deref().and_then(|name| options.get(name).map(|entry| (name, entry)));
        let weak = Rc::downgrade(self);
        let accept = move |data: ButtonData| -> Result<(), (String, String)> {
            let Some(popup) = weak.upgrade() else { return Ok(()) };
            let options = popup.app.options.borrow().clone();
            let current = current.as_deref();
            button_dialog::validate_name(&data.name, &options, current).map_err(|e| ("Invalid name".to_owned(), e))?;
            let main = popup.app.config.borrow().shortcut().to_owned();
            button_dialog::validate_hotkey(data.hotkey.as_deref(), &options, &main, current)
                .map_err(|e| ("Invalid hotkey".to_owned(), e))?;
            let updated = button_dialog::apply(&options, current, &data);
            if popup.commit(updated, "saving the button changes") {
                Ok(())
            } else {
                // The save error was already shown; keep the user's entries.
                Err(("Error".into(), "The button could not be saved.".into()))
            }
        };
        button_dialog::open(&self.window, title, initial, accept);
    }

    // ------------------------------------------------------------ reordering

    fn add_drag_and_drop(self: &Rc<Self>, name: &str, button: &gtk::Button) {
        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::MOVE);
        let weak = Rc::downgrade(self);
        let key = name.to_owned();
        source.connect_prepare(move |_, _, _| {
            let popup = weak.upgrade()?;
            popup.edit_mode.get().then(|| gdk::ContentProvider::for_value(&key.to_value()))
        });
        let dragged = button.downgrade();
        source.connect_drag_begin(move |source, _| {
            if let Some(button) = dragged.upgrade() {
                source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&button))), 0, 0);
            }
        });
        let weak = Rc::downgrade(self);
        source.connect_drag_end(move |_, _, _| {
            // A cancelled drag has no drop to remove the indicator.
            if let Some(popup) = weak.upgrade() {
                popup.clear_drop_indicator();
            }
        });
        button.add_controller(source);

        let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        let weak = Rc::downgrade(self);
        let target_button = button.downgrade();
        target.connect_motion(move |_, x, _| {
            let (Some(popup), Some(button)) = (weak.upgrade(), target_button.upgrade()) else {
                return gdk::DragAction::empty();
            };
            if !popup.edit_mode.get() {
                return gdk::DragAction::empty();
            }
            popup.show_drop_indicator(&button, x < button.width() as f64 / 2.0);
            gdk::DragAction::MOVE
        });
        let weak = Rc::downgrade(self);
        target.connect_leave(move |_| {
            if let Some(popup) = weak.upgrade() {
                popup.clear_drop_indicator();
            }
        });
        let weak = Rc::downgrade(self);
        let target_name = name.to_owned();
        let target_button = button.downgrade();
        target.connect_drop(move |_, value, x, _| {
            let (Some(popup), Some(button)) = (weak.upgrade(), target_button.upgrade()) else { return false };
            let Ok(source) = value.get::<String>() else { return false };
            popup.clear_drop_indicator();
            if !popup.edit_mode.get() {
                return false;
            }
            let before = x < button.width() as f64 / 2.0;
            popup.move_button(&source, &target_name, before)
        });
        button.add_controller(target);
    }

    fn show_drop_indicator(&self, button: &gtk::Button, before: bool) {
        self.clear_drop_indicator();
        button.add_css_class(if before { "drop-before" } else { "drop-after" });
        self.drop_indicator.replace(Some(button.clone()));
    }

    fn clear_drop_indicator(&self) {
        if let Some(button) = self.drop_indicator.take() {
            button.remove_css_class("drop-before");
            button.remove_css_class("drop-after");
        }
    }

    /// Move `source` next to `target`. Buttons are identified by name, so a
    /// drag that outlived a rebuild cannot move the wrong button.
    fn move_button(self: &Rc<Self>, source: &str, target: &str, before: bool) -> bool {
        let names = self.option_names();
        let Some(order) = reorder(&names, source, target, before) else { return false };
        if order == names {
            return true;
        }
        let options = self.app.options.borrow().clone();
        let mut updated = Options::new();
        if let Some(custom) = options.get(CUSTOM) {
            updated.insert(CUSTOM.into(), custom.clone());
        }
        for name in order {
            updated.insert(name.clone(), options[&name].clone());
        }
        self.commit(updated, "saving the button order")
    }
}

/// The new order after dropping `source` before or after `target`.
pub fn reorder(names: &[String], source: &str, target: &str, before: bool) -> Option<Vec<String>> {
    let from = names.iter().position(|n| n == source)?;
    names.iter().position(|n| n == target)?;
    if source == target {
        return Some(names.to_vec());
    }
    let mut order = names.to_vec();
    let moved = order.remove(from);
    let target_index = order.iter().position(|n| n == target).unwrap_or(order.len());
    order.insert(if before { target_index } else { target_index + 1 }, moved);
    Some(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers() {
        assert_eq!(gtk_trigger("ctrl+j"), "<Control>j");
        assert_eq!(gtk_trigger("Ctrl + Shift + P"), "<Control><Shift>p");
        assert_eq!(gtk_trigger("super+space"), "<Super>space");
        assert_eq!(gtk_trigger("alt+f5"), "<Alt>F5");
        assert_eq!(gtk_trigger("ctrl+1"), "<Control>1");
        assert_eq!(gtk_trigger("ctrl+Page_Down"), "<Control>Page_Down");
    }

    #[test]
    fn reordering() {
        let names: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        assert_eq!(reorder(&names, "a", "c", true).unwrap(), ["b", "a", "c", "d"]);
        assert_eq!(reorder(&names, "a", "c", false).unwrap(), ["b", "c", "a", "d"]);
        assert_eq!(reorder(&names, "d", "a", true).unwrap(), ["d", "a", "b", "c"]);
        assert_eq!(reorder(&names, "b", "b", false).unwrap(), names);
        assert_eq!(reorder(&names, "x", "a", true), None);
    }
}
