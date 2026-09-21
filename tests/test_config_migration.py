"""Startup config migration (run Qt offscreen)."""

from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

# Qt must be pinned to the offscreen platform before PySide6 is imported.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import qt_offscreen  # noqa: F401,E402

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from app_paths import CONFIG_VERSION
from WritingToolApp import WritingToolApp


def current_flags():
    return {f"is_config_file_updated_for_v{n}": True for n in range(8, CONFIG_VERSION + 1)}


class ConfigMigrationTests(unittest.TestCase):
    def migrate(self, config):
        app = SimpleNamespace(config=config, save_config=Mock())
        with patch("WritingToolApp.QMessageBox") as message_box, \
                patch("WritingToolApp.sys.exit") as exit_:
            WritingToolApp._migrate_config(app)
        return app, message_box, exit_

    def test_v10_removes_locale_without_restart(self):
        config = {
            "is_config_file_updated_for_v8": True,
            "is_config_file_updated_for_v9": True,
            "locale": "it",
        }
        app, message_box, exit_ = self.migrate(config)

        self.assertNotIn("locale", app.config)
        self.assertLessEqual(current_flags().items(), app.config.items())
        app.save_config.assert_called_once_with(app.config)
        message_box.information.assert_not_called()
        exit_.assert_not_called()

    def test_v10_stamps_config_without_locale(self):
        config = {
            "is_config_file_updated_for_v8": True,
            "is_config_file_updated_for_v9": True,
        }
        app, message_box, exit_ = self.migrate(config)

        self.assertEqual(app.config, current_flags())
        app.save_config.assert_called_once_with(app.config)
        exit_.assert_not_called()

    def test_v11_removes_theme_without_restart(self):
        config = {
            **{f"is_config_file_updated_for_v{n}": True for n in range(8, 11)},
            "theme": "gradient",
            "shortcut": "ctrl+space",
        }
        app, message_box, exit_ = self.migrate(config)

        self.assertNotIn("theme", app.config)
        self.assertEqual(app.config["shortcut"], "ctrl+space")
        self.assertLessEqual(current_flags().items(), app.config.items())
        app.save_config.assert_called_once_with(app.config)
        message_box.information.assert_not_called()
        exit_.assert_not_called()

        migrated_again, _, second_exit = self.migrate(app.config)
        migrated_again.save_config.assert_not_called()
        second_exit.assert_not_called()

    def test_current_config_is_left_alone(self):
        app, _, exit_ = self.migrate(current_flags() | {"shortcut": "ctrl+space"})

        app.save_config.assert_not_called()
        exit_.assert_not_called()


if __name__ == "__main__":
    unittest.main()
