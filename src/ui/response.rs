//! The response window: the result as rendered Markdown, with follow-up chat.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::app::{App, show_message};
use crate::prompt::FOLLOWUP_SYSTEM_INSTRUCTION;
use crate::providers::{Cost, Message, Prompt, Provider, ProviderError, Reply, Usage};
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

/// How long a request took: tenths of a second when `precise`, as under a
/// reply, or whole seconds for the running counter.
fn format_elapsed(elapsed: Duration, precise: bool) -> String {
    let seconds = elapsed.as_secs();
    match (seconds / 60, precise) {
        (0, true) => format!("{:.1}s", elapsed.as_secs_f64()),
        (0, false) => format!("{seconds}s"),
        (minutes, _) => format!("{minutes}m {:02}s", seconds % 60),
    }
}

fn format_cost(dollars: f64) -> String {
    match dollars {
        d if d <= 0.0 => "$0.00".to_owned(),
        d if d < 0.0001 => "<$0.0001".to_owned(),
        d if d < 0.01 => format!("${d:.4}"),
        d if d < 1.0 => format!("${d:.3}"),
        d => format!("${d:.2}"),
    }
}

/// A token count in little room: 980, 4.2k, 1.3M.
fn format_tokens(tokens: u64) -> String {
    match tokens {
        0..1_000 => tokens.to_string(),
        1_000..1_000_000 => format!("{:.1}k", tokens as f64 / 1e3),
        _ => format!("{:.1}M", tokens as f64 / 1e6),
    }
}

/// A number with thousands separators.
fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// The total of a chat's requests as a label and its tooltip: the cost when
/// every request has one, otherwise the tokens.
fn format_usage(requests: &[Usage]) -> Option<(String, String)> {
    if requests.is_empty() {
        return None;
    }
    let input: u64 = requests.iter().map(|usage| usage.input_tokens).sum();
    let output: u64 = requests.iter().map(|usage| usage.output_tokens).sum();
    let cost = requests.iter().try_fold(0.0, |total, usage| match usage.cost? {
        Cost::Exact(cost) | Cost::Estimated(cost) => Some(total + cost),
    });
    let estimated = requests.iter().any(|usage| matches!(usage.cost, Some(Cost::Estimated(_))));
    let label = match cost {
        Some(cost) if estimated => format!("~{}", format_cost(cost)),
        Some(cost) => format_cost(cost),
        None => format!("{} in · {} out", format_tokens(input), format_tokens(output)),
    };
    let count = requests.len();
    let mut tooltip = format!(
        "This chat: {count} request{}\n{} input tokens · {} output tokens",
        if count == 1 { "" } else { "s" },
        grouped(input),
        grouped(output)
    );
    if cost.is_some() && estimated {
        tooltip.push_str("\nCost estimated from OpenRouter list prices");
    }
    Some((label, tooltip))
}

fn user_label(text: &str, css_class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label.set_selectable(true);
    label.set_can_focus(false);
    label.add_css_class("user-message");
    label.add_css_class(css_class);
    label
}

struct Response {
    app: Rc<App>,
    /// The button's provider, which also answers the follow-ups.
    provider: Arc<dyn Provider>,
    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    chat: gtk::Box,
    scroller: gtk::ScrolledWindow,
    entry: gtk::Entry,
    thinking: gtk::Label,
    /// What the chat has cost so far, above the follow-up entry.
    cost: gtk::Label,
    /// The usage of each request that reported one.
    usage: RefCell<Vec<Usage>>,
    css: gtk::CssProvider,
    css_name: String,
    zoom: Cell<f64>,
    history: RefCell<Vec<Message>>,
    thinking_timer: RefCell<Option<glib::SourceId>>,
    thinking_state: Cell<usize>,
    /// When the current (or last) request was sent.
    started: Cell<Instant>,
    request: RefCell<Option<tokio::task::AbortHandle>>,
    /// Keep scrolling to the end while a new message is being laid out.
    follow_end: RefCell<Option<glib::SourceId>>,
}

/// What to ask, and the button's provider to ask it of.
pub struct Request {
    pub provider: Arc<dyn Provider>,
    /// The provider and model, shown in the header.
    pub label: String,
    pub system: String,
    pub prompt: String,
}

/// Open a window for `option`, show the user's instructions and text, and run
/// the request. The first reply is saved to the history entry `record`.
pub fn open(app: &Rc<App>, option: &str, selected_text: &str, extra: Option<&str>, request: Request, record: u64) {
    let Request { provider, label, system, prompt } = request;
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let css_name = format!("response-chat-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));

    let window = adw::ApplicationWindow::builder()
        .application(&app.gtk)
        .title(format!("{option} Result"))
        .default_width(1000)
        .default_height(700)
        .width_request(400)
        .height_request(300)
        .build();

    let header = adw::HeaderBar::new();
    let provider_label = gtk::Label::builder().label(label).ellipsize(gtk::pango::EllipsizeMode::End).build();
    provider_label.add_css_class("dim-label");
    provider_label.add_css_class("caption");
    provider_label.set_margin_start(6);
    header.pack_start(&provider_label);
    let zoom_in = icons::button("plus", 16, "Zoom In");
    let zoom_out = icons::button("minus", 16, "Zoom Out");
    let zoom_reset = icons::button("reset", 16, "Reset Zoom");
    header.pack_end(&zoom_reset);
    header.pack_end(&zoom_out);
    header.pack_end(&zoom_in);

    let thinking = gtk::Label::new(Some("Thinking"));
    thinking.add_css_class("title-3");
    thinking.add_css_class("dim-label");
    thinking.set_justify(gtk::Justification::Center);
    thinking.set_margin_top(12);

    let chat = gtk::Box::new(gtk::Orientation::Vertical, 8);
    chat.set_widget_name(&css_name);
    chat.set_margin_top(15);
    chat.set_margin_bottom(15);
    chat.set_margin_start(15);
    chat.set_margin_end(15);
    let scroller =
        gtk::ScrolledWindow::builder().child(&chat).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();

    let cost = gtk::Label::builder().xalign(0.0).visible(false).build();
    cost.add_css_class("dim-label");
    cost.add_css_class("caption");
    cost.set_margin_start(21);
    cost.set_margin_bottom(6);

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
    body.append(&cost);
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
        provider,
        window: window.clone(),
        toasts,
        chat,
        scroller: scroller.clone(),
        entry: entry.clone(),
        thinking,
        cost,
        usage: RefCell::new(Vec::new()),
        css,
        css_name,
        zoom: Cell::new(sanitize_zoom(app.config.borrow().response_window_zoom)),
        history: RefCell::new(vec![Message::user(prompt.clone())]),
        thinking_timer: RefCell::new(None),
        thinking_state: Cell::new(0),
        started: Cell::new(Instant::now()),
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
    response.add_copy_shortcut();

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

    if let Some(extra) = extra.map(str::trim).filter(|e| !e.is_empty()) {
        response.add_instruction_message(extra);
    }
    response.add_user_message(selected_text);
    response.start_thinking(true);
    window.present();

    let weak = Rc::downgrade(&response);
    response.run(system, Prompt::Single(prompt), move |result| {
        let Some(response) = weak.upgrade() else { return };
        response.stop_thinking();
        match response.count_usage(result) {
            Ok(text) if !text.trim().is_empty() => {
                response.history.borrow_mut().push(Message::assistant(text.clone()));
                response.app.record_response(record, &text);
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
    fn run(&self, system: String, prompt: Prompt, done: impl FnOnce(Result<Reply, ProviderError>) + 'static) {
        let provider = self.provider.clone();
        let job = runtime::spawn(async move { provider.respond(&system, prompt).await });
        self.request.replace(Some(job.abort_handle()));
        glib::spawn_future_local(async move {
            if let Ok(result) = job.await {
                done(result);
            }
        });
    }

    /// Add a reply's usage to the chat's total, and hand back its text.
    fn count_usage(&self, result: Result<Reply, ProviderError>) -> Result<String, ProviderError> {
        let reply = result?;
        if let Some(usage) = reply.usage {
            self.usage.borrow_mut().push(usage);
            if let Some((label, tooltip)) = format_usage(&self.usage.borrow()) {
                self.cost.set_label(&label);
                self.cost.set_tooltip_text(Some(&tooltip));
                self.cost.set_visible(true);
            }
        }
        Ok(reply.text)
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
            match response.count_usage(result) {
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
        self.append(&user_label(text, "user-message"));
    }

    /// The extra instructions typed in the popup, shown above the text.
    fn add_instruction_message(self: &Rc<Self>, text: &str) {
        self.append(&user_label(text, "instruction-message"));
    }

    fn add_assistant_message(self: &Rc<Self>, text: &str) {
        let message = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card.add_css_class("card");
        card.add_css_class("assistant-message");
        card.append(&markdown::render(text));
        message.append(&card);

        let took = gtk::Label::new(Some(&format_elapsed(self.started.get().elapsed(), true)));
        took.set_tooltip_text(Some("Time taken for this response"));
        took.set_xalign(0.0);
        took.set_hexpand(true);
        took.set_margin_start(6);
        took.add_css_class("dim-label");
        took.add_css_class("caption");
        let copy = icons::button("copy", 16, "Copy response");
        copy.add_css_class("flat");
        copy.set_cursor_from_name(Some("pointer"));
        let text = text.to_owned();
        let toasts = self.toasts.clone();
        copy.connect_clicked(move |button| {
            button.clipboard().set_text(&text);
            toasts.add_toast(adw::Toast::builder().title("Copied to clipboard").timeout(2).build());
        });
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        footer.append(&took);
        footer.append(&copy);
        message.append(&footer);
        self.append(&message);
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
        self.started.set(Instant::now());
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
        let elapsed = format_elapsed(self.started.get().elapsed(), false);
        let dots = THINKING_DOTS[self.thinking_state.get()];
        if self.thinking.is_visible() {
            // On its own centred line the counter stays put while the dots change.
            self.thinking.set_markup(&format!("Thinking{dots}\n<small>{elapsed}</small>"));
        } else {
            self.entry.set_placeholder_text(Some(&format!("Thinking {elapsed}{dots}")));
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

    /// Ctrl+C copies the text selected in the chat as markdown. The labels
    /// cannot take focus, so their own binding never sees the key.
    fn add_copy_shortcut(self: &Rc<Self>) {
        let shortcuts = gtk::ShortcutController::new();
        // The focused entry would otherwise consume the key first.
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        let callback = gtk::CallbackAction::new(move |_, _| {
            let Some(response) = weak.upgrade() else { return glib::Propagation::Proceed };
            if response.entry.selection_bounds().is_some() {
                return glib::Propagation::Proceed;
            }
            let Some(text) = markdown::selected_markdown(&response.chat) else { return glib::Propagation::Proceed };
            response.window.clipboard().set_text(&text);
            glib::Propagation::Stop
        });
        shortcuts.add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string("<Control>c"), Some(callback)));
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

    #[test]
    fn costs_and_token_counts() {
        assert_eq!(format_cost(0.0), "$0.00");
        assert_eq!(format_cost(0.00002), "<$0.0001");
        assert_eq!(format_cost(0.00214), "$0.0021");
        assert_eq!(format_cost(0.0581), "$0.058");
        assert_eq!(format_cost(1.239), "$1.24");
        assert_eq!(format_tokens(980), "980");
        assert_eq!(format_tokens(4210), "4.2k");
        assert_eq!(format_tokens(1_250_000), "1.2M");
        assert_eq!(grouped(980), "980");
        assert_eq!(grouped(4210), "4,210");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn chat_totals() {
        let usage = |cost| Usage { input_tokens: 2000, output_tokens: 400, cost };
        assert_eq!(format_usage(&[]), None);

        let (label, tooltip) = format_usage(&[usage(Some(Cost::Exact(0.002)))]).unwrap();
        assert_eq!(label, "$0.0020");
        assert_eq!(tooltip, "This chat: 1 request\n2,000 input tokens · 400 output tokens");

        let exact = usage(Some(Cost::Exact(0.002)));
        let (label, tooltip) = format_usage(&[exact, usage(Some(Cost::Estimated(0.003)))]).unwrap();
        assert_eq!(label, "~$0.0050");
        assert!(tooltip.starts_with("This chat: 2 requests\n4,000 input tokens · 800 output tokens\n"));
        assert!(tooltip.ends_with("estimated from OpenRouter list prices"));

        // One unpriced request leaves the total unknown.
        let (label, tooltip) = format_usage(&[exact, usage(None)]).unwrap();
        assert_eq!(label, "4.0k in · 800 out");
        assert!(!tooltip.contains("estimated"));
    }

    #[test]
    fn elapsed_times() {
        assert_eq!(format_elapsed(Duration::from_millis(3240), true), "3.2s");
        assert_eq!(format_elapsed(Duration::from_millis(3940), false), "3s");
        assert_eq!(format_elapsed(Duration::from_secs(65), true), "1m 05s");
        assert_eq!(format_elapsed(Duration::from_secs(125), false), "2m 05s");
    }
}
