//! Settings: the main shortcut and the AI provider with its options.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::glib;
use serde_json::{Map, Value};

use crate::app::{App, show_message};
use crate::icons;
use crate::portal::validate_trigger;
use crate::providers::openai_compat::is_openai_api;
use crate::providers::subscription::{
    self, AuthState, Choices, ModelList, Status, Subscription, model_id, model_metadata, reasoning_label,
    supported_efforts, supports_fast,
};
use crate::providers::{ProviderKind, SPEED_OPTIONS, SettingKind, SettingSpec, normalize_service_tier};

const CUSTOM_LABEL: &str = "🔧 Custom";

thread_local! {
    static OPEN: RefCell<glib::WeakRef<adw::Dialog>> = RefCell::new(glib::WeakRef::new());
}

/// Collects a provider's settings for config.json.
type Collector = Box<dyn Fn() -> Map<String, Value>>;

struct Settings {
    app: Rc<App>,
    dialog: adw::Dialog,
    providers_only: bool,
    shortcut: Option<adw::EntryRow>,
    provider_row: adw::ComboRow,
    provider_group: RefCell<Option<adw::PreferencesGroup>>,
    page: adw::PreferencesPage,
    collector: RefCell<Option<Collector>>,
    saved: Cell<bool>,
}

/// Show Settings. With `providers_only` (first run) only the provider is
/// asked for, and closing without saving quits the app.
pub fn show(app: &Rc<App>, providers_only: bool) {
    if let Some(dialog) = OPEN.with(|open| open.borrow().upgrade()) {
        dialog.present(None::<&gtk::Widget>);
        return;
    }
    let dialog = adw::Dialog::builder().title("Settings").content_width(600).content_height(720).build();
    OPEN.with(|open| open.borrow().set(Some(&dialog)));

    let page = adw::PreferencesPage::new();
    let shortcut = (!providers_only).then(|| {
        let group = adw::PreferencesGroup::new();
        let row = adw::EntryRow::builder().title("Shortcut Key").text(app.config.borrow().shortcut()).build();
        group.add(&row);
        page.add(&group);
        row
    });

    let names: Vec<&str> = ProviderKind::ALL.iter().map(|kind| kind.name()).collect();
    let provider_row =
        adw::ComboRow::builder().title("Choose AI Provider").model(&gtk::StringList::new(&names)).build();
    let current = app.current_kind();
    provider_row.set_selected(ProviderKind::ALL.iter().position(|kind| *kind == current).unwrap_or(0) as u32);
    let group = adw::PreferencesGroup::new();
    group.add(&provider_row);
    page.add(&group);

    let save = gtk::Button::with_label(if providers_only { "Finish AI Setup" } else { "Save" });
    save.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_end(&save);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    let settings = Rc::new(Settings {
        app: app.clone(),
        dialog: dialog.clone(),
        providers_only,
        shortcut,
        provider_row: provider_row.clone(),
        provider_group: RefCell::new(None),
        page,
        collector: RefCell::new(None),
        saved: Cell::new(false),
    });
    settings.show_provider(current);

    let weak = Rc::downgrade(&settings);
    provider_row.connect_selected_notify(move |row| {
        if let Some(settings) = weak.upgrade() {
            settings.show_provider(ProviderKind::ALL[row.selected() as usize]);
        }
    });
    let weak = Rc::downgrade(&settings);
    save.connect_clicked(move |button| {
        let Some(settings) = weak.upgrade() else { return };
        let button = button.clone();
        button.set_sensitive(false);
        glib::spawn_future_local(async move {
            settings.save().await;
            button.set_sensitive(true);
        });
    });
    // This handler owns the settings until the dialog closes. Releasing them
    // then breaks the settings -> dialog -> handler cycle.
    let owner = RefCell::new(Some(settings));
    dialog.connect_closed(move |_| {
        let Some(settings) = owner.take() else { return };
        if settings.providers_only && !settings.saved.get() {
            settings.app.exit();
        }
    });
    dialog.present(None::<&gtk::Widget>);
}

impl Settings {
    fn selected_kind(&self) -> ProviderKind {
        ProviderKind::ALL[self.provider_row.selected() as usize]
    }

    fn show_provider(self: &Rc<Self>, kind: ProviderKind) {
        if let Some(old) = self.provider_group.take() {
            self.page.remove(&old);
        }
        let group = adw::PreferencesGroup::builder().title(kind.name()).description(kind.description()).build();
        if let Some(logo) = icons::themed_texture(kind.logo(), 30) {
            let image = gtk::Image::from_paintable(Some(&logo));
            image.set_pixel_size(30);
            group.set_header_suffix(Some(&image));
        }
        if let Some((label, url)) = kind.link() {
            let row = adw::ActionRow::builder().title(label).activatable(true).build();
            row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
            row.connect_activated(move |_| open_url(url));
            group.add(&row);
        }

        let config = self.app.config.borrow().provider_config(kind.name());
        let collector = match kind {
            ProviderKind::Subscription => SubscriptionPanel::build(&group, self.app.subscription.clone(), &config),
            _ => generic_rows(&group, kind.settings(), &config, kind == ProviderKind::OpenAiCompatible),
        };
        self.collector.replace(Some(collector));
        self.page.add(&group);
        self.provider_group.replace(Some(group));
    }

    async fn save(self: &Rc<Self>) {
        let kind = self.selected_kind();
        let validation = match kind {
            ProviderKind::Subscription => self.app.subscription.validate().await,
            _ => Ok(()),
        };
        if let Err(message) = validation {
            show_message(Some(&self.dialog), "Provider setup incomplete", &message);
            return;
        }
        let shortcut = self.shortcut.as_ref().map(|row| row.text().trim().to_owned());
        if let Some(shortcut) = &shortcut
            && let Err(problem) = validate_trigger(shortcut)
        {
            show_message(
                Some(&self.dialog),
                "Invalid shortcut",
                &format!("Invalid shortcut: '{shortcut}'.\n\n{problem}"),
            );
            return;
        }
        let provider_config = self.collector.borrow().as_ref().map(|collect| collect()).unwrap_or_default();

        {
            let mut config = self.app.config.borrow_mut();
            if let Some(shortcut) = shortcut {
                config.shortcut = Some(shortcut);
            }
            config.provider = Some(kind.name().to_owned());
            config.providers.insert(kind.name().to_owned(), Value::Object(provider_config));
        }
        self.app.save_config();
        self.app.activate_provider();
        if self.providers_only {
            self.app.create_tray();
        }
        self.app.register_hotkey();
        self.saved.set(true);
        self.dialog.close();
    }
}

fn open_url(url: &str) {
    if let Err(e) = open::that_detached(url) {
        log::warn!("Could not open {url}: {e}");
    }
}

/// Rows for a provider described by [`SettingSpec`]s.
fn generic_rows(
    group: &adw::PreferencesGroup,
    specs: Vec<SettingSpec>,
    config: &Map<String, Value>,
    openai_speed_rule: bool,
) -> Collector {
    let mut getters: Vec<(SettingSpec, Box<dyn Fn() -> String>)> = Vec::new();
    let mut base_row: Option<adw::EntryRow> = None;
    let mut speed_row: Option<adw::ComboRow> = None;

    for spec in specs {
        let value = spec.value(config);
        let getter: Box<dyn Fn() -> String> = match &spec.kind {
            SettingKind::Text | SettingKind::Secret => {
                let row: adw::EntryRow = if matches!(spec.kind, SettingKind::Secret) {
                    adw::PasswordEntryRow::builder().title(spec.label).build().upcast()
                } else {
                    adw::EntryRow::builder().title(spec.label).build()
                };
                row.set_text(&value);
                if !spec.placeholder.is_empty() {
                    row.set_tooltip_text(Some(spec.placeholder));
                }
                group.add(&row);
                if spec.key == "api_base" {
                    base_row = Some(row.clone());
                }
                Box::new(move || row.text().to_string())
            }
            SettingKind::Dropdown { options, custom_placeholder } => {
                let mut labels: Vec<&str> = options.iter().map(|(label, _)| *label).collect();
                if custom_placeholder.is_some() {
                    labels.push(CUSTOM_LABEL);
                }
                let row = adw::ComboRow::builder().title(spec.label).model(&gtk::StringList::new(&labels)).build();
                group.add(&row);
                let custom = adw::EntryRow::builder().title("Custom value").build();
                if let Some(placeholder) = custom_placeholder {
                    custom.set_tooltip_text(Some(placeholder));
                    group.add(&custom);
                }
                // A saved value outside the presets is a custom one.
                match options.iter().position(|(_, v)| *v == value) {
                    Some(index) => row.set_selected(index as u32),
                    None if custom_placeholder.is_some() && !value.is_empty() => {
                        row.set_selected(options.len() as u32);
                        custom.set_text(&value);
                    }
                    None => row.set_selected(0),
                }
                let values: Vec<&'static str> = options.iter().map(|(_, v)| *v).collect();
                let is_custom = {
                    let values = values.clone();
                    move |row: &adw::ComboRow| row.selected() as usize >= values.len()
                };
                custom.set_visible(custom_placeholder.is_some() && is_custom(&row));
                row.connect_selected_notify({
                    let custom = custom.clone();
                    let is_custom = is_custom.clone();
                    let has_custom = custom_placeholder.is_some();
                    move |row| {
                        let show = has_custom && is_custom(row);
                        custom.set_visible(show);
                        if show {
                            custom.grab_focus();
                        }
                    }
                });
                Box::new(move || {
                    if is_custom(&row) {
                        custom.text().trim().to_owned()
                    } else {
                        values.get(row.selected() as usize).copied().unwrap_or_default().to_owned()
                    }
                })
            }
            SettingKind::Speed { description } => {
                let row = speed_combo(description, &value);
                group.add(&row);
                speed_row = Some(row.clone());
                Box::new(move || speed_value(&row))
            }
        };
        getters.push((spec, getter));
    }

    // Speed selection only reaches the official OpenAI API.
    if let (true, Some(base), Some(speed)) = (openai_speed_rule, base_row, speed_row) {
        speed.set_sensitive(is_openai_api(&base.text()));
        base.connect_changed(move |base| speed.set_sensitive(is_openai_api(&base.text())));
    }

    Box::new(move || getters.iter().map(|(spec, get)| (spec.key.to_owned(), spec.stored(&get()))).collect())
}

fn speed_combo(description: &str, value: &str) -> adw::ComboRow {
    let labels: Vec<&str> = SPEED_OPTIONS.iter().map(|(label, _)| *label).collect();
    let row =
        adw::ComboRow::builder().title("Speed").subtitle(description).model(&gtk::StringList::new(&labels)).build();
    row.set_subtitle_lines(0);
    row.set_selected(SPEED_OPTIONS.iter().position(|(_, v)| *v == normalize_service_tier(value)).unwrap_or(0) as u32);
    row
}

fn speed_value(row: &adw::ComboRow) -> String {
    SPEED_OPTIONS.get(row.selected() as usize).map(|(_, v)| *v).unwrap_or("default").to_owned()
}

fn set_combo_items(row: &adw::ComboRow, labels: &[String]) {
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    row.set_model(Some(&gtk::StringList::new(&labels)));
}

const SPEED_DESCRIPTION: &str = "Fast uses more ChatGPT credits and depends on model and account availability.";
const NO_FAST_WARNING: &str = "The selected model does not offer Fast. Standard will be used.";

/// A warning shown as a row subtitle.
fn warning_markup(text: &str) -> String {
    format!("<span foreground=\"#d97706\">{}</span>", glib::markup_escape_text(text))
}

/// Account, model and request controls for the ChatGPT subscription.
struct SubscriptionPanel {
    account: adw::ActionRow,
    login: gtk::Button,
    cancel: gtk::Button,
    logout: gtk::Button,
    install: gtk::Button,
    model_row: adw::ComboRow,
    reasoning_row: adw::ComboRow,
    /// Shown instead of the level's description after a saved level was dropped.
    reasoning_warning: RefCell<Option<String>>,
    speed_row: adw::ComboRow,
    /// Unsaved choices; the running provider changes only on Save.
    choices: RefCell<Choices>,
    models: RefCell<Vec<Value>>,
    authoritative: Cell<bool>,
    model_ids: RefCell<Vec<String>>,
    effort_ids: RefCell<Vec<String>>,
    signed_in: Cell<bool>,
    /// Set while the combo models are rebuilt, to ignore their change signals.
    updating: Cell<bool>,
}

impl SubscriptionPanel {
    fn build(group: &adw::PreferencesGroup, subscription: Arc<Subscription>, config: &Map<String, Value>) -> Collector {
        let account = adw::ActionRow::builder().title("ChatGPT account").subtitle_lines(0).build();
        let login = gtk::Button::with_label("Sign in with ChatGPT");
        let cancel = gtk::Button::with_label("Cancel sign-in");
        let logout = gtk::Button::with_label("Sign out");
        let install = gtk::Button::with_label("Install / Update Codex");
        login.add_css_class("suggested-action");
        for button in [&login, &cancel, &logout, &install] {
            button.set_valign(gtk::Align::Center);
            account.add_suffix(button);
        }
        group.add(&account);

        let model_row = adw::ComboRow::builder().title("Model").subtitle_lines(0).build();
        let reasoning_row = adw::ComboRow::builder().title("Thinking level").subtitle_lines(0).build();
        let choices = Choices::from_config(config);
        let speed_row = speed_combo(SPEED_DESCRIPTION, &choices.service_tier);
        group.add(&model_row);
        group.add(&reasoning_row);
        group.add(&speed_row);

        let panel = Rc::new(Self {
            account,
            login: login.clone(),
            cancel: cancel.clone(),
            logout: logout.clone(),
            install: install.clone(),
            model_row: model_row.clone(),
            reasoning_row: reasoning_row.clone(),
            reasoning_warning: RefCell::new(None),
            speed_row: speed_row.clone(),
            choices: RefCell::new(choices),
            models: RefCell::new(Vec::new()),
            authoritative: Cell::new(false),
            model_ids: RefCell::new(Vec::new()),
            effort_ids: RefCell::new(Vec::new()),
            signed_in: Cell::new(false),
            updating: Cell::new(false),
        });

        login.connect_clicked({
            let subscription = subscription.clone();
            move |_| subscription.login()
        });
        cancel.connect_clicked({
            let subscription = subscription.clone();
            move |_| subscription.cancel_login()
        });
        logout.connect_clicked({
            let subscription = subscription.clone();
            move |_| subscription.logout()
        });
        install.connect_clicked(|_| open_url(subscription::INSTALL_URL));

        let weak = Rc::downgrade(&panel);
        model_row.connect_selected_notify(move |_| {
            if let Some(panel) = weak.upgrade().filter(|p| !p.updating.get()) {
                panel.model_selected();
            }
        });
        let weak = Rc::downgrade(&panel);
        reasoning_row.connect_selected_notify(move |_| {
            if let Some(panel) = weak.upgrade().filter(|p| !p.updating.get()) {
                panel.choices.borrow_mut().reasoning_effort = panel.selected_effort();
                panel.reasoning_warning.replace(None);
                panel.update_reasoning_subtitle();
            }
        });

        panel.apply_status(&subscription.status());
        let models = subscription.models();
        panel.apply_models(&ModelList { authoritative: !models.is_empty(), models });

        // Follow status and model updates while the panel exists.
        let mut status_rx = subscription.watch_status();
        let weak = Rc::downgrade(&panel);
        glib::spawn_future_local(async move {
            while status_rx.changed().await.is_ok() {
                let status = status_rx.borrow_and_update().clone();
                let Some(panel) = weak.upgrade() else { break };
                panel.apply_status(&status);
            }
        });
        let mut models_rx = subscription.watch_models();
        let weak = Rc::downgrade(&panel);
        glib::spawn_future_local(async move {
            while models_rx.changed().await.is_ok() {
                let list = models_rx.borrow_and_update().clone();
                let Some(panel) = weak.upgrade() else { break };
                panel.apply_models(&list);
            }
        });
        subscription.refresh_account();

        // The collector keeps the panel alive for as long as Settings shows it.
        Box::new(move || {
            let choices = Choices {
                model: panel.selected_model(),
                reasoning_effort: panel.selected_effort(),
                service_tier: speed_value(&panel.speed_row),
            };
            match choices.to_config() {
                Value::Object(map) => map,
                _ => Map::new(),
            }
        })
    }

    /// Until the model list is known, keep the saved model rather than
    /// replacing it with the placeholder "Automatic" entry.
    fn selected_model(&self) -> String {
        if !self.authoritative.get() {
            return self.choices.borrow().model.clone();
        }
        self.row_model()
    }

    fn row_model(&self) -> String {
        self.model_ids.borrow().get(self.model_row.selected() as usize).cloned().unwrap_or_default()
    }

    /// Until the model list is known, keep the saved level rather than
    /// discarding it because the placeholder list lacks it.
    fn selected_effort(&self) -> String {
        if !self.authoritative.get() {
            return self.choices.borrow().reasoning_effort.clone();
        }
        self.effort_ids.borrow().get(self.reasoning_row.selected() as usize).cloned().unwrap_or_default()
    }

    fn model_selected(&self) {
        let effort = self.selected_effort();
        self.choices.borrow_mut().model = self.row_model();
        self.refresh_reasoning(&effort, true);
        self.update_speed();
    }

    fn apply_status(&self, status: &Status) {
        let mut markup = glib::markup_escape_text(&status.message).to_string();
        if let Some(link) = &status.link {
            markup.push_str(&format!(
                " <a href=\"{}\">Open the ChatGPT sign-in page</a>.",
                glib::markup_escape_text(link)
            ));
        }
        self.account.set_subtitle(&markup);

        let state = status.state;
        let signed_in = state == AuthState::SignedIn;
        let missing = state == AuthState::Missing;
        self.signed_in.set(signed_in);
        self.login.set_visible(!signed_in && !missing);
        self.login.set_sensitive(state != AuthState::SigningIn && state != AuthState::Checking);
        self.cancel.set_visible(state == AuthState::SigningIn);
        self.logout.set_visible(signed_in);
        self.install.set_visible(missing || state == AuthState::Unsupported);
        self.model_row.set_sensitive(signed_in);
        self.reasoning_row.set_sensitive(signed_in && self.effort_ids.borrow().len() > 1);
    }

    fn apply_models(&self, list: &ModelList) {
        self.authoritative.set(list.authoritative);
        let configured = self.choices.borrow().model.clone();
        let mut ids = vec![String::new()];
        let mut labels = vec!["Automatic (Codex default)".to_owned()];
        let mut models = Vec::new();
        for model in &list.models {
            let Some(id) = model_id(model) else { continue };
            if ids.iter().any(|existing| existing == id) {
                continue;
            }
            ids.push(id.to_owned());
            labels.push(model.get("displayName").and_then(Value::as_str).unwrap_or(id).to_owned());
            models.push(model.clone());
        }
        let index = ids.iter().position(|id| *id == configured);
        let unavailable = list.authoritative && !configured.is_empty() && index.is_none();

        self.updating.set(true);
        set_combo_items(&self.model_row, &labels);
        self.model_row.set_selected(index.unwrap_or(0) as u32);
        self.updating.set(false);
        self.model_ids.replace(ids);
        self.models.replace(models);

        if unavailable {
            self.choices.borrow_mut().model.clear();
            self.model_row.set_subtitle(&warning_markup(&format!(
                "The saved model \"{configured}\" is no longer available. Automatic will be used."
            )));
        } else {
            self.model_row.set_subtitle("");
        }

        let effort = self.choices.borrow().reasoning_effort.clone();
        self.refresh_reasoning(&effort, list.authoritative);
        self.update_speed();
    }

    fn refresh_reasoning(&self, requested: &str, authoritative: bool) {
        let models = self.models.borrow();
        let efforts = supported_efforts(model_metadata(&models, &self.selected_model()));
        drop(models);
        let mut ids = vec![String::new()];
        let mut labels = vec!["Automatic (model default)".to_owned()];
        for (effort, _) in &efforts {
            ids.push(effort.clone());
            labels.push(reasoning_label(effort));
        }
        let index = ids.iter().position(|id| id == requested);
        let unsupported = authoritative && !requested.is_empty() && index.is_none();

        self.updating.set(true);
        set_combo_items(&self.reasoning_row, &labels);
        self.reasoning_row.set_selected(index.unwrap_or(0) as u32);
        self.updating.set(false);
        self.effort_ids.replace(ids);

        let warning = unsupported.then(|| {
            format!(
                "The thinking level \"{}\" is not supported by the selected model. Automatic will be used.",
                reasoning_label(requested)
            )
        });
        if unsupported {
            self.choices.borrow_mut().reasoning_effort.clear();
        }
        self.reasoning_warning.replace(warning);
        self.reasoning_row.set_sensitive(self.signed_in.get() && !efforts.is_empty());
        self.update_reasoning_subtitle();
    }

    /// Describe the selected level: the model default for Automatic, or the
    /// level's own description.
    fn update_reasoning_subtitle(&self) {
        if let Some(warning) = &*self.reasoning_warning.borrow() {
            self.reasoning_row.set_subtitle(&warning_markup(warning));
            return;
        }
        let models = self.models.borrow();
        let model = model_metadata(&models, &self.selected_model());
        let selected =
            self.effort_ids.borrow().get(self.reasoning_row.selected() as usize).cloned().unwrap_or_default();
        let subtitle = if selected.is_empty() {
            model
                .and_then(|m| m.get("defaultReasoningEffort"))
                .and_then(Value::as_str)
                .filter(|e| !e.is_empty())
                .map(|effort| format!("Uses the model default: {}.", reasoning_label(effort)))
        } else {
            supported_efforts(model).into_iter().find(|(e, _)| *e == selected).and_then(|(_, description)| description)
        };
        self.reasoning_row.set_subtitle(&glib::markup_escape_text(&subtitle.unwrap_or_default()));
    }

    fn update_speed(&self) {
        let supported = supports_fast(&self.models.borrow(), &self.selected_model());
        if !supported && speed_value(&self.speed_row) == "priority" {
            self.speed_row.set_selected(0);
        }
        self.speed_row.set_sensitive(supported);
        self.speed_row.set_subtitle(&if supported {
            glib::markup_escape_text(SPEED_DESCRIPTION).to_string()
        } else {
            warning_markup(NO_FAST_WARNING)
        });
    }
}
