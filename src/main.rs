mod app;
mod clipboard;
mod codex;
mod config;
mod icons;
mod markdown;
mod obfuscate;
mod options;
mod paths;
mod portal;
mod prompt;
mod providers;
mod runtime;
mod tray;
mod ui;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("writing_tools=info")).init();
    // Wayland's app ID; the portal and window rules match it.
    glib::set_prgname(Some(paths::APP_ID));
    glib::set_application_name("Writing Tools");

    let application = adw::Application::builder().application_id(paths::APP_ID).build();
    let state: Rc<RefCell<Option<Rc<app::App>>>> = Rc::default();
    application.connect_startup({
        let state = state.clone();
        move |application| {
            let css = gtk::CssProvider::new();
            css.load_from_string(include_str!("style.css"));
            gtk::style_context_add_provider_for_display(
                &gdk::Display::default().expect("a display"),
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            state.replace(Some(app::App::start(application)));
        }
    });
    // GApplication activates once at launch; later activations come from a
    // second launch, which opens Settings in the running instance instead.
    let launched = std::cell::Cell::new(false);
    application.connect_activate({
        let state = state.clone();
        move |_| {
            if launched.replace(true) {
                log::info!("Writing Tools is already running; opening Settings");
                if let Some(app) = state.borrow().as_ref().filter(|app| app.config.borrow().provider.is_some()) {
                    ui::settings::show(app, false);
                }
            }
        }
    });
    let code = application.run_with_args::<&str>(&[]);
    drop(state);
    code
}
