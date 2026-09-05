import os
from pathlib import Path

# Portal / desktop-entry application ID. It lives here rather than in
# platform_input so every module that needs it can share one definition;
# platform_input already imports this module, so the dependency only goes
# one way.
APP_ID = "com.writingtools.WritingTools"


def app_root():
    return Path(__file__).resolve().parent.parent


def asset_root():
    return app_root() / "assets"


def data_home():
    """Return $XDG_DATA_HOME, falling back to the spec's default location."""
    value = os.environ.get("XDG_DATA_HOME")
    if value:
        candidate = Path(value)
        # The XDG spec says a relative value must be treated as unset.
        if candidate.is_absolute():
            return candidate
    return Path.home() / ".local" / "share"


def codex_data_root():
    """Return the private data directory used for Writing Tools' Codex login."""
    return data_home() / APP_ID / "codex"
