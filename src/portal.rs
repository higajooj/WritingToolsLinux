//! Global shortcut registration through the XDG GlobalShortcuts portal, and
//! the desktop entry that the portal requires for our app ID.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use ashpd::desktop::CreateSessionOptions;
use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use tokio::sync::oneshot;

use crate::paths::{self, APP_ID};
use crate::runtime;

/// The only portal shortcut; per-button shortcuts are local to the popup.
pub const GLOBAL_ID: &str = "global";

const APP_ICON: &[u8] = include_bytes!("../assets/icons/app_icon.png");

/// Recognised modifier spellings and their portal / Hyprland names.
fn modifier_name(part: &str) -> Option<&'static str> {
    match part.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some("CTRL"),
        "alt" => Some("ALT"),
        "shift" => Some("SHIFT"),
        "super" | "meta" | "win" | "cmd" => Some("SUPER"),
        _ => None,
    }
}

fn split_trigger(trigger: &str) -> Vec<&str> {
    trigger.split('+').map(str::trim).collect()
}

/// Validate the modifier-plus-key syntax used by configured shortcuts.
pub fn validate_trigger(trigger: &str) -> Result<(), &'static str> {
    let parts = split_trigger(trigger);
    if parts.iter().any(|part| part.is_empty()) {
        return Err("Separate keys with '+', for example ctrl+space.");
    }
    let modifiers = parts.iter().filter(|part| modifier_name(part).is_some()).count();
    if modifiers == 0 {
        return Err("Add a modifier, for example ctrl+space.");
    }
    if parts.len() - modifiers != 1 {
        return Err("Use one key with the modifiers, for example super+p.");
    }
    Ok(())
}

/// Portal trigger string. Key case is kept because the portal treats
/// `SUPER+p` and `SUPER+P` differently.
pub fn portal_trigger(trigger: &str) -> String {
    let parts: Vec<&str> = split_trigger(trigger).into_iter().filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return trigger.to_owned();
    }
    parts.iter().map(|part| modifier_name(part).unwrap_or(part)).collect::<Vec<_>>().join("+")
}

pub fn hyprland_bind(trigger: &str, shortcut_id: &str) -> String {
    let parts: Vec<&str> = split_trigger(trigger).into_iter().filter(|p| !p.is_empty()).collect();
    let modifiers: Vec<&str> = parts.iter().filter_map(|part| modifier_name(part)).collect();
    let key = parts.iter().find(|part| modifier_name(part).is_none()).map(|key| key.to_uppercase()).unwrap_or_default();
    format!("bind = {}, {key}, global, {APP_ID}:{shortcut_id}", modifiers.join(" "))
}

pub fn is_hyprland() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").map(|value| value.to_lowercase().contains("hyprland")).unwrap_or(false)
}

/// Hyprland ignores `preferred_trigger`; tell the user the bind to add.
pub fn compositor_bind_hint(trigger: &str) -> String {
    if !is_hyprland() {
        return String::new();
    }
    format!("Hyprland binds portal shortcuts in its own config: {}", hyprland_bind(trigger, GLOBAL_ID))
}

pub fn desktop_entry_path() -> PathBuf {
    paths::data_home().join("applications").join(format!("{APP_ID}.desktop"))
}

fn icon_path() -> PathBuf {
    paths::data_home().join("icons/hicolor/256x256/apps").join(format!("{APP_ID}.png"))
}

pub fn desktop_entry_contents(executable: &std::path::Path) -> String {
    [
        "[Desktop Entry]".to_owned(),
        "Type=Application".to_owned(),
        "Name=Writing Tools".to_owned(),
        "Comment=AI writing assistant".to_owned(),
        format!("Exec={}", shell_quote(&executable.to_string_lossy())),
        format!("Icon={APP_ID}"),
        "Terminal=false".to_owned(),
        "Categories=Utility;".to_owned(),
        format!("StartupWMClass={APP_ID}"),
        String::new(),
    ]
    .join("\n")
}

/// Desktop-entry Exec quoting: quote only when needed.
fn shell_quote(value: &str) -> String {
    if !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        return value.to_owned();
    }
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`").replace('$', "\\$");
    format!("\"{escaped}\"")
}

/// Create the desktop entry and icon required by the portal. Existing files
/// are left unchanged. Returns a user-facing problem, or an empty string.
pub fn ensure_desktop_entry() -> String {
    let icon = icon_path();
    if !icon.exists() {
        let result =
            icon.parent().map(std::fs::create_dir_all).unwrap_or(Ok(())).and_then(|_| std::fs::write(&icon, APP_ICON));
        if let Err(e) = result {
            log::warn!("Could not install the application icon at {}: {e}", icon.display());
        }
    }

    let path = desktop_entry_path();
    if path.exists() {
        return String::new();
    }
    let result = std::env::current_exe().and_then(|exe| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, desktop_entry_contents(&exe))
    });
    match result {
        Ok(()) => {
            log::info!("Installed desktop entry {} for portal app ID {APP_ID}", path.display());
            String::new()
        }
        Err(e) => {
            let message = format!(
                "Could not create desktop entry at {} ({e}). Wayland global shortcuts require it.",
                path.display()
            );
            log::warn!("{message}");
            message
        }
    }
}

pub enum PortalEvent {
    /// Registration finished; `Err` carries the portal's error text.
    Registered(Result<(), String>),
    Activated(String),
}

/// One live portal session. Dropping it or calling [`stop`](Self::stop)
/// closes the session.
#[derive(Default)]
pub struct ShortcutPortal {
    stop: Option<oneshot::Sender<()>>,
}

impl ShortcutPortal {
    /// Register the main shortcut, replacing any previous session. Events are
    /// delivered on `events`.
    pub fn register(&mut self, trigger: &str, events: async_channel::Sender<PortalEvent>) {
        self.stop();
        let (stop_tx, stop_rx) = oneshot::channel();
        self.stop = Some(stop_tx);
        let trigger = trigger.to_owned();
        runtime::spawn(async move {
            if let Err(e) = run_session(&trigger, &events, stop_rx).await {
                log::warn!("Wayland GlobalShortcuts unavailable: {e}");
                let _ = events.send(PortalEvent::Registered(Err(e.to_string()))).await;
            }
        });
    }

    pub fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl Drop for ShortcutPortal {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn register_app_id() {
    // Registry.Register must be the first portal call on the connection, and
    // may only happen once. Older portals lack it, so failure is not fatal.
    static REGISTERED: AtomicBool = AtomicBool::new(false);
    if REGISTERED.swap(true, Ordering::SeqCst) {
        return;
    }
    match APP_ID.parse() {
        Ok(app_id) => match ashpd::register_host_app(app_id).await {
            Ok(()) => log::debug!("Registered portal app id {APP_ID}"),
            Err(e) => log::info!("Portal app id registration skipped: {e}"),
        },
        Err(e) => log::warn!("Invalid app id {APP_ID}: {e}"),
    }
}

async fn run_session(
    trigger: &str,
    events: &async_channel::Sender<PortalEvent>,
    mut stop: oneshot::Receiver<()>,
) -> ashpd::Result<()> {
    register_app_id().await;
    let portal = GlobalShortcuts::new().await?;
    let session = portal.create_session(CreateSessionOptions::default()).await?;
    let result = async {
        let preferred = portal_trigger(trigger);
        let shortcuts =
            [NewShortcut::new(GLOBAL_ID, format!("Writing Tools: {GLOBAL_ID}")).preferred_trigger(preferred.as_str())];
        let mut activated = portal.receive_activated().await?;
        portal.bind_shortcuts(&session, &shortcuts, None, BindShortcutsOptions::default()).await?.response()?;
        log::info!("Wayland GlobalShortcuts bound for {APP_ID}: {GLOBAL_ID}");
        let hint = compositor_bind_hint(trigger);
        if !hint.is_empty() {
            log::info!("{hint}");
        }
        let _ = events.send(PortalEvent::Registered(Ok(()))).await;

        // Each session is closed before the next is created, so every
        // Activated signal on this connection belongs to this session.
        loop {
            tokio::select! {
                _ = &mut stop => break,
                signal = activated.next() => match signal {
                    Some(signal) => {
                        let _ = events.send(PortalEvent::Activated(signal.shortcut_id().to_owned())).await;
                    }
                    None => break,
                },
            }
        }
        Ok(())
    }
    .await;
    match session.close().await {
        Ok(()) => log::debug!("Portal session closed"),
        Err(e) => log::debug!("Portal session close failed: {e}"),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_triggers() {
        assert!(validate_trigger("ctrl+space").is_ok());
        assert!(validate_trigger("Super + P").is_ok());
        assert!(validate_trigger("ctrl+alt+shift+1").is_ok());
        assert_eq!(validate_trigger(""), Err("Separate keys with '+', for example ctrl+space."));
        assert_eq!(validate_trigger("ctrl++"), Err("Separate keys with '+', for example ctrl+space."));
        assert_eq!(validate_trigger("space"), Err("Add a modifier, for example ctrl+space."));
        assert_eq!(validate_trigger("ctrl"), Err("Use one key with the modifiers, for example super+p."));
        assert_eq!(validate_trigger("ctrl+a+b"), Err("Use one key with the modifiers, for example super+p."));
    }

    #[test]
    fn portal_trigger_keeps_key_case() {
        assert_eq!(portal_trigger("ctrl+space"), "CTRL+space");
        assert_eq!(portal_trigger("super+P"), "SUPER+P");
        assert_eq!(portal_trigger("Meta + alt + x"), "SUPER+ALT+x");
    }

    #[test]
    fn hyprland_binding() {
        assert_eq!(hyprland_bind("super+p", "global"), "bind = SUPER, P, global, com.writingtools.WritingTools:global");
        assert_eq!(
            hyprland_bind("ctrl+shift+space", "global"),
            "bind = CTRL SHIFT, SPACE, global, com.writingtools.WritingTools:global"
        );
    }

    #[test]
    fn desktop_entry_points_at_binary() {
        let entry = desktop_entry_contents(std::path::Path::new("/opt/writing tools/bin"));
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("\nExec=\"/opt/writing tools/bin\"\n"));
        assert!(entry.contains("\nIcon=com.writingtools.WritingTools\n"));
        assert!(entry.contains("\nStartupWMClass=com.writingtools.WritingTools\n"));
        assert!(desktop_entry_contents(std::path::Path::new("/usr/bin/wt")).contains("\nExec=/usr/bin/wt\n"));
    }
}
