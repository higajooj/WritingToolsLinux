//! Filesystem locations used by Writing Tools.

use std::path::PathBuf;

/// Portal / desktop-entry application ID, also the GApplication ID.
pub const APP_ID: &str = "com.writingtools.WritingTools";

/// Return an XDG base directory from `var`, falling back to `$HOME/<fallback>`.
/// The XDG spec says a relative value must be treated as unset.
fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    if let Some(value) = std::env::var_os(var) {
        let candidate = PathBuf::from(value);
        if candidate.is_absolute() {
            return candidate;
        }
    }
    home().join(fallback)
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn data_home() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share")
}

pub fn config_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config").join("writing-tools")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn options_path() -> PathBuf {
    config_dir().join("options.json")
}

/// Private data directory used for Writing Tools' Codex login.
pub fn codex_data_root() -> PathBuf {
    data_home().join(APP_ID).join("codex")
}

#[cfg(test)]
pub(crate) mod test_env {
    use std::sync::{Mutex, MutexGuard};

    /// Tests that change process environment variables must hold this lock.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub fn lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn set(var: &str, value: Option<&std::path::Path>) {
        // SAFETY: callers hold ENV_LOCK, so no other test reads the environment concurrently.
        unsafe {
            match value {
                Some(value) => std::env::set_var(var, value),
                None => std::env::remove_var(var),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn absolute_xdg_values_are_used() {
        let _guard = test_env::lock();
        test_env::set("XDG_DATA_HOME", Some(Path::new("/tmp/data")));
        test_env::set("XDG_CONFIG_HOME", Some(Path::new("/tmp/config")));
        assert_eq!(data_home(), PathBuf::from("/tmp/data"));
        assert_eq!(options_path(), PathBuf::from("/tmp/config/writing-tools/options.json"));
        assert_eq!(codex_data_root(), PathBuf::from("/tmp/data/com.writingtools.WritingTools/codex"));
    }

    #[test]
    fn relative_xdg_values_are_ignored() {
        let _guard = test_env::lock();
        test_env::set("HOME", Some(Path::new("/home/someone")));
        test_env::set("XDG_DATA_HOME", Some(Path::new("relative")));
        test_env::set("XDG_CONFIG_HOME", None);
        assert_eq!(data_home(), PathBuf::from("/home/someone/.local/share"));
        assert_eq!(config_path(), PathBuf::from("/home/someone/.config/writing-tools/config.json"));
    }
}
