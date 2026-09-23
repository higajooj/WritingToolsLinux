//! The response window: the result as rendered Markdown, with follow-up chat.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::app::{App, show_message};
use crate::prompt::FOLLOWUP_SYSTEM_INSTRUCTION;
use crate::providers::{Message, Prompt, ProviderError};
use crate::{icons, markdown, runtime};

pub const DEFAULT_ZOOM: f64 = 1.2;
pub const MIN_ZOOM: f64 = 0.5;
pub const MAX_ZOOM: f64 = 3.0;
pub const ZOOM_STEP: f64 = 1.1;
const BASE_FONT_PX: f64 = 14.0;
/// One Ctrl+scroll gesture emits many zoom steps; write the config once it settles.
const ZOOM_SAVE_DELAY: Duration = Duration::from_millis(500);
const FOLLOWUP_PLACEHOLDER: &str = "Ask a follow-up question...";
const THINKING_DOTS: [&str; 4] = ["", ".", "..", "..."];

#[derive(Clone, Copy)]
enum Zoom {
    In,
    Out,
    Reset,
}

/// Clamp a stored zoom factor, falling back to the default when invalid.
pub fn sanitize_zoom(value: Option<f64>) -> f64 {
    match value {
        Some(zoom) if zoom.is_finite() => zoom.clamp(MIN_ZOOM, MAX_ZOOM),
        _ => DEFAULT_ZOOM,
    }
}

fn next_zoom(current: f64, action: Zoom) -> f64 {
    match action {
        Zoom::In => (current * ZOOM_STEP).min(MAX_ZOOM),
        Zoom::Out => (current / ZOOM_STEP).max(MIN_ZOOM),
        Zoom::Reset => DEFAULT_ZOOM,
    }
}

struct Response {
    app: Rc<App>,
    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    chat: gtk::Box,
    scroller: gtk::ScrolledWindow,
    entry: gtk::Entry,
    thinking: gtk::Label,
    css: gtk::CssProvider,
    css_name: String,
    zoom: Cell<f64>,
    history: RefCell<Vec<Message>>,
    thinking_timer: RefCell<Option<glib::SourceId>>,
    thinking_state: Cell<usize>,
    request: RefCell<Option<tokio::task::AbortHandle>>,
    /// Keep scrolling to the end while a new message is being laid out.
    follow_end: RefCell<Option<glib::SourceId>>,
}

/// Open a window for `option`, show the user's text, and run the request.
pub fn open(app: &Rc<App>, option: &str, selected_text: &str, prompt: String, system: String) {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let css_name = format!("response-chat-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));

    let window = adw::ApplicationWindow::builder()
        .application(&app.gtk)
        .title(format!("{option} Result"))
        .default_width(600)
        .default_height(600)
        .width_request(400)
        .height_request(300)
        .build();

    let header = adw::HeaderBar::new();
    let zoom_in = icons::button("plus", 16, "Zoom In");
    let zoom_out = icons::button("minus", 16, "Zoom Out");
    let zoom_reset = icons::button("reset", 16, "Reset Zoom");
    header.pack_end(&zoom_reset);
    header.pack_end(&zoom_out);
    header.pack_end(&zoom_in);

    let thinking = gtk::Label::new(Some("Thinking"));
    thinking.add_css_class("title-3");
    thinking.add_css_class("dim-label");
    thinking.set_margin_top(12);

    let chat = gtk::Box::new(gtk::Orientation::Vertical, 8);
    chat.set_widget_name(&css_name);
    chat.set_margin_top(15);
    chat.set_margin_bottom(15);
    chat.set_margin_start(15);
    chat.set_margin_end(15);
    let scroller =
        gtk::ScrolledWindow::builder().child(&chat).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();

    let entry = gtk::Entry::builder().placeholder_text(FOLLOWUP_PLACEHOLDER).hexpand(true).sensitive(false).build();
    let send = icons::button("send", 16, "Send");
    send.add_css_class("suggested-action");
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bottom.set_margin_start(15);
    bottom.set_margin_end(15);
    bottom.set_margin_bottom(15);
    bottom.append(&entry);
    bottom.append(&send);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&thinking);
    body.append(&scroller);
    body.append(&bottom);
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&body));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&toasts));
    window.set_content(Some(&toolbar));

    let css = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("a display"),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let response = Rc::new(Response {
        app: app.clone(),
        window: window.clone(),
        toasts,
        chat,
        scroller: scroller.clone(),
        entry: entry.clone(),
        thinking,
        css,
        css_name,
        zoom: Cell::new(sanitize_zoom(app.config.borrow().response_window_zoom)),
        history: RefCell::new(vec![Message::user(prompt.clone())]),
        thinking_timer: RefCell::new(None),
        thinking_state: Cell::new(0),
        request: RefCell::new(None),
        follow_end: RefCell::new(None),
    });
    let weak = Rc::downgrade(&response);
    scroller.vadjustment().connect_changed(move |adjustment| {
        // "changed" is emitted while the viewport is being allocated, when a
        // new value would be ignored; scroll once that pass is over.
        if weak.upgrade().is_some_and(|response| response.follow_end.borrow().is_some()) {
            let adjustment = adjustment.clone();
            glib::idle_add_local_once(move || adjustment.set_value(adjustment.upper() - adjustment.page_size()));
        }
    });
    response.apply_zoom();

    for (button, action) in [(&zoom_in, Zoom::In), (&zoom_out, Zoom::Out), (&zoom_reset, Zoom::Reset)] {
        let weak = Rc::downgrade(&response);
        button.connect_clicked(move |_| {
            if let Some(response) = weak.upgrade() {
                response.zoom(action);
            }
        });
    }
    response.add_zoom_controls(&scroller);

    let weak = Rc::downgrade(&response);
    let send_message = move || {
        if let Some(response) = weak.upgrade() {
            response.send_followup();
        }
    };
    entry.connect_activate({
        let send_message = send_message.clone();
        move |_| send_message()
    });
    send.connect_clicked(move |_| send_message());

    // This handler owns the response until the window closes. Releasing it
    // then breaks the response -> window -> handler cycle.
    window.connect_close_request({
        let owner = RefCell::new(Some(response.clone()));
        move |_| {
            let Some(response) = owner.take() else { return glib::Propagation::Proceed };
            if let Some(request) = response.request.take() {
                request.abort();
            }
            response.stop_thinking();
            response.app.flush_config();
            gtk::style_context_remove_provider_for_display(&WidgetExt::display(&response.window), &response.css);
            glib::Propagation::Proceed
        }
    });

    response.add_user_message(selected_text);
    response.start_thinking(true);
    window.present();

    let weak = Rc::downgrade(&response);
    response.run(system, Prompt::Single(prompt), move |result| {
        let Some(response) = weak.upgrade() else { return };
        response.stop_thinking();
        match result {
            Ok(text) if !text.trim().is_empty() => {
                response.history.borrow_mut().push(Message::assistant(text.clone()));
                response.add_assistant_message(&text);
            }
            Ok(_) => response.add_note("The AI returned an empty response. Please try again."),
            Err(error) => {
                show_message(Some(&response.window), &error.title, &error.message);
                response.add_note(&error.message);
            }
        }
    });
}

impl Response {
    /// Run a request, delivering the result on the GTK thread. Closing the
    /// window aborts it.
    fn run(&self, system: String, prompt: Prompt, done: impl FnOnce(Result<String, ProviderError>) + 'static) {
        let Some(provider) = self.app.provider() else {
            done(Err(ProviderError::new("Error", "Choose an AI provider in Settings first.")));
            return;
        };
        let job = runtime::spawn(async move { provider.respond(&system, prompt).await });
        self.request.replace(Some(job.abort_handle()));
        glib::spawn_future_local(async move {
            if let Ok(result) = job.await {
                done(result);
            }
        });
    }

    fn send_followup(self: &Rc<Self>) {
        let question = self.entry.text().trim().to_owned();
        if question.is_empty() || !self.entry.is_sensitive() {
            return;
        }
        self.entry.set_text("");
        self.add_user_message(&question);
        self.history.borrow_mut().push(Message::user(question));
        self.start_thinking(false);

        let history = self.history.borrow().clone();
        let weak = Rc::downgrade(self);
        self.run(FOLLOWUP_SYSTEM_INSTRUCTION.to_owned(), Prompt::Chat(history), move |result| {
            let Some(response) = weak.upgrade() else { return };
            response.stop_thinking();
            match result {
                Ok(text) if !text.trim().is_empty() => {
                    response.history.borrow_mut().push(Message::assistant(text.clone()));
                    response.add_assistant_message(&text);
                }
                Ok(_) => response.add_note("The AI returned an empty response. Please try again."),
                Err(error) => {
                    show_message(Some(&response.window), &error.title, &error.message);
                    response.add_note("Sorry, an error occurred while processing your question.");
                }
            }
        });
    }

    // ------------------------------------------------------------ messages

    fn add_user_message(self: &Rc<Self>, text: &str) {
        let label = gtk::Label::new(Some(text));
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_xalign(0.0);
        label.set_selectable(true);
        label.set_can_focus(false);
        label.add_css_class("user-message");
        self.append(&label);
    }

    fn add_assistant_message(self: &Rc<Self>, text: &str) {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card.add_css_class("card");
        card.add_css_class("assistant-message");
        card.append(&markdown::render(text));

        let copy = icons::button("copy", 16, "Copy response");
        copy.add_css_class("flat");
        copy.set_halign(gtk::Align::End);
        copy.set_cursor_from_name(Some("pointer"));
        let text = text.to_owned();
        let toasts = self.toasts.clone();
        copy.connect_clicked(move |button| {
            button.clipboard().set_text(&text);
            toasts.add_toast(adw::Toast::builder().title("Copied to clipboard").timeout(2).build());
        });
        card.append(&copy);
        self.append(&card);
    }

    fn add_note(self: &Rc<Self>, text: &str) {
        let label = gtk::Label::new(Some(text));
        label.set_wrap(true);
        label.set_xalign(0.0);
        label.add_css_class("error");
        self.append(&label);
    }

    fn append(self: &Rc<Self>, widget: &impl IsA<gtk::Widget>) {
        self.chat.append(widget);
        // Wrapped labels settle their height over a few layout passes; follow
        // the growing end of the chat until they have.
        if let Some(timer) = self.follow_end.take() {
            timer.remove();
        }
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(Duration::from_millis(500), move || {
            if let Some(response) = weak.upgrade() {
                response.follow_end.replace(None);
            }
        });
        self.follow_end.replace(Some(timer));
        let adjustment = self.scroller.vadjustment();
        adjustment.set_value(adjustment.upper() - adjustment.page_size());
    }

    // ------------------------------------------------------------ thinking

    fn start_thinking(self: &Rc<Self>, initial: bool) {
        self.thinking_state.set(0);
        self.thinking.set_visible(initial);
        self.entry.set_sensitive(false);
        self.update_thinking();
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local(Duration::from_millis(300), move || {
            let Some(response) = weak.upgrade() else { return glib::ControlFlow::Break };
            response.thinking_state.set((response.thinking_state.get() + 1) % THINKING_DOTS.len());
            response.update_thinking();
            glib::ControlFlow::Continue
        });
        if let Some(old) = self.thinking_timer.replace(Some(id)) {
            old.remove();
        }
    }

    fn update_thinking(&self) {
        let text = format!("Thinking{}", THINKING_DOTS[self.thinking_state.get()]);
        if self.thinking.is_visible() {
            self.thinking.set_text(&text);
        } else {
            self.entry.set_placeholder_text(Some(&text));
        }
    }

    fn stop_thinking(&self) {
        if let Some(timer) = self.thinking_timer.take() {
            timer.remove();
        }
        self.thinking.set_visible(false);
        self.entry.set_placeholder_text(Some(FOLLOWUP_PLACEHOLDER));
        self.entry.set_sensitive(true);
        self.entry.grab_focus();
    }

    // ------------------------------------------------------------ zoom

    fn apply_zoom(&self) {
        let size = (BASE_FONT_PX * self.zoom.get()).round();
        self.css.load_from_string(&format!("#{} {{ font-size: {size}px; }}", self.css_name));
    }

    fn zoom(self: &Rc<Self>, action: Zoom) {
        let zoom = next_zoom(self.zoom.get(), action);
        if zoom == self.zoom.get() {
            return;
        }
        self.zoom.set(zoom);
        self.apply_zoom();
        // New windows read the in-memory config; the disk write is debounced.
        self.app.config.borrow_mut().response_window_zoom = Some(zoom);
        self.app.save_config_later(ZOOM_SAVE_DELAY);
    }

    /// Ctrl+scroll and Ctrl+plus/minus/0.
    fn add_zoom_controls(self: &Rc<Self>, scroller: &gtk::ScrolledWindow) {
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        scroll.connect_scroll(move |controller, _, dy| {
            let Some(response) = weak.upgrade() else { return glib::Propagation::Proceed };
            if !controller.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) || dy == 0.0 {
                return glib::Propagation::Proceed;
            }
            response.zoom(if dy < 0.0 { Zoom::In } else { Zoom::Out });
            glib::Propagation::Stop
        });
        scroller.add_controller(scroll);

        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_scope(gtk::ShortcutScope::Managed);
        for (trigger, action) in [
            ("<Control>plus|<Control>equal|<Control>KP_Add", Zoom::In),
            ("<Control>minus|<Control>KP_Subtract", Zoom::Out),
            ("<Control>0|<Control>KP_0", Zoom::Reset),
        ] {
            let weak = Rc::downgrade(self);
            let callback = gtk::CallbackAction::new(move |_, _| {
                if let Some(response) = weak.upgrade() {
                    response.zoom(action);
                }
                glib::Propagation::Stop
            });
            shortcuts.add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string(trigger), Some(callback)));
        }
        self.window.add_controller(shortcuts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_bounds() {
        assert_eq!(sanitize_zoom(None), DEFAULT_ZOOM);
        assert_eq!(sanitize_zoom(Some(f64::NAN)), DEFAULT_ZOOM);
        assert_eq!(sanitize_zoom(Some(10.0)), MAX_ZOOM);
        assert_eq!(sanitize_zoom(Some(0.1)), MIN_ZOOM);
        assert_eq!(sanitize_zoom(Some(1.5)), 1.5);
        assert_eq!(next_zoom(MAX_ZOOM, Zoom::In), MAX_ZOOM);
        assert_eq!(next_zoom(MIN_ZOOM, Zoom::Out), MIN_ZOOM);
        assert!((next_zoom(1.0, Zoom::In) - 1.1).abs() < 1e-9);
        assert_eq!(next_zoom(2.0, Zoom::Reset), DEFAULT_ZOOM);
    }
}
