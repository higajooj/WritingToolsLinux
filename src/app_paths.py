"""Locations of editable files and bundled resources in source and frozen runs."""

from pathlib import Path
import sys


def app_root():
    """Directory containing the source launcher or packaged executable."""
    if getattr(sys, "frozen", False):
        return Path(sys.executable).resolve().parent
    return Path(__file__).resolve().parent.parent


def asset_root():
    """PyInstaller unpacks resources to _MEIPASS; source keeps them in assets/."""
    if getattr(sys, "frozen", False):
        return Path(getattr(sys, "_MEIPASS", app_root()))
    return app_root() / "assets"
