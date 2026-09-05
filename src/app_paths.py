import os
from pathlib import Path


def app_root():
    return Path(__file__).resolve().parent.parent


def asset_root():
    return app_root() / "assets"


def codex_data_root():
    """Return the private data directory used for Writing Tools' Codex login."""
    data_home = os.environ.get("XDG_DATA_HOME")
    if data_home:
        return Path(data_home) / "com.writingtools.WritingTools" / "codex"
    return Path.home() / ".local" / "share" / "com.writingtools.WritingTools" / "codex"
