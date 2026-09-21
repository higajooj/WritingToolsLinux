"""Onboarding behavior (run Qt offscreen)."""

from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import qt_offscreen  # noqa: F401,E402

from PySide6 import QtWidgets

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from ui.OnboardingWindow import OnboardingWindow


class OnboardingWindowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def test_setup_saves_shortcut_without_theme_preference(self):
        app = SimpleNamespace(config=None, show_settings=Mock())
        window = OnboardingWindow(app)
        self.addCleanup(window.deleteLater)

        self.assertFalse(window.findChildren(QtWidgets.QRadioButton))
        window.shortcut_input.setText("ctrl+shift+space")
        window.on_next_clicked()

        self.assertEqual(app.config, {"shortcut": "ctrl+shift+space"})
        app.show_settings.assert_called_once_with(providers_only=True)


if __name__ == "__main__":
    unittest.main()
