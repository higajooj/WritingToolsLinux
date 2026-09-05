"""Locations of editable files and bundled resources in a source checkout."""

from pathlib import Path


def app_root():
    """Repository root: the directory containing main.py."""
    return Path(__file__).resolve().parent.parent


def asset_root():
    """Icons, backgrounds, and translations live in assets/."""
    return app_root() / "assets"
