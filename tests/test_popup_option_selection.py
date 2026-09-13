"""Popup option selection and per-run instruction behavior."""

import os
from pathlib import Path
import sys
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6 import QtCore, QtWidgets
from PySide6.QtCore import Signal, Slot
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
        self.assertEqual(
            self.window.custom_input.placeholderText(),
            "Add instructions (optional)...",
        )

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

    def test_clicking_selected_option_again_returns_to_custom(self):
        self.button("Proofread").click()
        self.button("Proofread").click()

        self.assertIsNone(self.window.selected_option)
        self.assertFalse(self.button("Proofread").property("selected"))
        self.assertEqual(
            self.window.custom_input.placeholderText(),
            "Describe your change...",
        )

        self.window.custom_input.setText("Turn this into verse")
        self.window.send_button.click()

        self.app.process_option.assert_called_once_with(
            "Custom",
            "Turn this into verse",
        )

    def test_empty_option_name_is_still_a_selection(self):
        self.window.on_generic_instruction("")
        self.window.send_button.click()

        self.app.process_option.assert_called_once_with("", None)

    def test_entering_edit_mode_clears_selection(self):
        self.button("Proofread").click()
        self.window.toggle_edit_mode()

        self.assertIsNone(self.window.selected_option)
        self.assertFalse(self.button("Proofread").property("selected"))


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

    def build_system_instruction(self, option, additional_instructions=None):
        return WritingToolApp._build_system_instruction(
            self.app,
            option,
            additional_instructions,
        )

    def test_system_instruction_unchanged_without_additions(self):
        self.assertEqual(
            self.build_system_instruction("Proofread", "   "),
            "Correct the text.",
        )

    def test_system_instruction_gives_additions_precedence(self):
        instruction = self.build_system_instruction("Proofread", " Translate to French ")

        self.assertTrue(instruction.startswith("Correct the text.\n\n"))
        self.assertIn("override the rules above", instruction)
        self.assertTrue(instruction.endswith("\nTranslate to French"))

    def test_custom_system_instruction_is_unchanged(self):
        self.assertEqual(
            self.build_system_instruction("Custom", "Make it rhyme"),
            "Apply the described change.",
        )


class _ResponseWindowStub(QtCore.QObject):
    def __init__(self):
        super().__init__()
        self.chat_history = []
        self.text = None

    @Slot(str)
    def set_text(self, text):
        self.text = text


class _OptionWorkerHost(QtCore.QObject):
    """The slice of WritingToolApp that `process_option_thread` touches."""

    show_message_signal = Signal(str, str)

    _setup_response_window = WritingToolApp._setup_response_window
    _build_option_prompt = WritingToolApp._build_option_prompt
    _build_system_instruction = WritingToolApp._build_system_instruction
    process_option_thread = WritingToolApp.process_option_thread


class ProcessOptionThreadTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def test_window_option_seeds_history_with_the_prompt_sent_to_provider(self):
        host = _OptionWorkerHost()
        host.options = OPTIONS
        host.input_backend = Mock()
        host.current_text_holder = SimpleNamespace(ready=threading.Event(), text="Long text")
        host.current_text_holder.ready.set()
        window = _ResponseWindowStub()
        host.show_response_window = Mock(return_value=window)
        host.current_provider = Mock()
        host.current_provider.get_response.return_value = "Short summary"

        worker = threading.Thread(
            target=host.process_option_thread,
            args=("Summary", "Use bullets"),
        )
        worker.start()
        while worker.is_alive():
            QtWidgets.QApplication.processEvents()
        worker.join()
        QtWidgets.QApplication.processEvents()

        prompt = "Summarize this:\n\nAdditional instructions: Use bullets\n\nText:\nLong text"
        host.show_response_window.assert_called_once_with("Summary", "Long text")
        self.assertEqual(window.chat_history, [{"role": "user", "content": prompt}])
        system_instruction, sent_prompt = host.current_provider.get_response.call_args.args
        self.assertEqual(sent_prompt, prompt)
        self.assertIn("Use bullets", system_instruction)
        self.assertEqual(window.text, "Short summary")


class ResponseHistoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def test_followups_wait_for_initial_response(self):
        class App(QtCore.QObject):
            followup_response_signal = Signal(str)

        app = App()
        app.config = {}
        window = ResponseWindow(app, "Summary Result")
        self.addCleanup(window.deleteLater)

        self.assertFalse(window.input_field.isEnabled())

        with patch("ui.ResponseWindow.QtCore.QTimer.singleShot"):
            window.set_text("Short summary")

        self.assertTrue(window.input_field.isEnabled())

    def test_initial_response_keeps_preseeded_request(self):
        request = "Summarize this:\n\nAdditional instructions: Use bullets\n\nText:\nLong text"
        response = SimpleNamespace(
            chat_history=[{"role": "user", "content": request}],
            option="Summary",
            selected_text="Long text",
            stop_thinking_animation=Mock(),
            chat_area=Mock(),
            _add_message=Mock(),
            app=SimpleNamespace(config={}),
            _adjust_window_height=Mock(),
        )

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
