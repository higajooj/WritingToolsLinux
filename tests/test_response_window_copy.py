"""Per-response copy controls in response popup windows."""

from pathlib import Path
import sys
from unittest.mock import Mock, patch
import unittest

# Qt must be pinned to the offscreen platform before PySide6 is imported.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import qt_offscreen  # noqa: F401,E402

from PySide6 import QtCore, QtTest, QtWidgets

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from ui.ResponseWindow import ResponseWindow


class ResponseWindowCopyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def setUp(self):
        app = Mock()
        app.config = {}
        self.window = ResponseWindow(app, "Summary Result")

    def tearDown(self):
        self.window.thinking_timer.stop()
        self.window.close()
        self.window.deleteLater()
        QtWidgets.QApplication.processEvents()

    def copy_buttons(self):
        return self.window.chat_area.findChildren(
            QtWidgets.QPushButton,
            "copyResponseButton",
        )

    def test_only_assistant_messages_have_copy_icons(self):
        with patch("ui.ResponseWindow.QtCore.QTimer.singleShot"):
            self.window.display_original_text("Original text")
            self.window.set_text("First response")
            self.window.input_field.setText("Follow-up question")
            self.window.send_message()
            self.window.handle_followup_response("Second response")

        buttons = self.copy_buttons()
        self.assertEqual(len(buttons), 2)
        self.assertTrue(all(button.icon().isNull() is False for button in buttons))
        self.assertTrue(all(button.toolTip() == "Copy response" for button in buttons))

    def test_clicking_a_later_copy_icon_copies_that_response(self):
        with patch("ui.ResponseWindow.QtCore.QTimer.singleShot"):
            self.window.set_text("First response")
            self.window.input_field.setText("Follow-up question")
            self.window.send_message()
            self.window.handle_followup_response("Second **Markdown** response")

        second_copy_button = self.copy_buttons()[1]
        QtTest.QTest.mouseClick(second_copy_button, QtCore.Qt.MouseButton.LeftButton)

        self.assertEqual(
            QtWidgets.QApplication.clipboard().text(),
            "Second **Markdown** response",
        )


if __name__ == "__main__":
    unittest.main()
