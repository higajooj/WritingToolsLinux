"""Popup option selection and per-run instruction behavior."""

import os
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6 import QtCore, QtWidgets
from PySide6.QtTest import QTest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from WritingToolApp import WritingToolApp
from ui.CustomPopupWindow import CustomPopupWindow
from ui.ResponseWindow import ResponseWindow


OPTIONS = {
    "Proofread": {
        "prefix": "Proofread this:\n\n",
        "instruction": "Correct the text.",
        "icon": "icons/magnifying-glass",
        "open_in_window": False,
    },
    "Summary": {
        "prefix": "Summarize this:\n\n",
        "instruction": "Summarize the text.",
        "icon": "icons/summary",
        "open_in_window": True,
    },
    "Custom": {
        "prefix": "Make this change:\n\n",
        "instruction": "Apply the described change.",
        "icon": "icons/summary",
        "open_in_window": False,
    },
}


class PopupOptionSelectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def setUp(self):
        self.app = SimpleNamespace(config={"theme": "plain"}, process_option=Mock())
        self.options_patch = patch.object(
            CustomPopupWindow,
            "load_options",
            return_value=OPTIONS,
        )
        self.options_patch.start()
        self.window = CustomPopupWindow(self.app)
        self.window.show()
        QtWidgets.QApplication.processEvents()

    def tearDown(self):
        self.window.close()
        self.window.deleteLater()
        self.options_patch.stop()
        QtWidgets.QApplication.processEvents()

    def button(self, key):
        return next(button for button in self.window.button_widgets if button.key == key)

    def test_click_selects_option_without_dispatching_or_closing(self):
        self.button("Proofread").click()

        self.app.process_option.assert_not_called()
        self.assertTrue(self.window.isVisible())
        self.assertEqual(self.window.selected_option, "Proofread")
        self.assertTrue(self.button("Proofread").property("selected"))
        self.assertIn("Proofread", self.window.custom_input.placeholderText())

    def test_enter_runs_selected_option_with_optional_instructions(self):
        self.button("Proofread").click()
        self.window.custom_input.setText("Use British English")
        self.window.custom_input.setFocus()
        QTest.keyClick(self.window.custom_input, QtCore.Qt.Key_Return)

        self.app.process_option.assert_called_once_with(
            "Proofread",
            "Use British English",
        )
        self.assertFalse(self.window.isVisible())

    def test_empty_input_runs_selected_option_normally(self):
        self.button("Proofread").click()
        self.window.send_button.click()

        self.app.process_option.assert_called_once_with("Proofread", None)

    def test_text_without_selection_keeps_custom_action(self):
        self.window.custom_input.setText("Turn this into verse")
        self.window.send_button.click()

        self.app.process_option.assert_called_once_with(
            "Custom",
            "Turn this into verse",
        )

    def test_switching_options_retains_text_and_moves_selection(self):
        self.button("Proofread").click()
        self.window.custom_input.setText("Keep the headings")
        self.button("Summary").click()

        self.assertEqual(self.window.selected_option, "Summary")
        self.assertEqual(self.window.custom_input.text(), "Keep the headings")
        self.assertFalse(self.button("Proofread").property("selected"))
        self.assertTrue(self.button("Summary").property("selected"))

    def test_empty_input_without_selection_does_nothing(self):
        self.window.send_button.click()

        self.app.process_option.assert_not_called()
        self.assertTrue(self.window.isVisible())


class OptionPromptTests(unittest.TestCase):
    def setUp(self):
        self.app = SimpleNamespace(options=OPTIONS)

    def build_prompt(self, option, selected_text, additional_instructions=None):
        return WritingToolApp._build_option_prompt(
            self.app,
            option,
            selected_text,
            additional_instructions,
        )

    def test_empty_addition_preserves_existing_named_option_prompt(self):
        self.assertEqual(
            self.build_prompt("Proofread", "Original text"),
            "Proofread this:\n\nOriginal text",
        )

    def test_named_option_includes_additional_instructions(self):
        self.assertEqual(
            self.build_prompt("Proofread", "Original text", "Keep slang"),
            "Proofread this:\n\nAdditional instructions: Keep slang"
            "\n\nText:\nOriginal text",
        )

    def test_custom_prompt_keeps_existing_shape(self):
        self.assertEqual(
            self.build_prompt("Custom", "Original text", "Make it rhyme"),
            "Make this change:\n\nDescribed change: Make it rhyme"
            "\n\nText: Original text",
        )


class ResponseHistoryTests(unittest.TestCase):
    def test_initial_response_keeps_preseeded_request(self):
        request = "Summarize this:\n\nAdditional instructions: Use bullets\n\nText:\nLong text"
        response = SimpleNamespace(
            chat_history=[{"role": "user", "content": request}],
            option="Summary",
            selected_text="Long text",
            stop_thinking_animation=Mock(),
            chat_area=Mock(),
            app=SimpleNamespace(config={}),
            _adjust_window_height=Mock(),
        )
        response.chat_area.add_message.return_value = Mock()

        with patch("ui.ResponseWindow.QtCore.QTimer.singleShot"):
            ResponseWindow.set_text(response, "Short summary")

        self.assertEqual(
            response.chat_history,
            [
                {"role": "user", "content": request},
                {"role": "assistant", "content": "Short summary"},
            ],
        )


if __name__ == "__main__":
    unittest.main()
