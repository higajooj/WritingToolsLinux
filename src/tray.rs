//! StatusNotifierItem tray icon.

use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, ToolTip, TrayMethods};

use crate::icons;
use crate::paths::APP_ID;
use crate::runtime;

#[derive(Clone, Copy, Debug)]
pub enum TrayAction {
    Settings,
    TogglePause,
    About,
    Exit,
}

struct WritingToolsTray {
    normal: Vec<Icon>,
    ready_icon: Vec<Icon>,
    ready: bool,
    paused: bool,
    actions: async_channel::Sender<TrayAction>,
}

impl WritingToolsTray {
    fn item(&self, label: &str, action: TrayAction) -> MenuItem<Self> {
        StandardItem {
            label: label.into(),
            activate: Box::new(move |tray: &mut Self| {
                let _ = tray.actions.try_send(action);
            }),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for WritingToolsTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        APP_ID.into()
    }

    fn title(&self) -> String {
        "WritingTools".into()
    }

    fn icon_name(&self) -> String {
        if self.normal.is_empty() { "accessories-text-editor".into() } else { String::new() }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        if self.ready { self.ready_icon.clone() } else { self.normal.clone() }
    }

    fn tool_tip(&self) -> ToolTip {
        let title = if self.ready { "WritingTools — Ready to paste" } else { "WritingTools" };
        ToolTip { title: title.into(), ..Default::default() }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            self.item("Settings", TrayAction::Settings),
            self.item(if self.paused { "Resume" } else { "Pause" }, TrayAction::TogglePause),
            self.item("About", TrayAction::About),
            self.item("Exit", TrayAction::Exit),
        ]
    }
}

fn pixmaps(pixbuf: Option<gtk::gdk_pixbuf::Pixbuf>) -> Vec<Icon> {
    pixbuf
        .map(|pixbuf| {
            let (width, height, data) = icons::argb(&pixbuf);
            vec![Icon { width, height, data }]
        })
        .unwrap_or_default()
}

pub struct Tray {
    handle: ksni::Handle<WritingToolsTray>,
}

impl Tray {
    /// Start the tray. Menu choices arrive on `actions`. Must run on the GTK thread
    /// (icons are rendered there).
    pub async fn spawn(actions: async_channel::Sender<TrayAction>) -> Option<Self> {
        let tray = WritingToolsTray {
            normal: pixmaps(icons::app_icon_pixbuf(64)),
            ready_icon: pixmaps(icons::pixbuf("clipboard_ready", 64, None)),
            ready: false,
            paused: false,
            actions,
        };
        match runtime::spawn(tray.spawn()).await {
            Ok(Ok(handle)) => Some(Self { handle }),
            Ok(Err(e)) => {
                log::warn!("Could not create the tray icon: {e}");
                None
            }
            Err(e) => {
                log::warn!("Could not create the tray icon: {e}");
                None
            }
        }
    }

    pub fn set_ready(&self, ready: bool) {
        self.update(move |tray| tray.ready = ready);
    }

    pub fn set_paused(&self, paused: bool) {
        self.update(move |tray| tray.paused = paused);
    }

    fn update(&self, f: impl FnOnce(&mut WritingToolsTray) + Send + 'static) {
        let handle = self.handle.clone();
        runtime::spawn(async move {
            handle.update(f).await;
        });
    }

    pub fn shutdown(&self) {
        let handle = self.handle.clone();
        runtime::runtime().block_on(async move { handle.shutdown().await });
    }
}
