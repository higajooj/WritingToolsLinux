//! Add / edit a popup button.

use adw::prelude::*;

use crate::app::show_message;
use crate::options::{OptionEntry, Options};
use crate::portal::validate_trigger;
use crate::prompt::CUSTOM;

pub const DEFAULT_PREFIX: &str = "Make this change to the following text:\n\n";
pub const DEFAULT_ICON: &str = "icons/custom";

/// What the dialog collects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ButtonData {
    pub name: String,
    pub instruction: String,
    pub open_in_window: bool,
    /// Lower-cased; `None` for no popup shortcut.
    pub hotkey: Option<String>,
}

/// Modifier-insensitive identity of a shortcut, so `Ctrl+J` and `control+j` match.
pub fn shortcut_identity(trigger: &str) -> String {
    let mut modifiers = Vec::new();
    let mut keys = Vec::new();
    for part in trigger.split('+').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()) {
        match part.as_str() {
            "ctrl" | "control" => modifiers.push("ctrl"),
            "alt" => modifiers.push("alt"),
            "shift" => modifiers.push("shift"),
            "super" | "meta" | "win" | "cmd" => modifiers.push("super"),
            _ => keys.push(part),
        }
    }
    modifiers.sort_unstable();
    modifiers.dedup();
    modifiers.into_iter().map(str::to_owned).chain(keys).collect::<Vec<_>>().join("+")
}

/// Names key options.json, so a duplicate would silently replace another
/// button and "Custom" would replace the typed-change prompt. `current` is the
/// button being edited, which may keep its name.
pub fn validate_name(name: &str, options: &Options, current: Option<&str>) -> Result<(), String> {
    if name.is_empty() {
        return Err("Please give the button a name.".into());
    }
    if name == CUSTOM {
        return Err("'Custom' is reserved for changes typed into the popup. Pick a different name.".into());
    }
    if Some(name) != current && options.contains_key(name) {
        return Err(format!("A button named '{name}' already exists. Pick a different name."));
    }
    Ok(())
}

/// Check a popup shortcut for syntax and conflicts. No shortcut is always fine.
pub fn validate_hotkey(
    hotkey: Option<&str>,
    options: &Options,
    main_shortcut: &str,
    current: Option<&str>,
) -> Result<(), String> {
    let Some(hotkey) = hotkey else { return Ok(()) };
    if let Err(problem) = validate_trigger(hotkey) {
        return Err(format!("Invalid shortcut: '{hotkey}'.\n\n{problem}"));
    }
    let identity = shortcut_identity(hotkey);
    // The compositor may still dispatch the main shortcut while the popup is
    // active, so the same combination is not bound locally.
    if identity == shortcut_identity(main_shortcut) {
        return Err(format!(
            "'{hotkey}' is already used as the main Writing Tools hotkey (set in Settings). Pick a different combination."
        ));
    }
    for (name, entry) in options {
        if Some(name.as_str()) == current {
            continue;
        }
        if let Some(other) = entry.hotkey.as_deref().map(str::trim).filter(|h| !h.is_empty())
            && shortcut_identity(other) == identity
        {
            return Err(format!("'{hotkey}' is already used by the '{name}' button. Pick a different combination."));
        }
    }
    Ok(())
}

/// The options.json entry for dialog output. Other fields of an existing
/// entry (including its prefix and icon) are kept.
pub fn build_entry(data: &ButtonData, existing: Option<&OptionEntry>) -> OptionEntry {
    let mut entry = existing.cloned().unwrap_or_else(|| OptionEntry {
        prefix: DEFAULT_PREFIX.into(),
        icon: DEFAULT_ICON.into(),
        ..Default::default()
    });
    entry.instruction = data.instruction.clone();
    entry.open_in_window = data.open_in_window;
    entry.hotkey = data.hotkey.clone();
    entry
}

/// Apply an add (`current == None`) or edit to a copy of `options`. A renamed
/// button keeps its position.
pub fn apply(options: &Options, current: Option<&str>, data: &ButtonData) -> Options {
    let entry = build_entry(data, current.and_then(|name| options.get(name)));
    match current {
        Some(current) => options
            .iter()
            .map(
                |(name, value)| {
                    if name == current { (data.name.clone(), entry.clone()) } else { (name.clone(), value.clone()) }
                },
            )
            .collect(),
        None => {
            let mut updated = options.clone();
            updated.insert(data.name.clone(), entry);
            updated
        }
    }
}

const INSTRUCTION_EXAMPLES: &str = "Examples:
    - Fix / improve / explain this code.
    - Make it funny.
    - Add emojis!
    - Roast this!
    - Make the text title case.
    - If it's all caps, make it all small, and vice-versa.
    - Write a reply to this.
    - Analyse potential biases in this news article.";

fn heading(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.add_css_class("heading");
    label
}

/// Show the dialog. `accept` returns an error message to keep it open.
pub fn open(
    parent: &gtk::Window,
    title: &str,
    initial: Option<(&str, &OptionEntry)>,
    accept: impl Fn(ButtonData) -> Result<(), (String, String)> + 'static,
) {
    let dialog = adw::Dialog::builder().title(title).content_width(520).build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);

    let name = gtk::Entry::new();
    content.append(&heading("Button Name:"));
    content.append(&name);

    let instruction = gtk::TextView::builder().wrap_mode(gtk::WrapMode::WordChar).accepts_tab(false).build();
    instruction.add_css_class("card-textview");
    let placeholder = gtk::Label::new(Some(INSTRUCTION_EXAMPLES));
    placeholder.set_xalign(0.0);
    placeholder.set_valign(gtk::Align::Start);
    placeholder.add_css_class("dim-label");
    placeholder.set_can_target(false);
    placeholder.set_margin_start(8);
    placeholder.set_margin_top(6);
    let instruction_overlay = gtk::Overlay::new();
    let scroller = gtk::ScrolledWindow::builder().child(&instruction).min_content_height(160).build();
    scroller.add_css_class("card");
    instruction_overlay.set_child(Some(&scroller));
    instruction_overlay.add_overlay(&placeholder);
    let buffer = instruction.buffer();
    buffer.connect_changed({
        let placeholder = placeholder.clone();
        move |buffer| placeholder.set_visible(buffer.char_count() == 0)
    });
    content.append(&heading("What should your AI do with your selected text? (System Instruction)"));
    content.append(&instruction_overlay);

    content.append(&heading("How should your AI response be shown?"));
    let clipboard = gtk::CheckButton::with_label("Copy to clipboard");
    let window = gtk::CheckButton::with_label("In a pop-up window (with follow-up support)");
    window.set_group(Some(&clipboard));
    let radios = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    radios.append(&clipboard);
    radios.append(&window);
    content.append(&radios);

    content.append(&heading("Popup shortcut (optional):"));
    let hotkey = gtk::Entry::builder().placeholder_text("e.g. ctrl+j  (leave blank for none)").build();
    content.append(&hotkey);
    let hint = gtk::Label::new(Some(
        "Open Writing Tools, then press this combination while its popup is active to run this button immediately.\nUse '+' between keys, e.g. ctrl+j or ctrl+shift+p.",
    ));
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");
    content.append(&hint);

    match initial {
        Some((initial_name, entry)) => {
            name.set_text(initial_name);
            buffer.set_text(&entry.instruction);
            window.set_active(entry.open_in_window);
            clipboard.set_active(!entry.open_in_window);
            hotkey.set_text(entry.hotkey.as_deref().unwrap_or_default());
        }
        None => clipboard.set_active(true),
    }
    placeholder.set_visible(buffer.char_count() == 0);

    let cancel = gtk::Button::with_label("Cancel");
    let ok = gtk::Button::with_label("OK");
    ok.add_css_class("suggested-action");
    let header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
    header.pack_start(&cancel);
    header.pack_end(&ok);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));
    dialog.set_default_widget(Some(&ok));
    name.set_activates_default(true);
    hotkey.set_activates_default(true);

    cancel.connect_clicked({
        let dialog = dialog.downgrade();
        move |_| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
        }
    });
    ok.connect_clicked({
        let dialog = dialog.downgrade();
        move |_| {
            let Some(dialog) = dialog.upgrade() else { return };
            let hotkey = hotkey.text().trim().to_lowercase();
            let data = ButtonData {
                name: name.text().trim().to_owned(),
                instruction: buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string(),
                open_in_window: window.is_active(),
                hotkey: (!hotkey.is_empty()).then_some(hotkey),
            };
            // A rejected entry keeps the dialog open with the user's input.
            match accept(data) {
                Ok(()) => {
                    dialog.close();
                }
                Err((title, message)) => {
                    show_message(Some(&dialog), &title, &message);
                }
            }
        }
    });
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Options {
        let mut options = crate::options::defaults();
        options.get_mut("Rewrite").unwrap().hotkey = Some("ctrl+2".into());
        options
    }

    #[test]
    fn identities() {
        assert_eq!(shortcut_identity("Ctrl + J"), shortcut_identity("control+j"));
        assert_eq!(shortcut_identity("shift+ctrl+p"), shortcut_identity("ctrl+shift+p"));
        assert_ne!(shortcut_identity("ctrl+j"), shortcut_identity("alt+j"));
        assert_eq!(shortcut_identity("super+space"), shortcut_identity("meta+space"));
    }

    #[test]
    fn names() {
        let options = options();
        assert!(validate_name("New", &options, None).is_ok());
        assert_eq!(validate_name("", &options, None).unwrap_err(), "Please give the button a name.");
        assert!(validate_name("Custom", &options, None).unwrap_err().contains("reserved"));
        assert!(validate_name("Proofread", &options, None).unwrap_err().contains("already exists"));
        assert!(validate_name("Proofread", &options, Some("Proofread")).is_ok());
        assert!(validate_name("Rewrite", &options, Some("Proofread")).is_err());
    }

    #[test]
    fn hotkeys() {
        let options = options();
        assert!(validate_hotkey(None, &options, "ctrl+space", None).is_ok());
        assert!(validate_hotkey(Some("ctrl+1"), &options, "ctrl+space", None).is_ok());
        assert!(
            validate_hotkey(Some("j"), &options, "ctrl+space", None).unwrap_err().starts_with("Invalid shortcut: 'j'.")
        );
        assert!(
            validate_hotkey(Some("control+space"), &options, "ctrl+space", None)
                .unwrap_err()
                .contains("main Writing Tools hotkey")
        );
        assert!(
            validate_hotkey(Some("control+2"), &options, "ctrl+space", None).unwrap_err().contains("'Rewrite' button")
        );
        assert!(validate_hotkey(Some("ctrl+2"), &options, "ctrl+space", Some("Rewrite")).is_ok());
    }

    #[test]
    fn add_and_edit_entries() {
        let options = options();
        let data = ButtonData {
            name: "Shout".into(),
            instruction: "CAPS".into(),
            open_in_window: true,
            hotkey: Some("ctrl+9".into()),
        };
        let added = apply(&options, None, &data);
        let entry = &added["Shout"];
        assert_eq!((entry.prefix.as_str(), entry.icon.as_str()), (DEFAULT_PREFIX, DEFAULT_ICON));
        assert_eq!(added.keys().last().map(String::as_str), Some("Shout"));

        // Renaming keeps the position, prefix, icon and unknown fields; clearing the hotkey drops it.
        let mut options = options;
        options.get_mut("Rewrite").unwrap().extra.insert("future".into(), serde_json::json!(1));
        let data = ButtonData { name: "Rephrase".into(), instruction: "x".into(), open_in_window: false, hotkey: None };
        let edited = apply(&options, Some("Rewrite"), &data);
        let names: Vec<&str> = edited.keys().map(String::as_str).collect();
        assert_eq!(&names[..3], ["Proofread", "Rephrase", "Friendly"]);
        let entry = &edited["Rephrase"];
        assert_eq!(entry.prefix, "Rewrite this:\n\n");
        assert_eq!(entry.icon, "icons/rewrite");
        assert_eq!(entry.hotkey, None);
        assert_eq!(entry.extra["future"], 1);
        assert!(!edited.contains_key("Rewrite"));
    }
}
