//! A record of past uses (`history.json`), shared by every button.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths;

pub const MAX_ENTRIES: usize = 200;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    /// Seconds since the Unix epoch.
    pub time: i64,
    /// The button that was used, or `Custom`.
    pub option: String,
    /// The text the action was applied to.
    pub text: String,
    /// What the user typed for this use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    /// The first reply; absent when the request failed or was cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// Append a use, dropping the oldest beyond `MAX_ENTRIES`. Returns its id.
pub fn push(entries: &mut Vec<Entry>, option: &str, text: &str, extra: Option<&str>, time: i64) -> u64 {
    let id = entries.last().map_or(1, |last| last.id + 1);
    let extra = extra.map(str::trim).filter(|extra| !extra.is_empty()).map(str::to_owned);
    entries.push(Entry { id, time, option: option.to_owned(), text: text.to_owned(), extra, response: None });
    if entries.len() > MAX_ENTRIES {
        entries.drain(..entries.len() - MAX_ENTRIES);
    }
    id
}

/// Attach the reply to a use. Returns false when it has since been dropped.
pub fn set_response(entries: &mut [Entry], id: u64, response: &str) -> bool {
    match entries.iter_mut().rev().find(|entry| entry.id == id) {
        Some(entry) => {
            entry.response = Some(response.to_owned());
            true
        }
        None => false,
    }
}

/// The start of `text` on one line, at most `max_chars` characters.
pub fn snippet(text: &str, max_chars: usize) -> String {
    let mut words = text.split_whitespace();
    let mut result = String::new();
    let mut length = 0;
    for word in &mut words {
        let space = usize::from(!result.is_empty());
        let word_length = word.chars().count();
        if length + space + word_length > max_chars {
            if result.is_empty() {
                result.extend(word.chars().take(max_chars));
            }
            result.push('…');
            return result;
        }
        if space == 1 {
            result.push(' ');
        }
        result.push_str(word);
        length += space + word_length;
    }
    result
}

pub fn load() -> io::Result<Vec<Entry>> {
    load_from(&paths::history_path())
}

pub fn save(entries: &[Entry]) -> io::Result<()> {
    save_to(&paths::history_path(), entries)
}

fn load_from(path: &Path) -> io::Result<Vec<Entry>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn save_to(path: &Path, entries: &[Entry]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(entries).map_err(io::Error::other)?;
    // The file holds whatever the user copied.
    paths::write_private(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_from(&dir.path().join("history.json")).unwrap().is_empty());
    }

    #[test]
    fn round_trips_privately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("history.json");
        let mut entries = Vec::new();
        let first = push(&mut entries, "Proofread", "some text", None, 10);
        push(&mut entries, "Custom", "other text", Some("  make it rhyme "), 20);
        assert!(set_response(&mut entries, first, "Some text."));
        save_to(&path, &entries).unwrap();

        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, entries);
        assert_eq!(loaded[0].response.as_deref(), Some("Some text."));
        assert_eq!(loaded[1].extra.as_deref(), Some("make it rhyme"));
        assert_eq!(loaded[1].response, None);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn blank_extra_is_dropped() {
        let mut entries = Vec::new();
        push(&mut entries, "Rewrite", "text", Some("   "), 0);
        assert_eq!(entries[0].extra, None);
    }

    #[test]
    fn keeps_only_the_newest_entries() {
        let mut entries = Vec::new();
        for i in 0..MAX_ENTRIES + 5 {
            push(&mut entries, "Rewrite", &format!("text {i}"), None, i as i64);
        }
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[0].text, "text 5");
        assert_eq!(entries[0].id, 6);
        // Ids keep increasing after old entries are dropped.
        assert_eq!(push(&mut entries, "Rewrite", "last", None, 0), MAX_ENTRIES as u64 + 6);
        assert!(!set_response(&mut entries, 1, "gone"));
    }

    #[test]
    fn snippet_collapses_whitespace_and_truncates() {
        assert_eq!(snippet("  one\n\ttwo   three ", 100), "one two three");
        assert_eq!(snippet("one two three", 7), "one two…");
        assert_eq!(snippet("one two three", 6), "one…");
        assert_eq!(snippet("ñandúñandú", 4), "ñand…");
        assert_eq!(snippet("", 10), "");
    }
}
