//! Application state and the flow from hotkey to result.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use gtk::glib;

use crate::config::{self, Config};
use crate::options::{self, Options};
use crate::portal::{self, PortalEvent, ShortcutPortal};
use crate::prompt;
use crate::providers::subscription::{Choices, Subscription, SubscriptionProvider};
use crate::providers::{self, Prompt, Provider, ProviderError, ProviderKind};
use crate::tray::{Tray, TrayAction};
use crate::{clipboard, codex, paths, runtime, ui};

const TRIGGER_WINDOW: Duration = Duration::from_millis(1500);
const MAX_TRIGGERS: usize = 3;
const READY_INDICATOR: Duration = Duration::from_secs(5);
const INCOMPATIBLE: &str = "ERROR_TEXT_INCOMPATIBLE_WITH_REQUEST";

#[derive(Default)]
struct ShortcutStatus {
    registered: bool,
    portal_error: Option<String>,
    entry_error: String,
}

pub struct App {
    pub gtk: adw::Application,
    _hold: gtk::gio::ApplicationHoldGuard,
    pub config: RefCell<Config>,
    pub options: RefCell<Options>,
    provider: RefCell<Option<Arc<dyn Provider>>>,
    pub subscription: Arc<Subscription>,
    paused: Cell<bool>,
    triggers: RefCell<Vec<Instant>>,
    popup: RefCell<Option<gtk::Window>>,
    /// The clipboard text captured when the popup opened.
    captured: RefCell<Option<Shared<BoxFuture<'static, String>>>>,
    /// The pending clipboard-mode request; a new hotkey press cancels it.
    clipboard_job: RefCell<Option<tokio::task::AbortHandle>>,
    portal: RefCell<ShortcutPortal>,
    portal_events: async_channel::Sender<PortalEvent>,
    shortcut: RefCell<ShortcutStatus>,
    tray: RefCell<Option<Tray>>,
    ready_timer: RefCell<Option<glib::SourceId>>,
    config_save_timer: RefCell<Option<glib::SourceId>>,
}

impl App {
    pub fn start(gtk_app: &adw::Application) -> Rc<Self> {
        let (config, load_failed) = match config::load() {
            Ok(config) => (config, false),
            Err(e) => {
                // Refuse to start over a config we cannot read: onboarding would
                // otherwise overwrite the user's settings.
                let message = format!("Could not read {}:\n\n{e}", paths::config_path().display());
                log::error!("{message}");
                let app = gtk_app.clone();
                show_message(None::<&gtk::Widget>, "Configuration Error", &message).connect_closed(move |_| app.quit());
                (None, true)
            }
        };
        let first_run = config.is_none() && !load_failed;
        let options = options::load().unwrap_or_else(|e| {
            log::error!("Could not load {}: {e}", paths::options_path().display());
            show_message(
                None::<&gtk::Widget>,
                "Error",
                &format!("Could not load the buttons ({e}). Using the defaults."),
            );
            options::defaults()
        });

        let (portal_events, portal_receiver) = async_channel::unbounded();
        let app = Rc::new(Self {
            gtk: gtk_app.clone(),
            _hold: gtk_app.hold(),
            config: RefCell::new(config.unwrap_or_default()),
            options: RefCell::new(options),
            provider: RefCell::new(None),
            subscription: Subscription::new(codex::CodexClient::new(paths::codex_data_root())),
            paused: Cell::new(false),
            triggers: RefCell::new(Vec::new()),
            popup: RefCell::new(None),
            captured: RefCell::new(None),
            clipboard_job: RefCell::new(None),
            portal: RefCell::new(ShortcutPortal::default()),
            portal_events,
            shortcut: RefCell::new(ShortcutStatus::default()),
            tray: RefCell::new(None),
            ready_timer: RefCell::new(None),
            config_save_timer: RefCell::new(None),
        });

        let weak = Rc::downgrade(&app);
        glib::spawn_future_local(async move {
            while let Ok(event) = portal_receiver.recv().await {
                let Some(app) = weak.upgrade() else { break };
                app.on_portal_event(event);
            }
        });

        let interrupted = runtime::spawn(async { tokio::signal::ctrl_c().await });
        let weak = Rc::downgrade(&app);
        glib::spawn_future_local(async move {
            if let Ok(Ok(())) = interrupted.await {
                log::info!("Received SIGINT. Exiting...");
                if let Some(app) = weak.upgrade() {
                    app.exit();
                }
            }
        });

        if load_failed {
            // The error dialog quits the app when it closes.
        } else if first_run {
            log::debug!("No config found, showing onboarding");
            ui::onboarding::show(&app);
        } else {
            app.activate_provider();
            app.create_tray();
            app.register_hotkey();
        }
        app
    }

    // ---------------------------------------------------------------- config

    pub fn save_config(&self) {
        if let Some(timer) = self.config_save_timer.take() {
            timer.remove();
        }
        if let Err(e) = config::save(&self.config.borrow()) {
            log::error!("Could not save {}: {e}", paths::config_path().display());
            show_message(None::<&gtk::Widget>, "Error", &format!("Could not save the settings: {e}"));
        }
    }

    /// Save the config after `delay`, coalescing repeated changes (zoom gestures).
    pub fn save_config_later(self: &Rc<Self>, delay: Duration) {
        if let Some(timer) = self.config_save_timer.take() {
            timer.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(delay, move || {
            if let Some(app) = weak.upgrade() {
                app.config_save_timer.replace(None);
                app.save_config();
            }
        });
        self.config_save_timer.replace(Some(id));
    }

    /// Flush a pending delayed save.
    pub fn flush_config(&self) {
        if self.config_save_timer.borrow().is_some() {
            self.save_config();
        }
    }

    pub fn current_kind(&self) -> ProviderKind {
        self.config.borrow().provider.as_deref().and_then(ProviderKind::from_name).unwrap_or(ProviderKind::Gemini)
    }

    /// Build the request provider from the saved settings.
    pub fn activate_provider(&self) {
        let kind = self.current_kind();
        let provider_config = self.config.borrow().provider_config(kind.name());
        let provider: Arc<dyn Provider> = match providers::build(kind, &provider_config) {
            Some(provider) => provider,
            None => {
                self.subscription.set_choices(Choices::from_config(&provider_config));
                Arc::new(SubscriptionProvider(self.subscription.clone()))
            }
        };
        self.provider.replace(Some(provider));
    }

    // ---------------------------------------------------------------- shortcut

    pub fn register_hotkey(&self) {
        self.shortcut.borrow_mut().entry_error = portal::ensure_desktop_entry();
        let shortcut = self.config.borrow().shortcut().to_owned();
        if self.shortcut.borrow().registered && self.portal.borrow().trigger() == Some(shortcut.as_str()) {
            return;
        }
        {
            let mut status = self.shortcut.borrow_mut();
            status.registered = false;
            status.portal_error = None;
        }
        self.portal.borrow_mut().register(&shortcut, self.portal_events.clone());
    }

    fn on_portal_event(self: &Rc<Self>, event: PortalEvent) {
        match event {
            PortalEvent::Registered(generation, _) if !self.portal.borrow().is_current(generation) => {}
            PortalEvent::Registered(_, result) => {
                let mut status = self.shortcut.borrow_mut();
                status.registered = result.is_ok();
                status.portal_error = result.err();
                drop(status);
                if !self.shortcut.borrow().registered {
                    log::warn!("Global shortcut not registered. {}", self.diagnostics());
                }
            }
            PortalEvent::Activated(id) if id == portal::GLOBAL_ID => {
                if self.paused.get() {
                    log::debug!("Paused; ignoring shortcut \"{id}\"");
                } else {
                    self.on_hotkey();
                }
            }
            PortalEvent::Activated(_) => {}
        }
    }

    /// Setup problems to show alongside a clipboard error.
    pub fn diagnostics(&self) -> String {
        let status = self.shortcut.borrow();
        let mut problems = Vec::new();
        if !clipboard::available() {
            problems.push("Install wl-clipboard (wl-copy and wl-paste) for clipboard workflows.".to_owned());
        }
        if !status.entry_error.is_empty() {
            problems.push(status.entry_error.clone());
        }
        match &status.portal_error {
            Some(error) => problems.push(error.clone()),
            None if status.registered => {
                let hint = portal::compositor_bind_hint(self.config.borrow().shortcut());
                if !hint.is_empty() {
                    problems.push(hint);
                }
            }
            None => {}
        }
        problems.join(" ")
    }

    /// Record a trigger; true when the hotkey fired too often in a short window.
    fn trigger_spam(&self) -> bool {
        let now = Instant::now();
        let mut triggers = self.triggers.borrow_mut();
        triggers.push(now);
        triggers.retain(|t| now.duration_since(*t) <= TRIGGER_WINDOW);
        triggers.len() >= MAX_TRIGGERS
    }

    fn on_hotkey(self: &Rc<Self>) {
        log::debug!("Hotkey pressed");
        if self.trigger_spam() {
            log::warn!("Hotkey spam detected - quitting application");
            self.exit();
            return;
        }
        if let Some(job) = self.clipboard_job.take() {
            log::debug!("Cancelling the current request");
            job.abort();
        }
        self.show_popup();
    }

    /// Show the popup at once and read the clipboard in parallel, so a slow
    /// clipboard never delays it. Wayland cannot read another application's
    /// selection, so users copy text before invoking Writing Tools.
    fn show_popup(self: &Rc<Self>) {
        let capture = runtime::spawn_blocking(clipboard::read).map(|text| text.unwrap_or_default()).boxed().shared();
        self.captured.replace(Some(capture));

        if let Some(stale) = self.popup.take() {
            stale.close();
        }
        let popup = ui::popup::show(self);
        self.popup.replace(Some(popup));
    }

    pub fn forget_popup(&self, popup: &gtk::Window) {
        let mut current = self.popup.borrow_mut();
        if current.as_ref() == Some(popup) {
            *current = None;
        }
    }

    // ---------------------------------------------------------------- requests

    pub fn provider(&self) -> Option<Arc<dyn Provider>> {
        self.provider.borrow().clone()
    }

    /// Run a button (or `Custom`) on the captured text, with optional
    /// instructions for this invocation.
    pub fn process_option(self: &Rc<Self>, name: String, extra: Option<String>) {
        log::debug!("Processing option: {name}");
        let capture = self.captured.borrow().clone();
        let app = self.clone();
        glib::spawn_future_local(async move {
            let text = match capture {
                Some(capture) => capture.await,
                None => String::new(),
            };
            if text.trim().is_empty() {
                let mut message = "Copy the text you want to use before invoking Writing Tools.".to_owned();
                let detail = app.diagnostics();
                if !detail.is_empty() {
                    message.push_str("\n\n");
                    message.push_str(&detail);
                }
                show_message(None::<&gtk::Widget>, "Error", &message);
                return;
            }
            let Some(option) = app.options.borrow().get(&name).cloned() else {
                show_message(None::<&gtk::Widget>, "Error", &format!("The button \"{name}\" no longer exists."));
                return;
            };
            let Some(provider) = app.provider() else {
                show_message(None::<&gtk::Widget>, "Error", "Choose an AI provider in Settings first.");
                return;
            };
            let system = prompt::build_system_instruction(&name, &option, extra.as_deref());
            let user_prompt = prompt::build_option_prompt(&name, &option, &text, extra.as_deref());

            if option.open_in_window {
                ui::response::open(&app, &name, &text, user_prompt, system);
                return;
            }

            let job = runtime::spawn(async move { provider.respond(&system, Prompt::Single(user_prompt)).await });
            app.clipboard_job.replace(Some(job.abort_handle()));
            let result = job.await;
            match result {
                // Cancelled by a new hotkey press: drop it silently.
                Err(_) => {}
                Ok(Err(error)) => app.show_provider_error(&error),
                Ok(Ok(response)) => app.handle_output(response).await,
            }
        });
    }

    pub fn show_provider_error(&self, error: &ProviderError) {
        log::error!("{}: {}", error.title, error.message);
        show_message(None::<&gtk::Widget>, &error.title, &error.message);
    }

    /// Copy a complete response and signal when it is ready to paste.
    async fn handle_output(self: &Rc<Self>, text: String) {
        if text.trim().is_empty() {
            show_message(
                None::<&gtk::Widget>,
                "Empty Response",
                "The AI returned an empty response, so nothing was copied. Please try again.",
            );
            return;
        }
        if text.trim() == INCOMPATIBLE {
            show_message(None::<&gtk::Widget>, "Error", "The text is incompatible with the requested change.");
            return;
        }
        let text = text.trim_end_matches('\n').to_owned();
        let copied = runtime::spawn_blocking(move || clipboard::write(&text)).await.unwrap_or(false);
        if !copied {
            show_message(
                None::<&gtk::Widget>,
                "Clipboard Error",
                "Could not copy the response. Check that wl-clipboard is installed and the Wayland clipboard is available.",
            );
            return;
        }
        self.show_clipboard_ready();
    }

    /// Switch the tray to the "ready to paste" icon; each copy restarts it.
    fn show_clipboard_ready(self: &Rc<Self>) {
        let Some(tray) = &*self.tray.borrow() else { return };
        tray.set_ready(true);
        if let Some(timer) = self.ready_timer.take() {
            timer.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(READY_INDICATOR, move || {
            if let Some(app) = weak.upgrade() {
                app.ready_timer.replace(None);
                if let Some(tray) = &*app.tray.borrow() {
                    tray.set_ready(false);
                }
            }
        });
        self.ready_timer.replace(Some(id));
    }

    // ---------------------------------------------------------------- tray

    pub fn create_tray(self: &Rc<Self>) {
        if self.tray.borrow().is_some() {
            return;
        }
        let (sender, receiver) = async_channel::unbounded();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let tray = Tray::spawn(sender).await;
            if let Some(app) = weak.upgrade() {
                if tray.is_some() {
                    log::debug!("Tray icon displayed");
                }
                app.tray.replace(tray);
            }
            while let Ok(action) = receiver.recv().await {
                let Some(app) = weak.upgrade() else { break };
                match action {
                    TrayAction::Settings => ui::settings::show(&app, false),
                    TrayAction::TogglePause => {
                        app.paused.set(!app.paused.get());
                        log::debug!("{}", if app.paused.get() { "App is paused" } else { "App is resumed" });
                        if let Some(tray) = &*app.tray.borrow() {
                            tray.set_paused(app.paused.get());
                        }
                    }
                    TrayAction::About => ui::about::show(),
                    TrayAction::Exit => app.exit(),
                }
            }
        });
    }

    // ---------------------------------------------------------------- exit

    pub fn exit(&self) {
        log::debug!("Stopping the listener");
        self.portal.borrow_mut().stop();
        self.flush_config();
        let subscription = self.subscription.clone();
        runtime::runtime().block_on(async move {
            let _ = tokio::time::timeout(Duration::from_secs(3), subscription.shutdown()).await;
        });
        if let Some(tray) = self.tray.take() {
            tray.shutdown();
        }
        log::debug!("Exiting application");
        self.gtk.quit();
    }
}

/// A simple message dialog. Without a parent it opens as its own window.
pub fn show_message(parent: Option<&impl IsA<gtk::Widget>>, title: &str, message: &str) -> adw::AlertDialog {
    let dialog = adw::AlertDialog::new(Some(title), Some(message));
    dialog.add_response("ok", "OK");
    dialog.set_default_response(Some("ok"));
    dialog.present(parent);
    dialog
}
