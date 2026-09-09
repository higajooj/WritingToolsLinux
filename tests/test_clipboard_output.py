"""Clipboard delivery and transient tray feedback (run Qt offscreen)."""

from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from PySide6 import QtCore, QtGui, QtWidgets
from PySide6.QtTest import QTest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from WritingToolApp import WritingToolApp
from aiprovider import GeminiProvider, OllamaProvider
from app_paths import asset_root
from platform_input import WaylandInputBackend
from ui.ResponseWindow import ResponseWindow


class OutputHarness(QtCore.QObject):
    """Exercise the app's output flow without starting providers or shortcuts."""

    handle_output_ready = WritingToolApp.handle_output_ready
    create_tray_icon = WritingToolApp.create_tray_icon
    show_clipboard_ready = WritingToolApp.show_clipboard_ready
    restore_tray_icon = WritingToolApp.restore_tray_icon

    def __init__(self):
        super().__init__()
        self.input_backend = Mock(spec=WaylandInputBackend)
        self.input_backend.write_clipboard.return_value = True
        self.show_message_signal = Mock()
        self.update_tray_menu = Mock()
        self.tray_icon = None
        self._ = lambda text: text
        normal_icon = QtGui.QIcon(str(asset_root() / "icons" / "app_icon.png"))
        tray = Mock()
        tray.icon.return_value = normal_icon
        with patch("WritingToolApp.QtWidgets.QSystemTrayIcon", return_value=tray):
            self.create_tray_icon()


class ClipboardOutputTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def setUp(self):
        self.app = OutputHarness()

    def tearDown(self):
        self.app.tray_ready_timer.stop()
        self.app.tray_menu.deleteLater()
        self.app.deleteLater()

    def test_copies_response_once_without_reading_or_restoring_clipboard(self):
        self.app.handle_output_ready("  Corrected text.\n")

        self.app.input_backend.write_clipboard.assert_called_once_with("  Corrected text.")
        self.app.input_backend.read_clipboard.assert_not_called()
        self.app.tray_icon.setIcon.assert_called_once_with(self.app.ready_tray_icon)
        self.assertIn("Ready to paste", self.app.tray_icon.setToolTip.call_args.args[0])
        self.assertEqual(self.app.tray_ready_timer.interval(), 5000)
        self.assertTrue(self.app.tray_ready_timer.isActive())
        self.assertFalse(self.app.ready_tray_icon.pixmap(24, 24).isNull())

    def test_short_valid_responses_are_not_treated_as_partial_error_markers(self):
        self.app.handle_output_ready("E")
        self.app.input_backend.write_clipboard.assert_called_once_with("E")

    def test_empty_or_incompatible_output_does_not_copy_or_signal_ready(self):
        for response in ("", " \n", None, True, " ERROR_TEXT_INCOMPATIBLE_WITH_REQUEST\n"):
            self.app.handle_output_ready(response)
        self.app.input_backend.write_clipboard.assert_not_called()
        self.app.tray_icon.setIcon.assert_not_called()
        self.assertFalse(self.app.tray_ready_timer.isActive())

        titles = [call.args[0] for call in self.app.show_message_signal.emit.call_args_list]
        self.assertEqual(titles, ["Empty Response"] * 4 + ["Error"])
        self.assertEqual(
            self.app.show_message_signal.emit.call_args.args,
            ("Error", "The text is incompatible with the requested change."),
        )

    def test_failed_copy_reports_error_without_ready_indicator(self):
        self.app.input_backend.write_clipboard.return_value = False
        self.app.handle_output_ready("Corrected text.")
        self.app.show_message_signal.emit.assert_called_once()
        self.app.tray_icon.setIcon.assert_not_called()
        self.assertFalse(self.app.tray_ready_timer.isActive())

    def test_new_result_restarts_indicator_and_timeout_keeps_clipboard(self):
        self.app.handle_output_ready("First result")
        self.app.tray_ready_timer.setInterval(40)
        QTest.qWait(20)
        self.app.handle_output_ready("Second result")
        QTest.qWait(60)  # The first result's deadline has passed.
        self.assertTrue(self.app.tray_ready_timer.isActive())
        self.app.tray_icon.setIcon.assert_called_with(self.app.ready_tray_icon)

        self.app.tray_ready_timer.setInterval(20)
        QTest.qWait(60)
        self.assertFalse(self.app.tray_ready_timer.isActive())
        self.app.tray_icon.setIcon.assert_called_with(self.app.normal_tray_icon)
        self.app.tray_icon.setToolTip.assert_called_with("WritingTools")
        self.assertEqual(self.app.input_backend.write_clipboard.call_count, 2)
        self.app.input_backend.write_clipboard.assert_called_with("Second result")

    def test_stale_result_is_dropped_while_a_response_window_is_active(self):
        # spec= keeps this honest: a reintroduced call to a method the real
        # window does not have (e.g. the old append_text) fails instead of
        # being auto-created by Mock.
        self.app.current_response_window = Mock(spec=ResponseWindow)
        self.app.handle_output_ready("Summary")
        self.app.input_backend.write_clipboard.assert_not_called()
        self.app.tray_icon.setIcon.assert_not_called()
        self.app.show_message_signal.emit.assert_not_called()
        self.assertFalse(self.app.tray_ready_timer.isActive())


class ProviderOutputTests(unittest.TestCase):
    def test_provider_errors_do_not_become_clipboard_results(self):
        for provider_class in (GeminiProvider, OllamaProvider):
            with self.subTest(provider=provider_class.__name__):
                app = SimpleNamespace(output_ready_signal=Mock(), show_message_signal=Mock())
                provider = provider_class(app)
                provider.model_name = "gemini-flash-latest"
                provider.api_model = "test-model"
                provider.client = Mock()
                provider.client.models.generate_content.side_effect = RuntimeError("Unavailable")
                provider.client.chat.side_effect = RuntimeError("Unavailable")
                provider.get_response("Proofread", "Original text")
                app.output_ready_signal.emit.assert_not_called()
                app.show_message_signal.emit.assert_called_once()

    def test_cancelled_request_does_not_reach_the_clipboard(self):
        app = SimpleNamespace(output_ready_signal=Mock(), show_message_signal=Mock())
        provider = GeminiProvider(app)
        provider.model_name = "gemini-flash-latest"
        provider.client = Mock()

        def finish_after_cancel(*args, **kwargs):
            # The turn completes just after the user cancelled it.
            provider.cancel()
            return SimpleNamespace(text="Corrected text.\n")

        provider.client.models.generate_content.side_effect = finish_after_cancel
        provider.get_response("Proofread", "Original text")
        app.output_ready_signal.emit.assert_not_called()
        app.show_message_signal.emit.assert_not_called()

    def test_gemini_delivers_one_complete_result(self):
        app = SimpleNamespace(output_ready_signal=Mock(), show_message_signal=Mock())
        provider = GeminiProvider(app)
        provider.model_name = "gemini-flash-latest"
        provider.client = Mock()
        provider.client.models.generate_content.return_value = SimpleNamespace(text="Corrected text.\n")
        provider.get_response("Proofread", "Original text")
        app.output_ready_signal.emit.assert_called_once_with("Corrected text.")
        app.show_message_signal.emit.assert_not_called()


if __name__ == "__main__":
    unittest.main()
