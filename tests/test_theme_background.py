"""Solid application background rendering (run Qt offscreen)."""

from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import qt_offscreen  # noqa: F401,E402

from PySide6 import QtCore, QtGui, QtWidgets

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from ui.UIUtils import ThemeBackground


class ThemeBackgroundTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def render_background(self, mode, border_radius=0):
        with patch("ui.UIUtils.colorMode", mode):
            widget = ThemeBackground(border_radius=border_radius)
            widget.setAttribute(QtCore.Qt.WidgetAttribute.WA_TranslucentBackground)
            widget.resize(40, 40)
            image = QtGui.QImage(widget.size(), QtGui.QImage.Format.Format_ARGB32)
            image.fill(QtCore.Qt.GlobalColor.transparent)
            widget.render(image)
            return image

    def test_light_and_dark_background_colors(self):
        for mode, expected in (("light", "#dedede"), ("dark", "#232323")):
            with self.subTest(mode=mode):
                image = self.render_background(mode)
                self.assertEqual(image.pixelColor(20, 20).name(), expected)

    def test_rounded_background_leaves_corner_transparent(self):
        image = self.render_background("dark", border_radius=10)

        self.assertEqual(image.pixelColor(0, 0).alpha(), 0)
        self.assertEqual(image.pixelColor(20, 20), QtGui.QColor("#232323"))


if __name__ == "__main__":
    unittest.main()
