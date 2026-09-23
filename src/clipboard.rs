//! Wayland clipboard access through wl-clipboard. Wayland does not let a
//! client read another application's selection, so the user copies first.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(2);

pub fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| in_path("wl-paste") && in_path("wl-copy"))
}

fn in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

/// Read the clipboard text, or an empty string if unavailable. Blocking.
pub fn read() -> String {
    if !available() {
        return String::new();
    }
    let Ok(mut child) = Command::new("wl-paste")
        .arg("--no-newline")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return String::new();
    };
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut stdout, &mut bytes).map(|_| bytes)
    });
    if !wait_with_timeout(&mut child) {
        return String::new();
    }
    match reader.join() {
        Ok(Ok(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        _ => String::new(),
    }
}

/// Copy `text` to the clipboard. Blocking; returns whether it succeeded.
pub fn write(text: &str) -> bool {
    if !available() {
        return false;
    }
    // wl-copy forks a daemon that keeps owning the selection and inherits our
    // stdio. Pipes would never see EOF, so its output goes to /dev/null.
    let Ok(mut child) =
        Command::new("wl-copy").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    else {
        return false;
    };
    let wrote = child.stdin.take().map(|mut stdin| stdin.write_all(text.as_bytes()).is_ok()).unwrap_or(false);
    wait_with_timeout(&mut child) && wrote
}

/// Wait for `child`; kill it after [`TIMEOUT`]. Returns whether it exited successfully.
fn wait_with_timeout(child: &mut std::process::Child) -> bool {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}
