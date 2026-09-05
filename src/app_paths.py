"""Locations of editable files and bundled resources in source and frozen runs."""

from pathlib import Path
import sys


def app_root():
    """Directory containing the source launcher or packaged executable."""
    if getattr(sys, "frozen", False):
        return Path(sys.executable).resolve().parent
    return Path(__file__).resolve().parent.parent


def asset_root():
    """PyInstaller keeps resources beside the executable; source uses assets/."""
    root = app_root()
    return root if getattr(sys, "frozen", False) else root / "assets"
