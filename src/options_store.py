import json
import shutil

from app_paths import default_options_path, options_path


def load_options():
    """Load user options, seeding them from the tracked defaults if absent."""
    path = options_path()
    if not path.exists():
        shutil.copyfile(default_options_path(), path)

    with path.open("r", encoding="utf-8") as options_file:
        return json.load(options_file)


def save_options(options):
    """Persist user options beside the application entry point."""
    with options_path().open("w", encoding="utf-8") as options_file:
        json.dump(options, options_file, indent=2)


def reset_options():
    """Replace user options with the current tracked defaults."""
    shutil.copyfile(default_options_path(), options_path())
    return load_options()
