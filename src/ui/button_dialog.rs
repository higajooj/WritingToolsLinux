//! Add / edit a popup button.

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::app::show_message;
use crate::options::{OptionEntry, Options};
use crate::portal::validate_trigger;
use crate::prompt::CUSTOM;
use crate::providers::ProviderKind;

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
    /// Provider name; `None` for the default provider.
    pub provider: Option<String>,
    /// `None` for the model set for the provider in Settings.
    pub model: Option<String>,
    /// A reusable base action rather than a button.
    pub is_base: bool,
    /// Name of the base action to build on.
    pub base: Option<String>,
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
        if Some(name.as_str()) == current || entry.is_base {
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

/// Names of the buttons built on the base action `base`, in display order.
pub fn dependents<'a>(options: &'a Options, base: &str) -> Vec<&'a str> {
    options.iter().filter(|(_, entry)| entry.base.as_deref() == Some(base)).map(|(name, _)| name.as_str()).collect()
}

/// A base action other buttons build on must stay one.
pub fn validate_base(data: &ButtonData, options: &Options, current: Option<&str>) -> Result<(), String> {
    let Some(current) = current.filter(|_| !data.is_base) else { return Ok(()) };
    let users = dependents(options, current);
    if users.is_empty() {
        return Ok(());
    }
    Err(format!("'{current}' is the base action of {}. Pick a different base for them first.", users.join(", ")))
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
    entry.is_base = data.is_base;
    // A base only lends its instruction; the rest belongs to buttons.
    if data.is_base {
        entry.open_in_window = false;
        entry.hotkey = None;
        entry.provider = None;
        entry.model = None;
        entry.base = None;
    } else {
        entry.open_in_window = data.open_in_window;
        entry.hotkey = data.hotkey.clone();
        entry.provider = data.provider.clone();
        entry.model = data.model.clone();
        entry.base = data.base.clone();
    }
    entry
}

/// Apply an add (`current == None`) or edit to a copy of `options`. A renamed
/// button keeps its position, and a renamed base stays the base of its buttons.
pub fn apply(options: &Options, current: Option<&str>, data: &ButtonData) -> Options {
    let entry = build_entry(data, current.and_then(|name| options.get(name)));
    match current {
        Some(current) => options
            .iter()
            .map(|(name, value)| {
                if name == current {
                    return (data.name.clone(), entry.clone());
                }
                let mut value = value.clone();
                if value.base.as_deref() == Some(current) {
                    value.base = Some(data.name.clone());
                }
                (name.clone(), value)
            })
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

const INSTRUCTION_HEADING: &str = "What should your AI do with your selected text? (System Instruction)";
const BASE_INSTRUCTION_HEADING: &str =
    "What should every button built on this base follow? (Comes before their own System Instruction)";

/// Show the dialog. `bases` names the base actions a button can build on and
/// `default_provider` the provider used by buttons that do not choose one.
/// `accept` returns an error message to keep it open.
pub fn open(
    parent: &gtk::Window,
    title: &str,
    initial: Option<(&str, &OptionEntry)>,
    bases: &[String],
    default_provider: ProviderKind,
    accept: impl Fn(ButtonData) -> Result<(), (String, String)> + 'static,
) {
    let dialog = adw::Window::builder()
        .title(title)
        .modal(true)
        .transient_for(parent)
        .default_width(1000)
        .default_height(700)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);

    let name = gtk::Entry::new();
    content.append(&heading("Button Name:"));
    content.append(&name);

    let is_base =
        gtk::CheckButton::with_label("Reusable base action (not shown as a button; other buttons can build on it)");
    content.append(&is_base);

    // Only buttons build on a base, and only when there is one to pick.
    let base_section = gtk::Box::new(gtk::Orientation::Vertical, 8);
    base_section.append(&heading("Base action (optional):"));
    let mut base_names = vec!["None"];
    base_names.extend(bases.iter().map(String::as_str));
    let base = gtk::DropDown::from_strings(&base_names);
    base.set_halign(gtk::Align::Start);
    base_section.append(&base);
    content.append(&base_section);

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
    let scroller = gtk::ScrolledWindow::builder().child(&instruction).min_content_height(160).vexpand(true).build();
    scroller.add_css_class("card");
    instruction_overlay.set_child(Some(&scroller));
    instruction_overlay.add_overlay(&placeholder);
    let buffer = instruction.buffer();
    buffer.connect_changed({
        let placeholder = placeholder.clone();
        move |buffer| placeholder.set_visible(buffer.char_count() == 0)
    });
    let instruction_heading = heading(INSTRUCTION_HEADING);
    content.append(&instruction_heading);
    content.append(&instruction_overlay);

    // Everything below only applies to buttons, not base actions.
    let button_section = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.append(&button_section);

    button_section.append(&heading("How should your AI response be shown?"));
    let clipboard = gtk::CheckButton::with_label("Copy to clipboard");
    let window = gtk::CheckButton::with_label("In a pop-up window (with follow-up support)");
    window.set_group(Some(&clipboard));
    let radios = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    radios.append(&clipboard);
    radios.append(&window);
    button_section.append(&radios);

    button_section.append(&heading("Popup shortcut (optional):"));
    let hotkey = gtk::Entry::builder().placeholder_text("e.g. ctrl+j  (leave blank for none)").build();
    button_section.append(&hotkey);
    let hint = gtk::Label::new(Some(
        "Open Writing Tools, then press this combination while its popup is active to run this button immediately.\nUse '+' between keys, e.g. ctrl+j or ctrl+shift+p.",
    ));
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");
    button_section.append(&hint);

    button_section.append(&heading("AI provider and model (optional):"));
    let default_label = format!("Default ({})", default_provider.name());
    let mut providers = vec![default_label.as_str()];
    providers.extend(ProviderKind::ALL.iter().map(|kind| kind.name()));
    let provider = gtk::DropDown::from_strings(&providers);
    let model = gtk::Entry::builder().hexpand(true).build();
    let update_model = {
        let model = model.clone();
        move |provider: &gtk::DropDown| {
            let own = provider.selected() > 0;
            model.set_sensitive(own);
            model.set_placeholder_text(Some(if own {
                "Model for this button  (leave blank for the provider's model in Settings)"
            } else {
                "Uses the default provider's model"
            }));
        }
    };
    provider.connect_selected_notify(update_model.clone());
    let provider_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    provider_row.append(&provider);
    provider_row.append(&model);
    button_section.append(&provider_row);

    match initial {
        Some((initial_name, entry)) => {
            name.set_text(initial_name);
            buffer.set_text(&entry.instruction);
            window.set_active(entry.open_in_window);
            clipboard.set_active(!entry.open_in_window);
            hotkey.set_text(entry.hotkey.as_deref().unwrap_or_default());
            is_base.set_active(entry.is_base);
            if let Some(index) = entry.base.as_deref().and_then(|name| bases.iter().position(|b| b == name)) {
                base.set_selected(index as u32 + 1);
            }
            let kind = entry.provider.as_deref().and_then(ProviderKind::from_name);
            if let Some(index) = kind.and_then(|kind| ProviderKind::ALL.iter().position(|k| *k == kind)) {
                provider.set_selected(index as u32 + 1);
                model.set_text(entry.model.as_deref().unwrap_or_default());
            }
        }
        None => clipboard.set_active(true),
    }
    placeholder.set_visible(buffer.char_count() == 0);
    update_model(&provider);
    let update_kind = {
        let has_bases = !bases.is_empty();
        let base_section = base_section.clone();
        let button_section = button_section.clone();
        move |is_base: &gtk::CheckButton| {
            let button = !is_base.is_active();
            base_section.set_visible(button && has_bases);
            button_section.set_visible(button);
            instruction_heading.set_label(if button { INSTRUCTION_HEADING } else { BASE_INSTRUCTION_HEADING });
        }
    };
    is_base.connect_toggled(update_kind.clone());
    update_kind(&is_base);

    let cancel = gtk::Button::with_label("Cancel");
    let ok = gtk::Button::with_label("OK");
    ok.add_css_class("suggested-action");
    let header = adw::HeaderBar::builder().show_end_title_buttons(false).show_start_title_buttons(false).build();
    header.pack_start(&cancel);
    header.pack_end(&ok);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    dialog.set_content(Some(&toolbar));
    dialog.set_default_widget(Some(&ok));
    name.set_activates_default(true);
    hotkey.set_activates_default(true);
    model.set_activates_default(true);

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
        let bases = bases.to_vec();
        move |_| {
            let Some(dialog) = dialog.upgrade() else { return };
            let hotkey = hotkey.text().trim().to_lowercase();
            // Position 0 is the default provider, which also decides the model.
            let kind = (provider.selected() as usize).checked_sub(1).and_then(|i| ProviderKind::ALL.get(i));
            let model = model.text().trim().to_owned();
            let data = ButtonData {
                name: name.text().trim().to_owned(),
                instruction: buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string(),
                open_in_window: window.is_active(),
                hotkey: (!hotkey.is_empty()).then_some(hotkey),
                provider: kind.map(|kind| kind.name().to_owned()),
                model: kind.filter(|_| !model.is_empty()).map(|_| model),
                is_base: is_base.is_active(),
                // Position 0 is no base.
                base: (base.selected() as usize).checked_sub(1).and_then(|i| bases.get(i)).cloned(),
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
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed({
        let dialog = dialog.downgrade();
        move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(dialog) = dialog.upgrade() {
                    dialog.close();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    dialog.add_controller(escape);
    dialog.present();
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
            provider: Some("OpenRouter".into()),
            model: Some("x/y".into()),
            ..Default::default()
        };
        let added = apply(&options, None, &data);
        let entry = &added["Shout"];
        assert_eq!((entry.provider.as_deref(), entry.model.as_deref()), (Some("OpenRouter"), Some("x/y")));
        assert_eq!((entry.prefix.as_str(), entry.icon.as_str()), (DEFAULT_PREFIX, DEFAULT_ICON));
        assert_eq!(added.keys().last().map(String::as_str), Some("Shout"));

        // Renaming keeps the position, prefix, icon and unknown fields; clearing
        // the hotkey or the provider drops it.
        let mut options = options;
        let rewrite = options.get_mut("Rewrite").unwrap();
        rewrite.extra.insert("future".into(), serde_json::json!(1));
        rewrite.provider = Some("OpenRouter".into());
        rewrite.model = Some("x/y".into());
        let data = ButtonData { name: "Rephrase".into(), instruction: "x".into(), ..Default::default() };
        let edited = apply(&options, Some("Rewrite"), &data);
        let names: Vec<&str> = edited.keys().map(String::as_str).collect();
        assert_eq!(&names[..3], ["Proofread", "Rephrase", "Friendly"]);
        let entry = &edited["Rephrase"];
        assert_eq!(entry.prefix, "Rewrite this:\n\n");
        assert_eq!(entry.icon, "icons/rewrite");
        assert_eq!(entry.hotkey, None);
        assert_eq!((entry.provider.as_deref(), entry.model.as_deref()), (None, None));
        assert_eq!(entry.extra["future"], 1);
        assert!(!edited.contains_key("Rewrite"));
    }

    #[test]
    fn base_actions() {
        let mut options = options();
        let base = ButtonData { name: "Style".into(), instruction: "No em-dashes.".into(), ..Default::default() };
        options = apply(&options, None, &ButtonData { is_base: true, ..base.clone() });
        options.get_mut("Rewrite").unwrap().base = Some("Style".into());
        assert_eq!(dependents(&options, "Style"), ["Rewrite"]);

        // A base never keeps a button's settings.
        let entry = build_entry(
            &ButtonData {
                is_base: true,
                open_in_window: true,
                hotkey: Some("ctrl+9".into()),
                provider: Some("OpenRouter".into()),
                model: Some("x/y".into()),
                base: Some("Other".into()),
                ..base.clone()
            },
            None,
        );
        assert!(entry.is_base && !entry.open_in_window);
        assert_eq!((entry.hotkey, entry.provider, entry.model, entry.base), (None, None, None, None));

        // Its buttons follow a rename, and it can't stop being a base while used.
        let renamed =
            apply(&options, Some("Style"), &ButtonData { name: "Tone".into(), is_base: true, ..base.clone() });
        assert_eq!(renamed["Rewrite"].base.as_deref(), Some("Tone"));
        assert!(validate_base(&ButtonData { is_base: true, ..base.clone() }, &options, Some("Style")).is_ok());
        assert!(validate_base(&base, &options, None).is_ok());
        assert!(validate_base(&base, &options, Some("Style")).unwrap_err().contains("base action of Rewrite"));

        // Base actions bind no shortcut, so theirs can't conflict.
        options.get_mut("Style").unwrap().hotkey = Some("ctrl+8".into());
        assert!(validate_hotkey(Some("ctrl+8"), &options, "ctrl+space", None).is_ok());
    }
}
